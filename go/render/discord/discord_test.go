package discord

import (
	"context"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/bwmarrin/discordgo"
	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settings/settingstest"
	"github.com/propheci/botconf/go/settingspb"
)

// fakeAPI records the Discord calls the panel makes.
type fakeAPI struct {
	mu        sync.Mutex
	sends     []*discordgo.MessageSend
	responses []*discordgo.InteractionResponse
	edits     []*discordgo.WebhookEdit
	followups []*discordgo.WebhookParams
}

func (f *fakeAPI) ChannelMessageSendComplex(_ string, data *discordgo.MessageSend, _ ...discordgo.RequestOption) (*discordgo.Message, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.sends = append(f.sends, data)
	return &discordgo.Message{ID: "m1"}, nil
}

func (f *fakeAPI) InteractionRespond(_ *discordgo.Interaction, resp *discordgo.InteractionResponse, _ ...discordgo.RequestOption) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.responses = append(f.responses, resp)
	return nil
}

func (f *fakeAPI) InteractionResponseEdit(_ *discordgo.Interaction, edit *discordgo.WebhookEdit, _ ...discordgo.RequestOption) (*discordgo.Message, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.edits = append(f.edits, edit)
	return &discordgo.Message{}, nil
}

func (f *fakeAPI) FollowupMessageCreate(_ *discordgo.Interaction, _ bool, data *discordgo.WebhookParams, _ ...discordgo.RequestOption) (*discordgo.Message, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.followups = append(f.followups, data)
	return &discordgo.Message{}, nil
}

func contextFor(t *testing.T) context.Context {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)
	return ctx
}

func newPanel(t *testing.T, server *settingstest.Server) (*Panel, *fakeAPI) {
	t.Helper()
	api := &fakeAPI{}
	return NewPanel(settingstest.Start(t, server), api), api
}

func plain(text string) *settingspb.Text {
	return &settingspb.Text{Spans: []*settingspb.Span{{Text: text}}}
}

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

func click(customID string) *discordgo.InteractionCreate {
	return &discordgo.InteractionCreate{Interaction: &discordgo.Interaction{
		Type:      discordgo.InteractionMessageComponent,
		ChannelID: "c1",
		Member:    &discordgo.Member{User: &discordgo.User{ID: "u1"}},
		Message:   &discordgo.Message{ID: "panel"},
		Data:      discordgo.MessageComponentInteractionData{CustomID: customID, ComponentType: discordgo.ButtonComponent},
	}}
}

func submit(value string) *discordgo.InteractionCreate {
	return &discordgo.InteractionCreate{Interaction: &discordgo.Interaction{
		Type:      discordgo.InteractionModalSubmit,
		ChannelID: "c1",
		User:      &discordgo.User{ID: "u1"},
		Data: discordgo.ModalSubmitInteractionData{CustomID: "cfg:!q", Components: []discordgo.MessageComponent{
			&discordgo.ActionsRow{Components: []discordgo.MessageComponent{&discordgo.TextInput{CustomID: valueID, Value: value}}},
		}},
	}}
}

// customIDs is the custom id of every button in components.
func customIDs(components []discordgo.MessageComponent) []string {
	var ids []string
	for _, component := range components {
		row, ok := component.(discordgo.ActionsRow)
		if !ok {
			continue
		}
		for _, inner := range row.Components {
			if button, ok := inner.(discordgo.Button); ok {
				ids = append(ids, button.CustomID)
			}
		}
	}
	return ids
}

func TestMarkdownEscapesPlainText(t *testing.T) {
	text := &settingspb.Text{Spans: []*settingspb.Span{
		{Text: "a*b_c "},
		{Style: settingspb.Style_STYLE_BOLD, Text: "Logging"},
		{Text: ": "},
		{Style: settingspb.Style_STYLE_CODE, Text: "info,x=*"},
		{Text: "\n"},
		{Style: settingspb.Style_STYLE_CODE_BLOCK, Text: `{"a": 1}`},
	}}
	want := "a\\*b\\_c **Logging**: `info,x=*`\n```\n{\"a\": 1}\n```"
	if got := Markdown(text); got != want {
		t.Fatalf("Markdown() = %q, want %q", got, want)
	}
}

