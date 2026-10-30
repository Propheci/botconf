package telegram

import (
	"strings"
	"testing"

	"github.com/gotd/td/tg"
	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settings/settingstest"
	"github.com/propheci/botconf/go/settingspb"
)

// backServer asks a question with ‹ Back on "a:3:v" and shows the setting on
// any other press; answers are recorded.
func backServer() *settingstest.Server {
	return &settingstest.Server{ActFunc: func(request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
		if request.GetPress() == "a:3:v" {
			return &settingspb.ActResponse{Ask: &settingspb.Ask{
				Question: "q1",
				Text:     plain("Send the new log filter."),
				Title:    "Logging",
				Back:     &settingspb.Button{Label: "‹ Back", Token: "o:s:3"},
			}}, nil
		}
		return &settingspb.ActResponse{Screen: screen("Logging: info", "‹ Back", "o:h")}, nil
	}}
}

func TestBotQuestionCarriesBack(t *testing.T) {
	panel, api := newBotPanel(t, backServer())
	if _, err := panel.HandleCallback(contextFor(t), press("cfg:a:3:v")); err != nil {
		t.Fatalf("HandleCallback() error = %v", err)
	}
	sends := all[*tg.MessagesSendMessageRequest](api)
	if len(sends) != 1 || !strings.Contains(sends[0].Message, "Tap ‹ Back") {
		t.Fatalf("sent %#v, want the question with ‹ Back", sends)
	}
	if got := callbackData(sends[0].ReplyMarkup); len(got) != 1 || got[0] != "cfg:o:s:3" {
		t.Fatalf("question buttons = %v, want ‹ Back", got)
	}
}

func TestBotBackEndsTheQuestion(t *testing.T) {
	for name, pressed := range map[string]int{"on the question": 101, "on the panel": 9} {
		t.Run(name, func(t *testing.T) {
			server := backServer()
			panel, api := newBotPanel(t, server)
			ctx := contextFor(t)
			_, _ = panel.HandleCallback(ctx, press("cfg:a:3:v"))
			back := press("cfg:o:s:3")
			back.MsgID = pressed
			if _, err := panel.HandleCallback(ctx, back); err != nil {
				t.Fatalf("HandleCallback() error = %v", err)
			}
			// The panel that asked is redrawn, and the question removed.
			edits := all[*tg.MessagesEditMessageRequest](api)
			if len(edits) != 1 || edits[0].ID != 9 || edits[0].Message != "Logging: info" {
				t.Fatalf("edits = %#v, want the panel (9) redrawn", edits)
			}
			deletes := all[*tg.MessagesDeleteMessagesRequest](api)
			if len(deletes) != 1 || deletes[0].ID[0] != 101 {
				t.Fatalf("deletes = %#v, want the question (101) removed", deletes)
			}
			// The next message is not an answer any more.
			text := &tg.Message{ID: 31, PeerID: &tg.PeerUser{UserID: owner}, Message: "debug"}
			if handled, _ := panel.HandleMessage(ctx, &tg.InputPeerUser{UserID: owner}, text); handled {
				t.Fatal("a message after ‹ Back was taken as an answer")
			}
			for _, act := range server.Acts() {
				if act.GetAnswer() != nil {
					t.Fatalf("sent an answer %v after ‹ Back", act.GetAnswer())
				}
			}
		})
	}
}

func TestBotPressOnAnotherPanelKeepsTheQuestion(t *testing.T) {
	panel, _ := newBotPanel(t, backServer())
	ctx := contextFor(t)
	_, _ = panel.HandleCallback(ctx, press("cfg:a:3:v"))
	other := press("cfg:o:h")
	other.MsgID = 77
	_, _ = panel.HandleCallback(ctx, other)
	text := &tg.Message{ID: 31, PeerID: &tg.PeerUser{UserID: owner}, Message: "debug"}
	if handled, _ := panel.HandleMessage(ctx, &tg.InputPeerUser{UserID: owner}, text); !handled {
		t.Fatal("a press on another panel dropped the question")
	}
}

func TestPickerQuestionsOfferCancel(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		return &settingspb.ActResponse{Ask: &settingspb.Ask{
			Question: "q2",
			Text:     plain("Who?"),
			Picker:   &settingspb.Picker{Domain: UserDomain, Many: true},
			Back:     &settingspb.Button{Label: "‹ Back", Token: "o:s:2"},
		}}, nil
	}}
	api := &recorder{}
	panel := NewBotPanel(settingstest.Start(t, server), tg.NewClient(api), WithPickers())
	_, _ = panel.HandleCallback(contextFor(t), press("cfg:a:2:i"))
	keyboard, ok := all[*tg.MessagesSendMessageRequest](api)[0].ReplyMarkup.(*tg.ReplyKeyboardMarkup)
	if !ok || len(keyboard.Rows) != 2 {
		t.Fatalf("question markup = %#v, want the picker and Cancel", keyboard)
	}
	if button, ok := keyboard.Rows[1].Buttons[0].(*tg.KeyboardButton); !ok || button.Text != cancelButton || !isCancel(button.Text) {
		t.Fatalf("second row = %#v, want a Cancel button that cancels", keyboard.Rows[1])
	}
}

func TestInlineBackEndsTheQuestion(t *testing.T) {
	server := backServer()
	server.RenderFunc = homeServer().RenderFunc
	panel, _, bot := newInlinePanel(t, server)
	ctx := contextFor(t)
	if err := panel.Open(ctx, chat, settings.Home()); err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	if _, err := panel.HandleInlineCallback(ctx, inlinePress("cfg:a:3:v")); err != nil {
		t.Fatalf("HandleInlineCallback() error = %v", err)
	}
	asked := all[*tg.MessagesEditInlineBotMessageRequest](bot.dcs[4])
	if len(asked) != 1 || !strings.Contains(asked[0].Message, "Tap ‹ Back") {
		t.Fatalf("edits = %#v, want the question with ‹ Back", asked)
	}
	if got := callbackData(asked[0].ReplyMarkup); len(got) != 1 || got[0] != "cfg:o:s:3" {
		t.Fatalf("question buttons = %v, want ‹ Back", got)
	}

	if _, err := panel.HandleInlineCallback(ctx, inlinePress("cfg:o:s:3")); err != nil {
		t.Fatalf("HandleInlineCallback() error = %v", err)
	}
	edits := all[*tg.MessagesEditInlineBotMessageRequest](bot.dcs[4])
	if len(edits) != 2 || edits[1].Message != "Logging: info" {
		t.Fatalf("edits = %#v, want the setting shown again", edits)
	}
	if handled, _ := panel.HandleMessage(ctx, chat, &tg.Message{ID: 40, Out: true, Message: "debug"}); handled {
		t.Fatal("a message after ‹ Back was taken as an answer")
	}
}
