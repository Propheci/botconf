package telegram

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/gotd/td/tg"
	"github.com/gotd/td/tgerr"
	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settingspb"
)

// DefaultPrefix is the callback data prefix of panel buttons unless another
// is set; it matches botconf's own Telegram panel.
const DefaultPrefix = "cfg:"

// pickerButtonID tags the reply-keyboard button that opens Telegram's picker.
const pickerButtonID = 1

// BotPanel shows a settings panel through a Telegram bot session: the panel is
// a message with an inline keyboard, presses arrive as callback queries, and
// questions are answered with the user's next message in the chat.
type BotPanel struct {
	client      *settings.Client
	api         *tg.Client
	prefix      string
	pickers     bool
	resolvePeer func(tg.PeerClass) (tg.InputPeerClass, error)
	questions   *settings.Questions
}

// BotOption configures a BotPanel.
type BotOption func(*BotPanel)

// WithPrefix sets the callback data prefix of the panel's buttons (default
// DefaultPrefix). Callback data with another prefix is left to the host.
func WithPrefix(prefix string) BotOption {
	return func(p *BotPanel) { p.prefix = prefix }
}

// WithPickers offers Telegram's user and chat pickers for questions about
// telegram.user and telegram.chat references, in private chats (the only
// place Telegram shows them).
func WithPickers() BotOption {
	return func(p *BotPanel) { p.pickers = true }
}

// WithPeerResolver sets how the chat a button was pressed in is addressed
// (default ZeroHashPeer), e.g. from the host's access-hash cache.
func WithPeerResolver(resolve func(tg.PeerClass) (tg.InputPeerClass, error)) BotOption {
	return func(p *BotPanel) { p.resolvePeer = resolve }
}

// NewBotPanel shows the panel served through client with the bot's api.
func NewBotPanel(client *settings.Client, api *tg.Client, opts ...BotOption) *BotPanel {
	p := &BotPanel{
		client:      client,
		api:         api,
		prefix:      DefaultPrefix,
		resolvePeer: ZeroHashPeer,
		questions:   settings.NewQuestions(10 * time.Minute),
	}
	for _, opt := range opts {
		opt(p)
	}
	return p
}

// view is what the panel can show in peer.
func (p *BotPanel) view(peer tg.InputPeerClass) settings.View {
	_, private := peer.(*tg.InputPeerUser)
	return settings.View{Limits: Limits(p.prefix), Pickers: p.pickers && private}
}

// Open posts the home screen (or a section) in peer, acting for user.
func (p *BotPanel) Open(ctx context.Context, peer tg.InputPeerClass, user int64, page settings.Page) error {
	screen, err := p.client.Render(ctx, actorFor(user), page, p.view(peer))
	if err != nil {
		return err
	}
	_, err = p.send(ctx, peer, screen.GetText(), markup(p.prefix, screen.GetRows()), 0)
	return err
}

// Command runs the text form of the settings command for user ("set x 1",
// "help"); empty args open the panel. msgID is the command's message, which
// text replies answer.
func (p *BotPanel) Command(ctx context.Context, peer tg.InputPeerClass, user int64, msgID int, args string) error {
	response, err := p.client.Command(ctx, actorFor(user), p.view(peer), strings.TrimSpace(args))
	if err != nil {
		_, sendErr := p.send(ctx, peer, plain(errorText(err)), nil, msgID)
		return errors.Join(err, sendErr)
	}
	return p.react(ctx, reaction{peer: peer, user: user, replyTo: msgID}, response)
}