func TestRowsBecomeActionRows(t *testing.T) {
	panel, _ := newPanel(t, &settingstest.Server{})
	long := strings.Repeat("x", 100)
	components := panel.components([]*settingspb.Row{
		{Buttons: []*settingspb.Button{{Label: long, Token: "o:1"}, {Label: "B", Token: "~abc"}}},
		{},
	})
	if len(components) != 1 {
		t.Fatalf("components = %#v, want one action row", components)
	}
	row := components[0].(discordgo.ActionsRow)
	first := row.Components[0].(discordgo.Button)
	if len([]rune(first.Label)) != 80 || first.CustomID != "cfg:o:1" {
		t.Fatalf("first button = %#v, want the label cut to 80 and the prefixed token", first)
	}
	if got := strings.Join(customIDs(components), " "); got != "cfg:o:1 cfg:~abc" {
		t.Fatalf("custom ids = %q", got)
	}
}

func TestOpenSendsTheHomeScreen(t *testing.T) {
	server := &settingstest.Server{RenderFunc: func(*settingspb.RenderRequest) (*settingspb.Screen, error) {
		return screen("Settings", "Logging", "o:1"), nil
	}}
	panel, api := newPanel(t, server)
	if err := panel.Open(contextFor(t), "c1", "u1", settings.Home()); err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	if len(api.sends) != 1 || api.sends[0].Content != "Settings" || customIDs(api.sends[0].Components)[0] != "cfg:o:1" {
		t.Fatalf("sends = %#v, want the home screen", api.sends)
	}
	render := server.Renders()[0]
	if render.GetActor().GetDomain() != UserDomain || render.GetActor().GetId() != "u1" || render.GetLimits().GetMaxRows() != 5 {
		t.Fatalf("render request = %v, want Discord's actor and limits", render)
	}
}

func TestPressUpdatesTheMessage(t *testing.T) {
	notice := "Saved"
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		return &settingspb.ActResponse{Notice: &notice, Screen: screen("Logging: debug", "Back", "o:")}, nil
	}}
	panel, api := newPanel(t, server)
	handled, err := panel.HandleInteraction(contextFor(t), click("cfg:p:3:1"))
	if !handled || err != nil {
		t.Fatalf("HandleInteraction() = %v, %v", handled, err)
	}
	if got := server.Acts()[0].GetPress(); got != "p:3:1" {
		t.Fatalf("pressed %q, want the token without the prefix", got)
	}
	if len(api.responses) != 1 || api.responses[0].Type != discordgo.InteractionResponseUpdateMessage || api.responses[0].Data.Content != "Logging: debug" {
		t.Fatalf("responses = %#v, want the message updated", api.responses)
	}
	if len(api.followups) != 1 || api.followups[0].Content != "Saved" || api.followups[0].Flags != discordgo.MessageFlagsEphemeral {
		t.Fatalf("followups = %#v, want the notice as an ephemeral message", api.followups)
	}
}

func TestPressDefersThenEdits(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		time.Sleep(100 * time.Millisecond)
		return &settingspb.ActResponse{Screen: screen("Logging: debug")}, nil
	}}
	panel, api := newPanel(t, server)
	panel.deferAfter = 10 * time.Millisecond
	if _, err := panel.HandleInteraction(contextFor(t), click("cfg:p:3:1")); err != nil {
		t.Fatalf("HandleInteraction() error = %v", err)
	}
	if len(api.responses) != 1 || api.responses[0].Type != discordgo.InteractionResponseDeferredMessageUpdate {
		t.Fatalf("responses = %#v, want the press acknowledged first", api.responses)
	}
	if len(api.edits) != 1 || *api.edits[0].Content != "Logging: debug" {
		t.Fatalf("edits = %#v, want the panel edited afterwards", api.edits)
	}
}

func TestRejectedPressIsEphemeral(t *testing.T) {
	rejected := "You may not change this."
	server := &settingstest.Server{ActFunc: func(*settingspb.ActRequest) (*settingspb.ActResponse, error) {
		return &settingspb.ActResponse{Rejected: &rejected}, nil
	}}
	panel, api := newPanel(t, server)
	_, _ = panel.HandleInteraction(contextFor(t), click("cfg:c:1"))
	if len(api.responses) != 1 || api.responses[0].Data.Content != rejected || api.responses[0].Data.Flags != discordgo.MessageFlagsEphemeral {
		t.Fatalf("responses = %#v, want the refusal shown to the presser only", api.responses)
	}
}

