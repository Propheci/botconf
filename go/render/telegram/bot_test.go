package telegram

import (
	"errors"
	"strings"
	"testing"

	"github.com/gotd/td/bin"
	"github.com/gotd/td/tg"
	"github.com/gotd/td/tgerr"
	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settings/settingstest"
	"github.com/propheci/botconf/go/settingspb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

const owner = 5

// newBotPanel is a bot panel over a scripted settings server and a recording
// Telegram API.
func newBotPanel(t *testing.T, server *settingstest.Server) (*BotPanel, *recorder) {
	t.Helper()
	api := &recorder{}
	return NewBotPanel(settingstest.Start(t, server), tg.NewClient(api)), api
}

func press(data string) *tg.UpdateBotCallbackQuery {
	return &tg.UpdateBotCallbackQuery{QueryID: 42, UserID: owner, Peer: &tg.PeerUser{UserID: owner}, MsgID: 9, Data: []byte(data)}
}

func TestOpenSendsTheHomeScreen(t *testing.T) {
	server := &settingstest.Server{RenderFunc: func(*settingspb.RenderRequest) (*settingspb.Screen, error) {
		return screen("Settings", "Logging", "o:1"), nil
	}}
	panel, api := newBotPanel(t, server)
	if err := panel.Open(contextFor(t), &tg.InputPeerUser{UserID: owner}, owner, settings.Home()); err != nil {
		t.Fatalf("Open() error = %v", err)
	}

	sends := all[*tg.MessagesSendMessageRequest](api)
	if len(sends) != 1 || sends[0].Message != "Settings" {
		t.Fatalf("sent %#v, want the home screen", sends)
	}
	if got := callbackData(sends[0].ReplyMarkup); len(got) != 1 || got[0] != "cfg:o:1" {
		t.Fatalf("callback data = %v, want cfg:o:1", got)
	}
	render := server.Renders()[0]
	if render.GetActor().GetDomain() != UserDomain || render.GetActor().GetId() != "5" {
		t.Fatalf("actor = %v, want telegram.user 5", render.GetActor())
	}
	if render.GetLimits().GetTokenBytes() != 60 || render.GetPage().GetHome() == nil {
		t.Fatalf("render request = %v, want the home page with Telegram's limits", render)
	}
}

func TestPressEditsThePanel(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		return &settingspb.ActResponse{Screen: screen("Logging: debug", "Back", "o:")}, nil
	}}
	panel, api := newBotPanel(t, server)
	handled, err := panel.HandleCallback(contextFor(t), press("cfg:p:3:1"))
	if !handled || err != nil {
		t.Fatalf("HandleCallback() = %v, %v", handled, err)
	}
	if got := server.Acts()[0].GetPress(); got != "p:3:1" {
		t.Fatalf("pressed %q, want the token without the prefix", got)
	}
	edits := all[*tg.MessagesEditMessageRequest](api)
	if len(edits) != 1 || edits[0].ID != 9 || edits[0].Message != "Logging: debug" {
		t.Fatalf("edits = %#v, want the panel message redrawn", edits)
	}
	if peer, ok := edits[0].Peer.(*tg.InputPeerUser); !ok || peer.UserID != owner {
		t.Fatalf("edit peer = %#v", edits[0].Peer)
	}
}

func TestUnchangedPanelIsNotAnError(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		return &settingspb.ActResponse{Screen: screen("Same")}, nil
	}}
	panel, api := newBotPanel(t, server)
	api.fail = func(request bin.Encoder) error {
		if _, ok := request.(*tg.MessagesEditMessageRequest); ok {
			return tgerr.New(400, "MESSAGE_NOT_MODIFIED")
		}
		return nil
	}
	if _, err := panel.HandleCallback(contextFor(t), press("cfg:o:")); err != nil {
		t.Fatalf("HandleCallback() error = %v, want MESSAGE_NOT_MODIFIED ignored", err)
	}
}

func TestRejectedPressShowsAnAlert(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		rejected := "You may not change this."
		return &settingspb.ActResponse{Rejected: &rejected}, nil
	}}
	panel, api := newBotPanel(t, server)
	if _, err := panel.HandleCallback(contextFor(t), press("cfg:c:1")); err != nil {
		t.Fatalf("HandleCallback() error = %v", err)
	}
	answers := all[*tg.MessagesSetBotCallbackAnswerRequest](api)
	if len(answers) != 1 || !answers[0].Alert || answers[0].Message != "You may not change this." {
		t.Fatalf("answers = %#v, want one alert", answers)
	}
	if edits := all[*tg.MessagesEditMessageRequest](api); len(edits) != 0 {
		t.Fatalf("a rejected press redrew the panel: %#v", edits)
	}
}