// HandleCallback handles a press of one of this panel's buttons. It returns
// false, leaving the update to the host, when the data is not the panel's.
// Every press it takes is answered exactly once.
func (p *BotPanel) HandleCallback(ctx context.Context, update *tg.UpdateBotCallbackQuery) (bool, error) {
	data := string(update.Data)
	if !strings.HasPrefix(data, p.prefix) {
		return false, nil
	}
	answered := false
	answer := func(text string, alert bool) error {
		answered = true
		_, err := p.api.MessagesSetBotCallbackAnswer(ctx, &tg.MessagesSetBotCallbackAnswerRequest{
			QueryID: update.QueryID,
			Message: text,
			Alert:   alert,
		})
		return err
	}
	defer func() {
		if !answered {
			_ = answer("", false)
		}
	}()

	peer, err := p.resolvePeer(update.Peer)
	if err != nil {
		return true, errors.Join(err, answer(errorText(err), true))
	}
	response, err := p.client.Press(ctx, actorFor(update.UserID), p.view(peer), strings.TrimPrefix(data, p.prefix))
	if err != nil {
		return true, errors.Join(err, answer(errorText(err), true))
	}
	return true, p.react(ctx, reaction{peer: peer, user: update.UserID, panel: update.MsgID, answer: answer}, response)
}

// HandleMessage takes msg as the answer to a pending question from its sender
// in peer. It returns false when no question is waiting for them.
func (p *BotPanel) HandleMessage(ctx context.Context, peer tg.InputPeerClass, msg *tg.Message) (bool, error) {
	user, ok := senderOf(msg)
	if !ok {
		return false, nil
	}
	input := settings.TextInput(msg.Message)
	if isCancel(msg.Message) {
		input = settings.CancelInput()
	}
	return p.answer(ctx, peer, user, msg.ID, input)
}

// HandleService takes the chats or users picked with Telegram's picker (a
// service message) as the answer to a pending question. It returns false
// for other service messages.
func (p *BotPanel) HandleService(ctx context.Context, peer tg.InputPeerClass, msg *tg.MessageService) (bool, error) {
	action, ok := msg.Action.(*tg.MessageActionRequestedPeer)
	if !ok || action.ButtonID != pickerButtonID {
		return false, nil
	}
	user, ok := msg.FromID.(*tg.PeerUser)
	if !ok {
		peerUser, private := msg.PeerID.(*tg.PeerUser)
		if !private {
			return false, nil
		}
		user = peerUser
	}
	var domain string
	var ids []json.RawMessage
	for _, picked := range action.Peers {
		id, pickedDomain, ok := botAPIChatID(picked)
		if !ok {
			continue
		}
		if domain == "" {
			domain = pickedDomain
		}
		ids = append(ids, id)
	}
	return p.answer(ctx, peer, user.UserID, msg.ID, settings.PickedInput(domain, ids...))
}

// answer sends input for user's pending question in peer.
func (p *BotPanel) answer(ctx context.Context, peer tg.InputPeerClass, user int64, msgID int, input settings.Input) (bool, error) {
	conversation, userKey := inputPeerKey(peer), strconv.FormatInt(user, 10)
	question, ref, ok := p.questions.Pending(conversation, userKey)
	if !ok {
		return false, nil
	}
	panel, questionMsg := parseRef(ref)
	response, err := p.client.Answer(ctx, actorFor(user), p.view(peer), question, input)
	if err != nil {
		_, sendErr := p.send(ctx, peer, plain(errorText(err)), nil, msgID)
		return true, errors.Join(err, sendErr)
	}
	if response.Rejected != nil {
		// The question stays open for another try.
		_, err := p.send(ctx, peer, plain(response.GetRejected()), nil, msgID)
		return true, err
	}
	p.questions.Finish(conversation, userKey)
	p.deleteMessages(ctx, peer, questionMsg, msgID)
	return true, p.react(ctx, reaction{peer: peer, user: user, panel: panel}, response)
}

// reaction is where a response lands: the chat, who acted, the panel message
// (0: none), the message to reply to, and the callback to answer if any.
type reaction struct {
	peer    tg.InputPeerClass
	user    int64
	panel   int
	replyTo int
	answer  func(text string, alert bool) error
}