func askingServer() *settingstest.Server {
	return &settingstest.Server{ActFunc: func(request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
		if request.GetAnswer() != nil {
			return &settingspb.ActResponse{Screen: screen("Logging: debug")}, nil
		}
		current := "info"
		return &settingspb.ActResponse{Ask: &settingspb.Ask{Question: "q1", Text: plain("Send the new log filter."), Title: "Logging", Current: &current}}, nil
	}}
}

func TestAskOpensAModal(t *testing.T) {
	panel, api := newPanel(t, askingServer())
	if _, err := panel.HandleInteraction(contextFor(t), click("cfg:a:3:v")); err != nil {
		t.Fatalf("HandleInteraction() error = %v", err)
	}
	if len(api.responses) != 1 || api.responses[0].Type != discordgo.InteractionResponseModal {
		t.Fatalf("responses = %#v, want a modal", api.responses)
	}
	modal := api.responses[0].Data
	input := modal.Components[0].(discordgo.ActionsRow).Components[0].(discordgo.TextInput)
	if modal.CustomID != "cfg:!q" || modal.Title != "Logging" || input.Value != "info" {
		t.Fatalf("modal = %#v (input %#v), want the question with the current value", modal, input)
	}
}

func TestModalSubmitAnswers(t *testing.T) {
	server := askingServer()
	panel, api := newPanel(t, server)
	ctx := contextFor(t)
	_, _ = panel.HandleInteraction(ctx, click("cfg:a:3:v"))

	handled, err := panel.HandleInteraction(ctx, submit("debug"))
	if !handled || err != nil {
		t.Fatalf("HandleInteraction() = %v, %v", handled, err)
	}
	if got := server.Acts()[1].GetAnswer(); got.GetQuestion() != "q1" || got.GetText() != "debug" {
		t.Fatalf("answer = %v, want q1 answered with debug", got)
	}
	last := api.responses[len(api.responses)-1]
	if last.Type != discordgo.InteractionResponseUpdateMessage || last.Data.Content != "Logging: debug" {
		t.Fatalf("response = %#v, want the panel redrawn", last)
	}
	// The question is answered: a second submit finds nothing.
	_, _ = panel.HandleInteraction(ctx, submit("again"))
	if len(server.Acts()) != 2 {
		t.Fatalf("acts = %d, want the second submit refused", len(server.Acts()))
	}
}

func TestPickerUsesASelectMenu(t *testing.T) {
	server := &settingstest.Server{ActFunc: func(request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
		if request.GetAnswer() != nil {
			return &settingspb.ActResponse{Screen: screen("Admins: 2")}, nil
		}
		return &settingspb.ActResponse{Ask: &settingspb.Ask{Question: "q2", Text: plain("Who?"), Picker: &settingspb.Picker{Domain: UserDomain, Many: true}}}, nil
	}}
	panel, api := newPanel(t, server)
	ctx := contextFor(t)
	_, _ = panel.HandleInteraction(ctx, click("cfg:a:2:i"))
	menu, ok := api.responses[0].Data.Components[0].(discordgo.ActionsRow).Components[0].(discordgo.SelectMenu)
	if !ok || menu.MenuType != discordgo.UserSelectMenu || menu.CustomID != "cfg:!p" || menu.MaxValues != 25 {
		t.Fatalf("response = %#v, want a user select menu", api.responses[0])
	}

	selected := click("cfg:!p")
	selected.Data = discordgo.MessageComponentInteractionData{CustomID: "cfg:!p", ComponentType: discordgo.UserSelectMenuComponent, Values: []string{"7", "8"}}
	if _, err := panel.HandleInteraction(ctx, selected); err != nil {
		t.Fatalf("HandleInteraction() error = %v", err)
	}
	picked := server.Acts()[1].GetAnswer().GetPicked()
	if picked.GetDomain() != UserDomain || strings.Join(picked.GetIdsJson(), ",") != `"7","8"` {
		t.Fatalf("picked = %v, want users 7 and 8 as JSON strings", picked)
	}
}

func TestForeignCustomIDIsIgnored(t *testing.T) {
	server := &settingstest.Server{}
	panel, api := newPanel(t, server)
	handled, err := panel.HandleInteraction(contextFor(t), click("other:thing"))
	if handled || err != nil || len(api.responses) != 0 || len(server.Acts()) != 0 {
		t.Fatalf("HandleInteraction() = %v, %v with %d responses, want it left to the host", handled, err, len(api.responses))
	}
}
