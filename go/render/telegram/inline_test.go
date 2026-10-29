package telegram

import (
	"context"
	"errors"
	"strings"
	"sync"
	"testing"

	"github.com/gotd/td/bin"
	gotd "github.com/gotd/td/telegram"
	"github.com/gotd/td/tg"
	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settings/settingstest"
	"github.com/propheci/botconf/go/settingspb"
)

// fakeBot is a helper bot whose API and data centers are recorders.
type fakeBot struct {
	api *recorder

	mu       sync.Mutex
	dcs      map[int]*recorder
	dialled  []int
	failDial error
}

func (b *fakeBot) API() *tg.Client { return tg.NewClient(b.api) }

func (b *fakeBot) DC(_ context.Context, dc int, _ int64) (gotd.CloseInvoker, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.failDial != nil {
		return nil, b.failDial
	}
	b.dialled = append(b.dialled, dc)
	if b.dcs == nil {
		b.dcs = map[int]*recorder{}
	}
	if b.dcs[dc] == nil {
		b.dcs[dc] = &recorder{}
	}
	return closer{b.dcs[dc]}, nil
}

type closer struct{ *recorder }

func (closer) Close() error { return nil }

var chat = &tg.InputPeerChat{ChatID: 10}

// newInlinePanel is an inline panel whose account and helper bot are
// recorders. When the account asks the bot for inline results, the bot
// receives the account's own inline query at once.
func newInlinePanel(t *testing.T, server *settingstest.Server) (*InlinePanel, *recorder, *fakeBot) {
	t.Helper()
	user, bot := &recorder{}, &fakeBot{api: &recorder{}}
	panel := NewInlinePanel(settingstest.Start(t, server), tg.NewClient(user), bot, &tg.InputUser{UserID: 99}, owner)
	user.fail = func(request bin.Encoder) error {
		if query, ok := request.(*tg.MessagesGetInlineBotResultsRequest); ok {
			_, err := panel.HandleInlineQuery(context.Background(), &tg.UpdateBotInlineQuery{QueryID: 77, UserID: owner, Query: query.Query})
			return err
		}
		return nil
	}
	return panel, user, bot
}

func homeServer() *settingstest.Server {
	return &settingstest.Server{RenderFunc: func(*settingspb.RenderRequest) (*settingspb.Screen, error) {
		return screen("Settings", "Logging", "o:1", "Modules", "o:2"), nil
	}}
}

func TestOpenGoesThroughTheHelperBot(t *testing.T) {
	panel, user, bot := newInlinePanel(t, homeServer())
	if err := panel.Open(contextFor(t), chat, settings.Home()); err != nil {
		t.Fatalf("Open() error = %v", err)
	}

	queries := all[*tg.MessagesGetInlineBotResultsRequest](user)
	if len(queries) != 1 || len(queries[0].Query) != 22 {
		t.Fatalf("queries = %#v, want one 22-character token", queries)
	}
	if bot, ok := queries[0].Bot.(*tg.InputUser); !ok || bot.UserID != 99 {
		t.Fatalf("query bot = %#v, want the helper bot", queries[0].Bot)
	}
	results := all[*tg.MessagesSetInlineBotResultsRequest](bot.api)
	if len(results) != 1 || !results[0].Private || results[0].CacheTime != 0 || len(results[0].Results) != 1 {
		t.Fatalf("inline answers = %#v, want one private uncached result", results)
	}
	article := results[0].Results[0].(*tg.InputBotInlineResult)
	sent := article.SendMessage.(*tg.InputBotInlineMessageText)
	if sent.Message != "Settings" || strings.Join(callbackData(sent.ReplyMarkup), " ") != "cfg:o:1 cfg:o:2" {
		t.Fatalf("inline result = %#v, want the screen with its buttons", sent)
	}
	sends := all[*tg.MessagesSendInlineBotResultRequest](user)
	if len(sends) != 1 || sends[0].QueryID != 77 || sends[0].ID != "panel" {
		t.Fatalf("sends = %#v, want the bot's result sent", sends)
	}
	if texts := all[*tg.MessagesSendMessageRequest](user); len(texts) != 0 {
		t.Fatalf("the account also sent text: %#v", texts)
	}
}

func TestInlineQueriesFromOthersGetNothing(t *testing.T) {
	panel, user, bot := newInlinePanel(t, homeServer())
	user.fail = func(request bin.Encoder) error {
		if query, ok := request.(*tg.MessagesGetInlineBotResultsRequest); ok {
			ctx := context.Background()
			if handled, err := panel.HandleInlineQuery(ctx, &tg.UpdateBotInlineQuery{QueryID: 1, UserID: 6, Query: query.Query}); !handled || err != nil {
				t.Errorf("someone else's query: %v, %v", handled, err)
			}
			_, err := panel.HandleInlineQuery(ctx, &tg.UpdateBotInlineQuery{QueryID: 2, UserID: owner, Query: query.Query})
			return err
		}
		return nil
	}
	if err := panel.Open(contextFor(t), chat, settings.Home()); err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	results := all[*tg.MessagesSetInlineBotResultsRequest](bot.api)
	if len(results) != 2 || len(results[0].Results) != 0 || len(results[1].Results) != 1 {
		t.Fatalf("inline answers = %#v, want nothing for the stranger and the panel for the account", results)
	}

	// A query that is no panel token is left to the host.
	if handled, _ := panel.HandleInlineQuery(contextFor(t), &tg.UpdateBotInlineQuery{UserID: owner, Query: "sticker"}); handled {
		t.Fatal("took a query that is not a panel token")
	}
}