// react shows a response: toasts, the redrawn or posted panel, a question,
// or a text reply.
func (p *BotPanel) react(ctx context.Context, at reaction, response *settingspb.ActResponse) error {
	if response.Rejected != nil {
		if at.answer != nil {
			return at.answer(response.GetRejected(), true)
		}
		_, err := p.send(ctx, at.peer, plain(response.GetRejected()), nil, at.replyTo)
		return err
	}
	var errs []error
	screen := response.GetScreen()
	if notice := response.GetNotice(); notice != "" {
		if at.answer != nil {
			errs = append(errs, at.answer(notice, false))
		} else if screen != nil {
			screen = withNotice(screen, notice)
		} else {
			_, err := p.send(ctx, at.peer, plain(notice), nil, at.replyTo)
			errs = append(errs, err)
		}
	}
	if screen != nil {
		rows := screen.GetRows()
		if response.GetClosed() {
			rows = nil
		}
		if at.panel != 0 {
			errs = append(errs, p.edit(ctx, at.peer, at.panel, screen.GetText(), markup(p.prefix, rows)))
		} else {
			panel, err := p.send(ctx, at.peer, screen.GetText(), markup(p.prefix, rows), at.replyTo)
			errs = append(errs, err)
			at.panel = panel
		}
	}
	if post := response.GetPost(); post != nil {
		panel, err := p.send(ctx, at.peer, post.GetText(), markup(p.prefix, post.GetRows()), at.replyTo)
		errs = append(errs, err)
		if at.panel == 0 {
			at.panel = panel
		}
	}
	if ask := response.GetAsk(); ask != nil {
		errs = append(errs, p.ask(ctx, at, ask))
	}
	if reply := response.GetReply(); reply != nil {
		_, err := p.send(ctx, at.peer, reply, nil, at.replyTo)
		errs = append(errs, err)
	}
	return errors.Join(errs...)
}

// ask sends a question and keeps it for the user's answer: with the
// suggested answers as buttons, Telegram's picker, or a force-reply.
func (p *BotPanel) ask(ctx context.Context, at reaction, ask *settingspb.Ask) error {
	var replyMarkup tg.ReplyMarkupClass
	if len(ask.GetRows()) > 0 {
		replyMarkup = markup(p.prefix, ask.GetRows())
	} else if picker := pickerButton(ask.GetPicker()); picker != nil && p.view(at.peer).Pickers {
		replyMarkup = &tg.ReplyKeyboardMarkup{
			Resize:      true,
			SingleUse:   true,
			Selective:   true,
			Placeholder: ask.GetTitle(),
			Rows:        []tg.KeyboardButtonRow{{Buttons: []tg.KeyboardButtonClass{picker}}},
		}
	} else {
		replyMarkup = &tg.ReplyKeyboardForceReply{Selective: true, Placeholder: ask.GetTitle()}
	}
	questionMsg, err := p.send(ctx, at.peer, askText(ask), replyMarkup, at.replyTo)
	if err != nil {
		return err
	}
	p.questions.Ask(inputPeerKey(at.peer), strconv.FormatInt(at.user, 10), ask, formatRef(at.panel, questionMsg))
	return nil
}

// pickerButton is the reply-keyboard button opening Telegram's picker for a
// domain, or nil when Telegram has none for it.
func pickerButton(picker *settingspb.Picker) tg.KeyboardButtonClass {
	quantity := 1
	if picker.GetMany() {
		quantity = 10
	}
	switch picker.GetDomain() {
	case UserDomain:
		return &tg.KeyboardButtonRequestPeer{Text: "👤 Pick", ButtonID: pickerButtonID, PeerType: &tg.RequestPeerTypeUser{}, MaxQuantity: quantity}
	case ChatDomain:
		return &tg.KeyboardButtonRequestPeer{Text: "👥 Pick", ButtonID: pickerButtonID, PeerType: &tg.RequestPeerTypeChat{}, MaxQuantity: quantity}
	default:
		return nil
	}
}