func TestStalePressShowsANoticeAndRedraws(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		notice := "This panel is out of date."
		return &settingspb.ActResponse{Stale: true, Notice: &notice, Screen: screen("Settings")}, nil
	}}
	panel, api := newBotPanel(t, server)
	if _, err := panel.HandleCallback(contextFor(t), press("cfg:~AbCdEfGhIj")); err != nil {
		t.Fatalf("HandleCallback() error = %v", err)
	}
	answers := all[*tg.MessagesSetBotCallbackAnswerRequest](api)
	if len(answers) != 1 || answers[0].Alert || answers[0].Message != "This panel is out of date." {
		t.Fatalf("answers = %#v, want the notice as a toast", answers)
	}
	if edits := all[*tg.MessagesEditMessageRequest](api); len(edits) != 1 || edits[0].Message != "Settings" {
		t.Fatalf("edits = %#v, want the panel redrawn", edits)
	}
	if got := server.Acts()[0].GetPress(); got != "~AbCdEfGhIj" {
		t.Fatalf("pressed %q, want the handle sent back as received", got)
	}
}

func TestAskThenAnswerSendsTheAnswer(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
		if request.GetAnswer() != nil {
			return &settingspb.ActResponse{Screen: screen("Logging: debug")}, nil
		}
		return &settingspb.ActResponse{Ask: &settingspb.Ask{Question: "q1", Text: plain("Send the new log filter."), Title: "Logging"}}, nil
	}}
	panel, api := newBotPanel(t, server)
	ctx := contextFor(t)
	if _, err := panel.HandleCallback(ctx, press("cfg:a:3:v")); err != nil {
		t.Fatalf("HandleCallback() error = %v", err)
	}
	sends := all[*tg.MessagesSendMessageRequest](api)
	if len(sends) != 1 || !strings.HasPrefix(sends[0].Message, "Send the new log filter.") || !strings.Contains(sends[0].Message, "/cancel") {
		t.Fatalf("sent %#v, want the question with how to cancel", sends)
	}
	if reply, ok := sends[0].ReplyMarkup.(*tg.ReplyKeyboardForceReply); !ok || reply.Placeholder != "Logging" {
		t.Fatalf("question markup = %#v, want a force-reply", sends[0].ReplyMarkup)
	}
	questionMsg := 101

	// Someone else's message in the chat is not the answer.
	other := &tg.Message{ID: 30, FromID: &tg.PeerUser{UserID: 6}, PeerID: &tg.PeerChat{ChatID: 1}, Message: "nope"}
	if handled, _ := panel.HandleMessage(ctx, &tg.InputPeerUser{UserID: owner}, other); handled {
		t.Fatal("took another user's message as the answer")
	}

	answer := &tg.Message{ID: 31, PeerID: &tg.PeerUser{UserID: owner}, Message: "debug"}
	handled, err := panel.HandleMessage(ctx, &tg.InputPeerUser{UserID: owner}, answer)
	if !handled || err != nil {
		t.Fatalf("HandleMessage() = %v, %v", handled, err)
	}
	acts := server.Acts()
	if got := acts[1].GetAnswer(); got.GetQuestion() != "q1" || got.GetText() != "debug" {
		t.Fatalf("answer = %v, want q1 answered with debug", got)
	}
	if edits := all[*tg.MessagesEditMessageRequest](api); len(edits) != 1 || edits[0].ID != 9 {
		t.Fatalf("edits = %#v, want the panel message redrawn", edits)
	}
	deletes := all[*tg.MessagesDeleteMessagesRequest](api)
	if len(deletes) != 1 || len(deletes[0].ID) != 2 || deletes[0].ID[0] != questionMsg || deletes[0].ID[1] != 31 {
		t.Fatalf("deletes = %#v, want the question and the answer removed", deletes)
	}

	// The question is answered: the next message is ordinary.
	if handled, _ := panel.HandleMessage(ctx, &tg.InputPeerUser{UserID: owner}, answer); handled {
		t.Fatal("a second message was taken as an answer")
	}
}

func TestCancelGivesUp(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
		if request.GetAnswer() != nil {
			return &settingspb.ActResponse{Screen: screen("Settings")}, nil
		}
		return &settingspb.ActResponse{Ask: &settingspb.Ask{Question: "q1", Text: plain("Value?")}}, nil
	}}
	panel, _ := newBotPanel(t, server)
	ctx := contextFor(t)
	_, _ = panel.HandleCallback(ctx, press("cfg:a:3:v"))
	cancel := &tg.Message{ID: 31, PeerID: &tg.PeerUser{UserID: owner}, Message: "/cancel"}
	if _, err := panel.HandleMessage(ctx, &tg.InputPeerUser{UserID: owner}, cancel); err != nil {
		t.Fatalf("HandleMessage() error = %v", err)
	}
	if server.Acts()[1].GetAnswer().GetCancel() == nil {
		t.Fatalf("answer = %v, want a cancel", server.Acts()[1].GetAnswer())
	}
}

