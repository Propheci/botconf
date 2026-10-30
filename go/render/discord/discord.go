// Package discord draws botconf settings panels on Discord with discordgo:
// the panel is a message with buttons, presses and select menus arrive as
// component interactions, and questions are answered in modals.
//
// The renderer only maps screens to messages and interactions back to the
// settings client. It decides nothing about settings and never parses
// tokens: a token goes back exactly as the server sent it.
package discord

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/bwmarrin/discordgo"
	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settingspb"
)

const (
	// DefaultPrefix is the custom id prefix of panel components unless
	// another is set.
	DefaultPrefix = "cfg:"
	// UserDomain is the reference domain of Discord users, as actors and in
	// the user select menu.
	UserDomain = "discord.user"
	// ChannelDomain is the reference domain of Discord channels, in the
	// channel select menu.
	ChannelDomain = "discord.channel"

	// MaxCustomIDBytes is the longest custom id Discord accepts.
	MaxCustomIDBytes = 100
	// MaxMessageChars is the longest Discord message.
	MaxMessageChars = 2000
	// maxRows and maxPerRow are Discord's limits on a message's components.
	maxRows   = 5
	maxPerRow = 5
	// maxLabelChars is the longest button label.
	maxLabelChars = 80
	// maxTitleChars is the longest modal title and text input label.
	maxTitleChars = 45

	// answerID and pickID follow the prefix in the custom ids of the
	// renderer's own components: the button that opens a question's modal (and
	// the modal itself), and a picker's select menu. Panel tokens never start
	// with "!".
	answerID = "!q"
	pickID   = "!p"
	// valueID is the custom id of the modal's text input.
	valueID = "value"

	// deferAfter is how long a press may take before the interaction is
	// acknowledged and the panel edited later: Discord wants an answer within
	// 3 seconds.
	deferAfter = 2 * time.Second
)

// API is the part of a discordgo session the panel uses; *discordgo.Session
// implements it.
type API interface {
	ChannelMessageSendComplex(channelID string, data *discordgo.MessageSend, options ...discordgo.RequestOption) (*discordgo.Message, error)
	InteractionRespond(interaction *discordgo.Interaction, resp *discordgo.InteractionResponse, options ...discordgo.RequestOption) error
	InteractionResponseEdit(interaction *discordgo.Interaction, newresp *discordgo.WebhookEdit, options ...discordgo.RequestOption) (*discordgo.Message, error)
	FollowupMessageCreate(interaction *discordgo.Interaction, wait bool, data *discordgo.WebhookParams, options ...discordgo.RequestOption) (*discordgo.Message, error)
}

// Panel shows a settings panel on Discord.
type Panel struct {
	client     *settings.Client
	api        API
	prefix     string
	questions  *settings.Questions
	deferAfter time.Duration
}

// Option configures a Panel.
type Option func(*Panel)

// WithPrefix sets the custom id prefix of the panel's components (default
// DefaultPrefix). Interactions with other custom ids are left to the host.
func WithPrefix(prefix string) Option {
	return func(p *Panel) { p.prefix = prefix }
}

// NewPanel shows the panel served through client with a Discord session.
func NewPanel(client *settings.Client, api API, opts ...Option) *Panel {
	p := &Panel{
		client:     client,
		api:        api,
		prefix:     DefaultPrefix,
		questions:  settings.NewQuestions(10 * time.Minute),
		deferAfter: deferAfter,
	}
	for _, opt := range opts {
		opt(p)
	}
	return p
}

// Limits are what a Discord panel with custom id prefix can hold.
func Limits(prefix string) *settingspb.Limits {
	tokenBytes := MaxCustomIDBytes - len(prefix)
	if tokenBytes < 0 {
		tokenBytes = 0
	}
	return &settingspb.Limits{TokenBytes: uint32(tokenBytes), TextChars: MaxMessageChars, MaxRows: maxRows, MaxPerRow: maxPerRow}
}

func (p *Panel) view() settings.View {
	return settings.View{Limits: Limits(p.prefix), Pickers: true}
}

func actorFor(user string) *settings.Actor {
	return &settings.Actor{Domain: UserDomain, ID: user}
}

