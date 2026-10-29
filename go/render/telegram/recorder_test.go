package telegram

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/gotd/td/bin"
	"github.com/gotd/td/tg"
	"github.com/propheci/botconf/go/settingspb"
)

// recorder is a Telegram API that answers every call with a plausible
// success and records the requests, so a test can see what went on the wire.
type recorder struct {
	mu       sync.Mutex
	requests []bin.Encoder
	sent     int
	// fail, when set, can fail a request instead of answering it.
	fail func(bin.Encoder) error
}

func (r *recorder) Invoke(_ context.Context, input bin.Encoder, output bin.Decoder) error {
	r.mu.Lock()
	r.requests = append(r.requests, input)
	fail := r.fail
	r.mu.Unlock()
	if fail != nil {
		if err := fail(input); err != nil {
			return err
		}
	}
	var response bin.Encoder
	switch input.(type) {
	case *tg.MessagesSendMessageRequest, *tg.MessagesSendInlineBotResultRequest:
		r.mu.Lock()
		r.sent++
		response = &tg.UpdateShortSentMessage{ID: 100 + r.sent}
		r.mu.Unlock()
	case *tg.MessagesEditMessageRequest:
		response = &tg.Updates{}
	case *tg.MessagesDeleteMessagesRequest, *tg.ChannelsDeleteMessagesRequest:
		response = &tg.MessagesAffectedMessages{}
	case *tg.MessagesGetInlineBotResultsRequest:
		response = &tg.MessagesBotResults{
			QueryID: 77,
			Results: []tg.BotInlineResultClass{&tg.BotInlineResult{
				ID:          "panel",
				Type:        "article",
				SendMessage: &tg.BotInlineMessageText{Message: "panel"},
			}},
		}
	default:
		response = &tg.BoolTrue{}
	}
	var buffer bin.Buffer
	if err := buffer.Encode(response); err != nil {
		return err
	}
	return output.Decode(&buffer)
}

// all returns the requests of type T, in order.
func all[T bin.Encoder](r *recorder) []T {
	r.mu.Lock()
	defer r.mu.Unlock()
	var found []T
	for _, request := range r.requests {
		if typed, ok := request.(T); ok {
			found = append(found, typed)
		}
	}
	return found
}

func (r *recorder) count() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.requests)
}

func contextFor(t *testing.T) context.Context {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)
	return ctx
}

// screen is a panel screen with one plain line and one row of buttons, each
// given as label then token.
func screen(text string, buttons ...string) *settingspb.Screen {
	row := &settingspb.Row{}
	for i := 0; i+1 < len(buttons); i += 2 {
		row.Buttons = append(row.Buttons, &settingspb.Button{Label: buttons[i], Token: buttons[i+1]})
	}
	result := &settingspb.Screen{Text: plain(text)}
	if len(row.Buttons) > 0 {
		result.Rows = []*settingspb.Row{row}
	}
	return result
}

// callbackData is the data of each button of an inline keyboard.
func callbackData(markup tg.ReplyMarkupClass) []string {
	keyboard, ok := markup.(*tg.ReplyInlineMarkup)
	if !ok {
		return nil
	}
	var data []string
	for _, row := range keyboard.Rows {
		for _, button := range row.Buttons {
			if callback, ok := button.(*tg.KeyboardButtonCallback); ok {
				data = append(data, string(callback.Data))
			}
		}
	}
	return data
}
