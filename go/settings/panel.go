package settings

import (
	"context"
	"encoding/json"

	"github.com/propheci/botconf/go/settingspb"
)

// Page is a page of the settings panel: the home screen or one section.
type Page struct {
	section string
}

// Home is the panel's home screen: the core settings and a button per
// section.
func Home() Page { return Page{} }

// Section is the page of one section, by its id.
func Section(id string) Page { return Page{section: id} }

func (p Page) proto() *settingspb.Page {
	if p.section == "" {
		return &settingspb.Page{Page: &settingspb.Page_Home{Home: &settingspb.HomePage{}}}
	}
	return &settingspb.Page{Page: &settingspb.Page_Section{Section: p.section}}
}

// View is what the client can display where the panel is shown.
type View struct {
	// Limits make screens and tokens fit; nil means Telegram's.
	Limits *settingspb.Limits
	// Pickers says whether the platform's pickers (e.g. Telegram's user
	// picker) can be offered in this conversation.
	Pickers bool
}

// Input answers one of the panel's questions: typed text, ids picked with a
// picker, or giving up.
type Input struct {
	text   *string
	picked *settingspb.Picked
}

// TextInput is a typed answer.
func TextInput(text string) Input { return Input{text: &text} }

// PickedInput is ids picked with the platform's picker, each as JSON in the
// domain's form (e.g. `123456789`).
func PickedInput(domain string, ids ...json.RawMessage) Input {
	picked := &settingspb.Picked{Domain: domain}
	for _, id := range ids {
		picked.IdsJson = append(picked.IdsJson, string(id))
	}
	return Input{picked: picked}
}

// CancelInput gives up on the question.
func CancelInput() Input { return Input{} }

func (i Input) proto(question string) *settingspb.Answer {
	answer := &settingspb.Answer{Question: question}
	switch {
	case i.text != nil:
		answer.Input = &settingspb.Answer_Text{Text: *i.text}
	case i.picked != nil:
		answer.Input = &settingspb.Answer_Picked{Picked: i.picked}
	default:
		answer.Input = &settingspb.Answer_Cancel{Cancel: &settingspb.Cancel{}}
	}
	return answer
}

// Render returns a page of the panel, to show as a new message.
func (c *Client) Render(ctx context.Context, actor *Actor, page Page, view View) (*settingspb.Screen, error) {
	response, err := c.api.Render(c.call(ctx), &settingspb.RenderRequest{
		Actor:   c.actor(actor),
		Page:    page.proto(),
		Limits:  view.Limits,
		Pickers: view.Pickers,
	})
	if err != nil {
		return nil, convert(err)
	}
	return response.GetScreen(), nil
}

// Press presses the panel button carrying token.
func (c *Client) Press(ctx context.Context, actor *Actor, view View, token string) (*settingspb.ActResponse, error) {
	return c.act(ctx, actor, view, func(request *settingspb.ActRequest) {
		request.Act = &settingspb.ActRequest_Press{Press: token}
	})
}

// Answer answers question (as Ask.Question gave it) with input.
func (c *Client) Answer(ctx context.Context, actor *Actor, view View, question string, input Input) (*settingspb.ActResponse, error) {
	return c.act(ctx, actor, view, func(request *settingspb.ActRequest) {
		request.Act = &settingspb.ActRequest_Answer{Answer: input.proto(question)}
	})
}

// Command runs the text form of the settings command with its arguments
// (empty: open the panel; `help`: the subcommands).
func (c *Client) Command(ctx context.Context, actor *Actor, view View, text string) (*settingspb.ActResponse, error) {
	return c.act(ctx, actor, view, func(request *settingspb.ActRequest) {
		request.Act = &settingspb.ActRequest_Command{Command: text}
	})
}

func (c *Client) act(ctx context.Context, actor *Actor, view View, set func(*settingspb.ActRequest)) (*settingspb.ActResponse, error) {
	request := &settingspb.ActRequest{Actor: c.actor(actor), Limits: view.Limits, Pickers: view.Pickers}
	set(request)
	response, err := c.api.Act(c.call(ctx), request)
	return response, convert(err)
}