// Open posts the home screen (or a section) in a channel, acting for user.
func (p *Panel) Open(ctx context.Context, channelID, userID string, page settings.Page) error {
	screen, err := p.client.Render(ctx, actorFor(userID), page, p.view())
	if err != nil {
		return err
	}
	_, err = p.api.ChannelMessageSendComplex(channelID, &discordgo.MessageSend{
		Content:    Markdown(screen.GetText()),
		Components: p.components(screen.GetRows()),
	})
	return err
}

// HandleInteraction handles this panel's component and modal interactions.
// It returns false, leaving the interaction to the host, when the custom id
// is not the panel's.
func (p *Panel) HandleInteraction(ctx context.Context, event *discordgo.InteractionCreate) (bool, error) {
	i := event.Interaction
	user := interactionUser(i)
	switch i.Type {
	case discordgo.InteractionMessageComponent:
		data := i.MessageComponentData()
		if !strings.HasPrefix(data.CustomID, p.prefix) {
			return false, nil
		}
		switch strings.TrimPrefix(data.CustomID, p.prefix) {
		case answerID:
			return true, p.openModal(i, user)
		case pickID:
			question, _, ok := p.questions.Pending(i.ChannelID, user)
			if !ok {
				return true, p.ephemeral(i, "This question has expired; open the setting again.", false)
			}
			input := p.picked(data)
			return true, p.act(ctx, i, false, func(ctx context.Context) (*settingspb.ActResponse, error) {
				return p.client.Answer(ctx, actorFor(user), p.view(), question, input)
			})
		default:
			token := strings.TrimPrefix(data.CustomID, p.prefix)
			return true, p.act(ctx, i, true, func(ctx context.Context) (*settingspb.ActResponse, error) {
				return p.client.Press(ctx, actorFor(user), p.view(), token)
			})
		}
	case discordgo.InteractionModalSubmit:
		data := i.ModalSubmitData()
		if data.CustomID != p.prefix+answerID {
			return false, nil
		}
		question, _, ok := p.questions.Pending(i.ChannelID, user)
		if !ok {
			return true, p.ephemeral(i, "This question has expired; open the setting again.", false)
		}
		input := settings.TextInput(modalValue(data))
		return true, p.act(ctx, i, false, func(ctx context.Context) (*settingspb.ActResponse, error) {
			return p.client.Answer(ctx, actorFor(user), p.view(), question, input)
		})
	}
	return false, nil
}

// act runs a press or an answer and shows its result. Discord wants an
// answer within 3 seconds, so a slow call is acknowledged first and the
// panel edited afterwards. Whatever it was, an accepted press or answer ends
// the user's pending question: an answer answered it, and a press of the panel
// (‹ Back, another page) means they moved on. A press that asks again sets
// the new question in show.
func (p *Panel) act(ctx context.Context, i *discordgo.Interaction, canModal bool, call func(context.Context) (*settingspb.ActResponse, error)) error {
	type result struct {
		response *settingspb.ActResponse
		err      error
	}
	done := make(chan result, 1)
	go func() {
		response, err := call(ctx)
		done <- result{response, err}
	}()
	var got result
	deferred := false
	select {
	case got = <-done:
	case <-time.After(p.deferAfter):
		deferred = true
		if err := p.api.InteractionRespond(i, &discordgo.InteractionResponse{Type: discordgo.InteractionResponseDeferredMessageUpdate}); err != nil {
			return err
		}
		got = <-done
	}
	if got.err != nil {
		return errors.Join(got.err, p.ephemeral(i, errorText(got.err), deferred))
	}
	if got.response.Rejected == nil {
		p.questions.Finish(i.ChannelID, interactionUser(i))
	}
	return p.show(i, got.response, deferred, canModal && !deferred)
}