func TestTokensAreSingleUse(t *testing.T) {
	panel, user, bot := newInlinePanel(t, homeServer())
	var second bool
	user.fail = func(request bin.Encoder) error {
		if query, ok := request.(*tg.MessagesGetInlineBotResultsRequest); ok {
			ctx := context.Background()
			if _, err := panel.HandleInlineQuery(ctx, &tg.UpdateBotInlineQuery{QueryID: 1, UserID: owner, Query: query.Query}); err != nil {
				return err
			}
			second, _ = panel.HandleInlineQuery(ctx, &tg.UpdateBotInlineQuery{QueryID: 2, UserID: owner, Query: query.Query})
		}
		return nil
	}
	if err := panel.Open(contextFor(t), chat, settings.Home()); err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	if second {
		t.Fatal("a token answered a second query")
	}
	if results := all[*tg.MessagesSetInlineBotResultsRequest](bot.api); len(results) != 1 {
		t.Fatalf("inline answers = %d, want one", len(results))
	}
}

func inlinePress(data string) *tg.UpdateInlineBotCallbackQuery {
	return &tg.UpdateInlineBotCallbackQuery{
		QueryID: 42,
		UserID:  owner,
		MsgID:   &tg.InputBotInlineMessageID{DCID: 4, ID: 123, AccessHash: 456},
		Data:    []byte(data),
	}
}

func TestInlinePressEditsOnTheMessagesDC(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		notice := "Saved"
		return &settingspb.ActResponse{Notice: &notice, Screen: screen("Logging: debug", "Back", "o:")}, nil
	}}
	panel, _, bot := newInlinePanel(t, server)
	handled, err := panel.HandleInlineCallback(contextFor(t), inlinePress("cfg:p:3:1"))
	if !handled || err != nil {
		t.Fatalf("HandleInlineCallback() = %v, %v", handled, err)
	}
	if got := server.Acts()[0]; got.GetPress() != "p:3:1" || got.GetActor().GetId() != "5" {
		t.Fatalf("act = %v, want the press for the presser", got)
	}
	if len(bot.dialled) != 1 || bot.dialled[0] != 4 {
		t.Fatalf("dialled DCs %v, want the message's DC 4", bot.dialled)
	}
	edits := all[*tg.MessagesEditInlineBotMessageRequest](bot.dcs[4])
	if len(edits) != 1 || edits[0].Message != "Logging: debug" || len(callbackData(edits[0].ReplyMarkup)) != 1 {
		t.Fatalf("edits = %#v, want the panel redrawn", edits)
	}
	if id, ok := edits[0].ID.(*tg.InputBotInlineMessageID); !ok || id.ID != 123 {
		t.Fatalf("edited %#v, want the pressed message", edits[0].ID)
	}
	if main := all[*tg.MessagesEditInlineBotMessageRequest](bot.api); len(main) != 0 {
		t.Fatalf("edited through the main connection: %#v", main)
	}
	answers := all[*tg.MessagesSetBotCallbackAnswerRequest](bot.api)
	if len(answers) != 1 || answers[0].Message != "Saved" || answers[0].Alert {
		t.Fatalf("answers = %#v, want the notice as one toast", answers)
	}
}

func TestEveryInlinePressIsAnswered(t *testing.T) {
	rejected := "No"
	for name, act := range map[string]func(*settingspb.ActRequest) (*settingspb.ActResponse, error){
		"redraw": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
			return &settingspb.ActResponse{Screen: screen("x")}, nil
		},
		"rejected": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
			return &settingspb.ActResponse{Rejected: &rejected}, nil
		},
		"error": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) { return nil, errors.New("boom") },
	} {
		t.Run(name, func(t *testing.T) {
			panel, _, bot := newInlinePanel(t, &settingstest.Server{ActFunc: act})
			_, _ = panel.HandleInlineCallback(contextFor(t), inlinePress("cfg:o:"))
			if answers := all[*tg.MessagesSetBotCallbackAnswerRequest](bot.api); len(answers) != 1 {
				t.Fatalf("answered %d times, want exactly once", len(answers))
			}
		})
	}
	panel, _, bot := newInlinePanel(t, &settingstest.Server{})
	if handled, _ := panel.HandleInlineCallback(contextFor(t), inlinePress("wuphf:label")); handled || bot.api.count() != 0 {
		t.Fatal("took a press that is not the panel's")
	}
}