// send posts text in peer, as a reply to replyTo when it is not 0, and
// returns the new message's id.
func (p *BotPanel) send(ctx context.Context, peer tg.InputPeerClass, text *settingspb.Text, replyMarkup tg.ReplyMarkupClass, replyTo int) (int, error) {
	message, entities := Entities(text, 0)
	request := &tg.MessagesSendMessageRequest{
		Peer:        peer,
		Message:     message,
		Entities:    entities,
		ReplyMarkup: replyMarkup,
		RandomID:    randomID(),
	}
	if replyTo != 0 {
		request.ReplyTo = &tg.InputReplyToMessage{ReplyToMsgID: replyTo}
	}
	updates, err := p.api.MessagesSendMessage(ctx, request)
	if err != nil {
		return 0, fmt.Errorf("send settings panel message: %w", err)
	}
	return sentMessageID(updates), nil
}

// edit redraws the panel message; an unchanged panel is not an error.
func (p *BotPanel) edit(ctx context.Context, peer tg.InputPeerClass, msgID int, text *settingspb.Text, replyMarkup tg.ReplyMarkupClass) error {
	message, entities := Entities(text, 0)
	_, err := p.api.MessagesEditMessage(ctx, &tg.MessagesEditMessageRequest{
		Peer:        peer,
		ID:          msgID,
		Message:     message,
		Entities:    entities,
		ReplyMarkup: replyMarkup,
	})
	if err != nil && !tgerr.Is(err, "MESSAGE_NOT_MODIFIED") {
		return fmt.Errorf("edit settings panel message: %w", err)
	}
	return nil
}

// deleteMessages removes the question and its answer, best effort: a bot
// cannot always delete a user's message.
func (p *BotPanel) deleteMessages(ctx context.Context, peer tg.InputPeerClass, ids ...int) {
	deleteMessages(ctx, p.api, peer, ids...)
}

// deleteMessages removes messages in peer, ignoring failures.
func deleteMessages(ctx context.Context, api *tg.Client, peer tg.InputPeerClass, ids ...int) {
	var kept []int
	for _, id := range ids {
		if id != 0 {
			kept = append(kept, id)
		}
	}
	if len(kept) == 0 {
		return
	}
	if channel, ok := peer.(*tg.InputPeerChannel); ok {
		_, _ = api.ChannelsDeleteMessages(ctx, &tg.ChannelsDeleteMessagesRequest{
			Channel: &tg.InputChannel{ChannelID: channel.ChannelID, AccessHash: channel.AccessHash},
			ID:      kept,
		})
		return
	}
	_, _ = api.MessagesDeleteMessages(ctx, &tg.MessagesDeleteMessagesRequest{Revoke: true, ID: kept})
}

// sentMessageID finds the id of the message a send created.
func sentMessageID(updates tg.UpdatesClass) int {
	switch updates := updates.(type) {
	case *tg.UpdateShortSentMessage:
		return updates.ID
	case *tg.Updates:
		return idFromUpdates(updates.Updates)
	case *tg.UpdatesCombined:
		return idFromUpdates(updates.Updates)
	}
	return 0
}

func idFromUpdates(updates []tg.UpdateClass) int {
	for _, update := range updates {
		switch update := update.(type) {
		case *tg.UpdateMessageID:
			return update.ID
		case *tg.UpdateNewMessage:
			return update.Message.GetID()
		case *tg.UpdateNewChannelMessage:
			return update.Message.GetID()
		}
	}
	return 0
}

// formatRef keeps the panel and question message ids with a question.
func formatRef(panel, question int) string {
	return strconv.Itoa(panel) + "/" + strconv.Itoa(question)
}

func parseRef(ref string) (panel, question int) {
	before, after, _ := strings.Cut(ref, "/")
	panel, _ = strconv.Atoi(before)
	question, _ = strconv.Atoi(after)
	return panel, question
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