// show answers an interaction with a response: the redrawn panel, a posted
// one, a question (in a modal when one can be opened), and notices as
// ephemeral messages.
func (p *Panel) show(i *discordgo.Interaction, response *settingspb.ActResponse, deferred, canModal bool) error {
	if rejected := response.GetRejected(); rejected != "" {
		return p.ephemeral(i, rejected, deferred)
	}
	user := interactionUser(i)
	var content string
	var components []discordgo.MessageComponent
	update := false
	if screen := response.GetScreen(); screen != nil {
		update = true
		content = Markdown(screen.GetText())
		if !response.GetClosed() {
			components = p.components(screen.GetRows())
		}
	}

	var errs []error
	if ask := response.GetAsk(); ask != nil {
		messageID := ""
		if i.Message != nil {
			messageID = i.Message.ID
		}
		p.questions.Ask(i.ChannelID, user, ask, messageID)
		if menu := p.pickerMenu(ask); menu != nil {
			update = true
			content = Markdown(ask.GetText())
			components = append([]discordgo.MessageComponent{discordgo.ActionsRow{Components: []discordgo.MessageComponent{menu}}}, p.components(askRows(ask))...)
		} else if canModal {
			return errors.Join(p.api.InteractionRespond(i, &discordgo.InteractionResponse{Type: discordgo.InteractionResponseModal, Data: p.modal(ask)}), p.notice(i, response))
		} else {
			// A modal can only answer a button press: offer one that opens it.
			update = true
			content = Markdown(ask.GetText())
			components = append([]discordgo.MessageComponent{discordgo.ActionsRow{Components: []discordgo.MessageComponent{
				discordgo.Button{Label: "✏️ Answer", Style: discordgo.PrimaryButton, CustomID: p.prefix + answerID},
			}}}, p.components(askRows(ask))...)
		}
	}

	switch {
	case update && deferred:
		_, err := p.api.InteractionResponseEdit(i, &discordgo.WebhookEdit{Content: &content, Components: &components})
		errs = append(errs, err)
	case update:
		errs = append(errs, p.api.InteractionRespond(i, &discordgo.InteractionResponse{
			Type: discordgo.InteractionResponseUpdateMessage,
			Data: &discordgo.InteractionResponseData{Content: content, Components: components},
		}))
	case !deferred:
		// Nothing to redraw: acknowledge the press.
		errs = append(errs, p.api.InteractionRespond(i, &discordgo.InteractionResponse{Type: discordgo.InteractionResponseDeferredMessageUpdate}))
	}
	if post := response.GetPost(); post != nil {
		_, err := p.api.ChannelMessageSendComplex(i.ChannelID, &discordgo.MessageSend{
			Content:    Markdown(post.GetText()),
			Components: p.components(post.GetRows()),
		})
		errs = append(errs, err)
	}
	errs = append(errs, p.notice(i, response))
	return errors.Join(errs...)
}

// notice shows a response's notice (a stale button's too) as an ephemeral
// follow-up, once the interaction has been answered.
func (p *Panel) notice(i *discordgo.Interaction, response *settingspb.ActResponse) error {
	if response.GetNotice() == "" {
		return nil
	}
	_, err := p.api.FollowupMessageCreate(i, false, &discordgo.WebhookParams{Content: response.GetNotice(), Flags: discordgo.MessageFlagsEphemeral})
	return err
}

// ephemeral shows text to the interaction's user only.
func (p *Panel) ephemeral(i *discordgo.Interaction, text string, deferred bool) error {
	if deferred {
		_, err := p.api.FollowupMessageCreate(i, false, &discordgo.WebhookParams{Content: text, Flags: discordgo.MessageFlagsEphemeral})
		return err
	}
	return p.api.InteractionRespond(i, &discordgo.InteractionResponse{
		Type: discordgo.InteractionResponseChannelMessageWithSource,
		Data: &discordgo.InteractionResponseData{Content: text, Flags: discordgo.MessageFlagsEphemeral},
	})
}

// openModal opens the modal of the user's pending question.
func (p *Panel) openModal(i *discordgo.Interaction, user string) error {
	if _, _, ok := p.questions.Pending(i.ChannelID, user); !ok {
		return p.ephemeral(i, "This question has expired; open the setting again.", false)
	}
	return p.api.InteractionRespond(i, &discordgo.InteractionResponse{
		Type: discordgo.InteractionResponseModal,
		Data: p.modal(&settingspb.Ask{Title: "Settings"}),
	})
}

// modal asks a question in a modal with one text input.
func (p *Panel) modal(ask *settingspb.Ask) *discordgo.InteractionResponseData {
	title := cut(ask.GetTitle(), maxTitleChars)
	if title == "" {
		title = "Settings"
	}
	input := discordgo.TextInput{
		CustomID:    valueID,
		Label:       title,
		Style:       discordgo.TextInputParagraph,
		Placeholder: cut(plainText(ask.GetText()), 100),
		Required:    true,
	}
	if ask.Current != nil {
		input.Value = ask.GetCurrent()
	}
	return &discordgo.InteractionResponseData{
		CustomID:   p.prefix + answerID,
		Title:      title,
		Components: []discordgo.MessageComponent{discordgo.ActionsRow{Components: []discordgo.MessageComponent{input}}},
	}
}