func TestOutgoingAnswerIsTaken(t *testing.T) {
	server := homeServer()
	server.ActFunc = func(request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
		if request.GetAnswer() != nil {
			return &settingspb.ActResponse{Screen: screen("Logging: debug")}, nil
		}
		return &settingspb.ActResponse{Ask: &settingspb.Ask{Question: "q1", Text: plain("Send the new log filter.")}}, nil
	}
	panel, user, bot := newInlinePanel(t, server)
	ctx := contextFor(t)
	if err := panel.Open(ctx, chat, settings.Home()); err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	if _, err := panel.HandleInlineCallback(ctx, inlinePress("cfg:a:3:v")); err != nil {
		t.Fatalf("HandleInlineCallback() error = %v", err)
	}
	asked := all[*tg.MessagesEditInlineBotMessageRequest](bot.dcs[4])
	if len(asked) != 1 || !strings.HasPrefix(asked[0].Message, "Send the new log filter.") {
		t.Fatalf("edits = %#v, want the question on the panel", asked)
	}

	// Not the account's message, or not in the panel's chat: not the answer.
	incoming := &tg.Message{ID: 30, Message: "debug"}
	if handled, _ := panel.HandleMessage(ctx, chat, incoming); handled {
		t.Fatal("took an incoming message as the answer")
	}
	elsewhere := &tg.Message{ID: 31, Out: true, Message: "hello"}
	if handled, _ := panel.HandleMessage(ctx, &tg.InputPeerUser{UserID: 7}, elsewhere); handled {
		t.Fatal("took a message in another chat as the answer")
	}

	answer := &tg.Message{ID: 32, Out: true, Message: "debug"}
	handled, err := panel.HandleMessage(ctx, chat, answer)
	if !handled || err != nil {
		t.Fatalf("HandleMessage() = %v, %v", handled, err)
	}
	if got := server.Acts()[1].GetAnswer(); got.GetQuestion() != "q1" || got.GetText() != "debug" {
		t.Fatalf("answer = %v, want q1 answered with debug", got)
	}
	edits := all[*tg.MessagesEditInlineBotMessageRequest](bot.dcs[4])
	if len(edits) != 2 || edits[1].Message != "Logging: debug" {
		t.Fatalf("edits = %#v, want the panel redrawn after the answer", edits)
	}
	deletes := all[*tg.MessagesDeleteMessagesRequest](user)
	if len(deletes) != 1 || deletes[0].ID[0] != 32 {
		t.Fatalf("deletes = %#v, want the answer removed", deletes)
	}
	if handled, _ := panel.HandleMessage(ctx, chat, &tg.Message{ID: 33, Out: true, Message: "x"}); handled {
		t.Fatal("a second message was taken as an answer")
	}
}

func TestFallbackNumbersOptions(t *testing.T) {
	server := homeServer()
	server.ActFunc = func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		return &settingspb.ActResponse{Screen: screen("Modules", "Back", "o:")}, nil
	}
	user := &recorder{}
	panel := NewInlinePanel(settingstest.Start(t, server), tg.NewClient(user), nil, nil, owner)
	ctx := contextFor(t)
	if err := panel.Open(ctx, chat, settings.Home()); err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	sends := all[*tg.MessagesSendMessageRequest](user)
	if len(sends) != 1 || !strings.Contains(sends[0].Message, "1. Logging") || !strings.Contains(sends[0].Message, "2. Modules") {
		t.Fatalf("sent %#v, want numbered options", sends)
	}

	if handled, _ := panel.HandleMessage(ctx, chat, &tg.Message{ID: 40, Out: true, Message: "3"}); handled {
		t.Fatal("took a number with no option")
	}
	handled, err := panel.HandleMessage(ctx, chat, &tg.Message{ID: 41, Out: true, Message: "2"})
	if !handled || err != nil {
		t.Fatalf("HandleMessage() = %v, %v", handled, err)
	}
	if got := server.Acts()[0].GetPress(); got != "o:2" {
		t.Fatalf("pressed %q, want the second option's token", got)
	}
	sends = all[*tg.MessagesSendMessageRequest](user)
	if len(sends) != 2 || !strings.HasPrefix(sends[1].Message, "Modules") || !strings.Contains(sends[1].Message, "1. Back") {
		t.Fatalf("sent %#v, want the next text panel", sends)
	}
}

func TestFailingHelperBotFallsBackToText(t *testing.T) {
	panel, user, _ := newInlinePanel(t, homeServer())
	user.fail = func(request bin.Encoder) error {
		if _, ok := request.(*tg.MessagesGetInlineBotResultsRequest); ok {
			return errors.New("BOT_INLINE_DISABLED")
		}
		return nil
	}
	if err := panel.Open(contextFor(t), chat, settings.Home()); err != nil {
		t.Fatalf("Open() error = %v, want the text fallback to succeed", err)
	}
	if sends := all[*tg.MessagesSendMessageRequest](user); len(sends) != 1 || !strings.Contains(sends[0].Message, "1. Logging") {
		t.Fatalf("sent %#v, want the text panel", sends)
	}
	if inline := all[*tg.MessagesSendInlineBotResultRequest](user); len(inline) != 0 {
		t.Fatalf("sent an inline result anyway: %#v", inline)
	}
}