func TestPickedPeersAnswerTheQuestion(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
		if request.GetAnswer() != nil {
			return &settingspb.ActResponse{Screen: screen("Sudo users: 1")}, nil
		}
		return &settingspb.ActResponse{Ask: &settingspb.Ask{Question: "q2", Text: plain("Who?"), Picker: &settingspb.Picker{Domain: UserDomain, Many: true}}}, nil
	}}
	api := &recorder{}
	panel := NewBotPanel(settingstest.Start(t, server), tg.NewClient(api), WithPickers())
	ctx := contextFor(t)
	_, _ = panel.HandleCallback(ctx, press("cfg:a:2:i"))
	if !server.Acts()[0].GetPickers() {
		t.Fatal("pickers not offered in a private chat")
	}
	sends := all[*tg.MessagesSendMessageRequest](api)
	keyboard, ok := sends[0].ReplyMarkup.(*tg.ReplyKeyboardMarkup)
	if !ok {
		t.Fatalf("question markup = %#v, want the picker keyboard", sends[0].ReplyMarkup)
	}
	if button, ok := keyboard.Rows[0].Buttons[0].(*tg.KeyboardButtonRequestPeer); !ok || button.MaxQuantity != 10 {
		t.Fatalf("picker = %#v, want a request-peer button for several users", keyboard.Rows[0].Buttons[0])
	}

	service := &tg.MessageService{ID: 32, PeerID: &tg.PeerUser{UserID: owner}, Action: &tg.MessageActionRequestedPeer{
		ButtonID: pickerButtonID,
		Peers:    []tg.PeerClass{&tg.PeerUser{UserID: 7}, &tg.PeerUser{UserID: 8}},
	}}
	handled, err := panel.HandleService(ctx, &tg.InputPeerUser{UserID: owner}, service)
	if !handled || err != nil {
		t.Fatalf("HandleService() = %v, %v", handled, err)
	}
	picked := server.Acts()[1].GetAnswer().GetPicked()
	if picked.GetDomain() != UserDomain || strings.Join(picked.GetIdsJson(), ",") != "7,8" {
		t.Fatalf("picked = %v, want users 7 and 8", picked)
	}
}

func TestForeignCallbackDataIsIgnored(t *testing.T) {
	server := &settingstest.Server{}
	panel, api := newBotPanel(t, server)
	handled, err := panel.HandleCallback(contextFor(t), press("wuphf:label"))
	if handled || err != nil {
		t.Fatalf("HandleCallback() = %v, %v, want it left to the host", handled, err)
	}
	if api.count() != 0 || len(server.Acts()) != 0 {
		t.Fatalf("a foreign press made %d calls and %d acts", api.count(), len(server.Acts()))
	}
}

func TestEveryPressIsAnswered(t *testing.T) {
	notice := "Saved"
	rejected := "No"
	for name, act := range map[string]func(*settingspb.ActRequest) (*settingspb.ActResponse, error){
		"redraw": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
			return &settingspb.ActResponse{Screen: screen("x")}, nil
		},
		"notice": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
			return &settingspb.ActResponse{Notice: &notice, Screen: screen("x")}, nil
		},
		"rejected": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
			return &settingspb.ActResponse{Rejected: &rejected}, nil
		},
		"nothing": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) { return &settingspb.ActResponse{}, nil },
		"ask": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
			return &settingspb.ActResponse{Ask: &settingspb.Ask{Question: "q", Text: plain("?")}}, nil
		},
		"error": func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
			return nil, status.Error(codes.PermissionDenied, "owners only")
		},
	} {
		t.Run(name, func(t *testing.T) {
			panel, api := newBotPanel(t, &settingstest.Server{ActFunc: act})
			_, err := panel.HandleCallback(contextFor(t), press("cfg:o:"))
			if name == "error" && !errors.Is(err, settings.ErrPermissionDenied) {
				t.Fatalf("error = %v, want the server's refusal", err)
			}
			if answers := all[*tg.MessagesSetBotCallbackAnswerRequest](api); len(answers) != 1 {
				t.Fatalf("answered %d times, want exactly once", len(answers))
			}
		})
	}
}

func TestCommandRepliesWithText(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		return &settingspb.ActResponse{Reply: plain("logging.filter = info")}, nil
	}}
	panel, api := newBotPanel(t, server)
	if err := panel.Command(contextFor(t), &tg.InputPeerUser{UserID: owner}, owner, 40, " get logging.filter "); err != nil {
		t.Fatalf("Command() error = %v", err)
	}
	if got := server.Acts()[0].GetCommand(); got != "get logging.filter" {
		t.Fatalf("command = %q", got)
	}
	sends := all[*tg.MessagesSendMessageRequest](api)
	reply, ok := sends[0].ReplyTo.(*tg.InputReplyToMessage)
	if len(sends) != 1 || sends[0].Message != "logging.filter = info" || !ok || reply.ReplyToMsgID != 40 {
		t.Fatalf("sent %#v, want the reply to the command", sends)
	}
}

func TestCommandWithoutArgumentsPostsThePanel(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		return &settingspb.ActResponse{Post: screen("Settings", "Logging", "o:1")}, nil
	}}
	panel, api := newBotPanel(t, server)
	if err := panel.Command(contextFor(t), &tg.InputPeerUser{UserID: owner}, owner, 40, ""); err != nil {
		t.Fatalf("Command() error = %v", err)
	}
	sends := all[*tg.MessagesSendMessageRequest](api)
	if len(sends) != 1 || sends[0].Message != "Settings" || len(callbackData(sends[0].ReplyMarkup)) != 1 {
		t.Fatalf("sent %#v, want the panel posted", sends)
	}
}