// pickerMenu is Discord's user or channel select menu for a question that
// offers a picker, or nil.
func (p *Panel) pickerMenu(ask *settingspb.Ask) discordgo.MessageComponent {
	picker := ask.GetPicker()
	var menuType discordgo.SelectMenuType
	switch picker.GetDomain() {
	case UserDomain:
		menuType = discordgo.UserSelectMenu
	case ChannelDomain:
		menuType = discordgo.ChannelSelectMenu
	default:
		return nil
	}
	maxValues := 1
	if picker.GetMany() {
		maxValues = 25
	}
	return discordgo.SelectMenu{
		MenuType:    menuType,
		CustomID:    p.prefix + pickID,
		Placeholder: cut(ask.GetTitle(), 150),
		MaxValues:   maxValues,
	}
}

// picked turns a select menu's values into the answer's ids, each a JSON
// string in the domain's form.
func (p *Panel) picked(data discordgo.MessageComponentInteractionData) settings.Input {
	domain := UserDomain
	if data.ComponentType == discordgo.ChannelSelectMenuComponent {
		domain = ChannelDomain
	}
	ids := make([]json.RawMessage, 0, len(data.Values))
	for _, value := range data.Values {
		id, _ := json.Marshal(value)
		ids = append(ids, id)
	}
	return settings.PickedInput(domain, ids...)
}

// components turns rows of panel buttons into action rows whose custom ids
// are the prefix followed by each button's token.
func (p *Panel) components(rows []*settingspb.Row) []discordgo.MessageComponent {
	var components []discordgo.MessageComponent
	for _, row := range rows {
		var buttons []discordgo.MessageComponent
		for _, button := range row.GetButtons() {
			if len(buttons) == maxPerRow {
				break
			}
			buttons = append(buttons, discordgo.Button{
				Label:    cut(button.GetLabel(), maxLabelChars),
				Style:    discordgo.SecondaryButton,
				CustomID: p.prefix + button.GetToken(),
			})
		}
		if len(buttons) == 0 {
			continue
		}
		if len(components) == maxRows {
			break
		}
		components = append(components, discordgo.ActionsRow{Components: buttons})
	}
	return components
}

// interactionUser is the id of whoever caused an interaction.
func interactionUser(i *discordgo.Interaction) string {
	if i.Member != nil && i.Member.User != nil {
		return i.Member.User.ID
	}
	if i.User != nil {
		return i.User.ID
	}
	return ""
}

// modalValue is the text typed in the modal.
func modalValue(data discordgo.ModalSubmitInteractionData) string {
	for _, component := range data.Components {
		var row []discordgo.MessageComponent
		switch component := component.(type) {
		case *discordgo.ActionsRow:
			row = component.Components
		case discordgo.ActionsRow:
			row = component.Components
		}
		for _, field := range row {
			switch field := field.(type) {
			case *discordgo.TextInput:
				return field.Value
			case discordgo.TextInput:
				return field.Value
			}
		}
	}
	return ""
}

// cut shortens text to max characters, with an ellipsis.
func cut(text string, max int) string {
	if utf8.RuneCountInString(text) <= max {
		return text
	}
	runes := []rune(text)
	return string(runes[:max-1]) + "…"
}

// errorText is what a user sees when a call fails.
func errorText(err error) string {
	var invalid *settings.InvalidValueError
	switch {
	case errors.As(err, &invalid):
		return invalid.Reason
	case errors.Is(err, settings.ErrPermissionDenied):
		return "You may not change these settings."
	case errors.Is(err, settings.ErrUnavailable):
		return "The settings are unavailable right now; try again later."
	default:
		return "Something went wrong with the settings."
	}
}

// askRows are a question's buttons: its suggestions, then ‹ Back.
func askRows(ask *settingspb.Ask) []*settingspb.Row {
	rows := append([]*settingspb.Row(nil), ask.GetRows()...)
	if back := ask.GetBack(); back != nil {
		rows = append(rows, &settingspb.Row{Buttons: []*settingspb.Button{back}})
	}
	return rows
}
