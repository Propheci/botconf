package telegram

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"

	gotd "github.com/gotd/td/telegram"
	"github.com/gotd/td/tg"
	"github.com/gotd/td/tgerr"
	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settingspb"
)

const (
	// inlineTokenTTL is how long a screen waits for its inline query.
	inlineTokenTTL = 2 * time.Minute
	// inlineTimeout bounds the trip through the helper bot before falling
	// back to text.
	inlineTimeout = 5 * time.Second
	// inlineQuestionTTL is how long a question waits for its answer.
	inlineQuestionTTL = 10 * time.Minute
	// inlineResultID is the id of the one inline result a panel query gets.
	inlineResultID = "panel"
)

// HelperBot is the helper bot's session: its API, and invokers for other
// data centers (an inline message must be edited on its own DC).
// *telegram.Client implements it.
type HelperBot interface {
	API() *tg.Client
	DC(ctx context.Context, dc int, max int64) (gotd.CloseInvoker, error)
}

// InlinePanel shows a settings panel from a Telegram user account, which
// cannot attach buttons to its messages, through a helper bot in inline mode:
// the account asks the bot for an inline result holding the screen and sends
// it, so the message carries the bot's buttons; presses then reach the bot,
// which edits the message. Without a helper bot (or when it does not answer)
// the panel falls back to text with numbered options.
//
// Only the account itself (ownUserID) drives the panel: the bot answers no
// one else's inline queries, and questions are answered with the account's
// own messages.
type InlinePanel struct {
	client    *settings.Client
	user      *tg.Client
	bot       HelperBot
	botUser   tg.InputUserClass
	ownUserID int64
	prefix    string
	timeout   time.Duration
	now       func() time.Time

	mu       sync.Mutex
	screens  map[string]pendingScreen
	lastPeer tg.InputPeerClass
	question *inlineQuestion
	fallback map[string]fallbackPanel
}

// pendingScreen is a screen waiting for the helper bot's inline query.
type pendingScreen struct {
	screen  *settingspb.Screen
	expires time.Time
}

// inlineQuestion is the question waiting for the account's next message in
// a chat; message is the inline panel to redraw, nil for a text panel.
type inlineQuestion struct {
	question string
	peer     string
	message  tg.InputBotInlineMessageIDClass
	expires  time.Time
}

// fallbackPanel is the numbered options of the last text panel in a chat.
type fallbackPanel struct {
	tokens  []string
	expires time.Time
}

// InlineOption configures an InlinePanel.
type InlineOption func(*InlinePanel)

// WithInlinePrefix sets the callback data prefix of the panel's buttons
// (default DefaultPrefix). The helper bot's other callbacks are left to the
// host.
func WithInlinePrefix(prefix string) InlineOption {
	return func(p *InlinePanel) { p.prefix = prefix }
}

// WithInlineTimeout bounds the trip through the helper bot (default 5s)
// before the panel falls back to text.
func WithInlineTimeout(timeout time.Duration) InlineOption {
	return func(p *InlinePanel) { p.timeout = timeout }
}

// NewInlinePanel shows the panel served through client from the account
// whose API is user and whose id is ownUserID, through the helper bot (bot,
// addressed by the account as botUser). A nil bot shows text panels only.
func NewInlinePanel(client *settings.Client, user *tg.Client, bot HelperBot, botUser tg.InputUserClass, ownUserID int64, opts ...InlineOption) *InlinePanel {
	if typed, ok := bot.(*gotd.Client); ok && typed == nil {
		bot = nil
	}
	p := &InlinePanel{
		client:    client,
		user:      user,
		bot:       bot,
		botUser:   botUser,
		ownUserID: ownUserID,
		prefix:    DefaultPrefix,
		timeout:   inlineTimeout,
		now:       time.Now,
		screens:   map[string]pendingScreen{},
		fallback:  map[string]fallbackPanel{},
	}
	for _, opt := range opts {
		opt(p)
	}
	return p
}

// view is what an inline panel can show: Telegram's limits, no pickers (a
// user account has none to offer).
func (p *InlinePanel) view() settings.View {
	return settings.View{Limits: Limits(p.prefix)}
}

func (p *InlinePanel) actor() *settings.Actor {
	return actorFor(p.ownUserID)
}

// Open renders page and posts it in peer from the account.
func (p *InlinePanel) Open(ctx context.Context, peer tg.InputPeerClass, page settings.Page) error {
	screen, err := p.client.Render(ctx, p.actor(), page, p.view())
	if err != nil {
		return err
	}
	return p.show(ctx, peer, screen)
}

// Command runs the text form of the settings command from the account in
// peer; empty args post the panel. msgID is the command's message, which
// text replies answer.
func (p *InlinePanel) Command(ctx context.Context, peer tg.InputPeerClass, msgID int, args string) error {
	response, err := p.client.Command(ctx, p.actor(), p.view(), strings.TrimSpace(args))
	if err != nil {
		return errors.Join(err, p.sendText(ctx, peer, plain(errorText(err)), msgID))
	}
	return p.reactText(ctx, peer, msgID, response)
}

// show posts screen in peer through the helper bot, or as text when that
// fails.
func (p *InlinePanel) show(ctx context.Context, peer tg.InputPeerClass, screen *settingspb.Screen) error {
	p.mu.Lock()
	p.lastPeer = peer
	p.mu.Unlock()
	if p.bot == nil {
		return p.showText(ctx, peer, screen, 0)
	}

	token := newInlineToken()
	p.mu.Lock()
	p.screens[token] = pendingScreen{screen: screen, expires: p.now().Add(inlineTokenTTL)}
	p.mu.Unlock()

	sent := func() error {
		ctx, cancel := context.WithTimeout(ctx, p.timeout)
		defer cancel()
		results, err := p.user.MessagesGetInlineBotResults(ctx, &tg.MessagesGetInlineBotResultsRequest{
			Bot:   p.botUser,
			Peer:  peer,
			Query: token,
		})
		if err != nil {
			return err
		}
		if len(results.Results) == 0 {
			return errors.New("the helper bot returned no panel")
		}
		_, err = p.user.MessagesSendInlineBotResult(ctx, &tg.MessagesSendInlineBotResultRequest{
			Peer:     peer,
			RandomID: randomID(),
			QueryID:  results.QueryID,
			ID:       results.Results[0].GetID(),
		})
		return err
	}()
	p.mu.Lock()
	delete(p.screens, token)
	p.mu.Unlock()
	if sent == nil {
		return nil
	}
	if err := p.showText(ctx, peer, screen, 0); err != nil {
		return errors.Join(fmt.Errorf("show the panel through the helper bot: %w", sent), err)
	}
	return nil
}

// HandleInlineQuery answers the helper bot's inline query for a panel. It
// returns false, leaving the query to the host, when the query is not a
// panel token of the account's.
func (p *InlinePanel) HandleInlineQuery(ctx context.Context, update *tg.UpdateBotInlineQuery) (bool, error) {
	if p.bot == nil {
		return false, nil
	}
	own := update.UserID == p.ownUserID
	p.mu.Lock()
	pending, ok := p.screens[update.Query]
	if ok && own {
		// Single use.
		delete(p.screens, update.Query)
	}
	p.mu.Unlock()
	if !ok {
		return false, nil
	}

	// Anyone else asking for a panel token gets nothing, and the token stays
	// usable by the account.
	request := &tg.MessagesSetInlineBotResultsRequest{QueryID: update.QueryID, CacheTime: 0, Private: true}
	if own && p.now().Before(pending.expires) {
		message, entities := Entities(pending.screen.GetText(), 0)
		request.Results = []tg.InputBotInlineResultClass{&tg.InputBotInlineResult{
			ID:    inlineResultID,
			Type:  "article",
			Title: "Settings",
			SendMessage: &tg.InputBotInlineMessageText{
				Message:     message,
				Entities:    entities,
				ReplyMarkup: markup(p.prefix, pending.screen.GetRows()),
			},
		}}
	}
	_, err := p.bot.API().MessagesSetInlineBotResults(ctx, request)
	return true, err
}

// HandleInlineCallback handles a press of an inline panel's button, which
// reaches the helper bot. It returns false when the data is not the panel's.
// Every press it takes is answered exactly once.
func (p *InlinePanel) HandleInlineCallback(ctx context.Context, update *tg.UpdateInlineBotCallbackQuery) (bool, error) {
	data := string(update.Data)
	if p.bot == nil || !strings.HasPrefix(data, p.prefix) {
		return false, nil
	}
	answered := false
	answer := func(text string, alert bool) error {
		answered = true
		_, err := p.bot.API().MessagesSetBotCallbackAnswer(ctx, &tg.MessagesSetBotCallbackAnswerRequest{
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

	response, err := p.client.Press(ctx, actorFor(update.UserID), p.view(), strings.TrimPrefix(data, p.prefix))
	if err != nil {
		return true, errors.Join(err, answer(errorText(err), true))
	}
	if response.Rejected != nil {
		return true, answer(response.GetRejected(), true)
	}
	var errs []error
	if notice := response.GetNotice(); notice != "" {
		errs = append(errs, answer(notice, false))
	}
	errs = append(errs, p.reactInline(ctx, update.MsgID, response))
	return true, errors.Join(errs...)
}

// reactInline shows a response on an inline panel message. A post replaces
// the panel too: the helper bot cannot tell which chat the message is in.
func (p *InlinePanel) reactInline(ctx context.Context, id tg.InputBotInlineMessageIDClass, response *settingspb.ActResponse) error {
	screen := response.GetScreen()
	if post := response.GetPost(); post != nil {
		screen = post
	}
	if ask := response.GetAsk(); ask != nil {
		p.mu.Lock()
		peer := ""
		if p.lastPeer != nil {
			peer = inputPeerKey(p.lastPeer)
		}
		p.question = &inlineQuestion{question: ask.GetQuestion(), peer: peer, message: id, expires: p.now().Add(inlineQuestionTTL)}
		p.mu.Unlock()
		return p.editInline(ctx, id, &settingspb.Screen{Text: askText(ask), Rows: ask.GetRows()}, false)
	}
	if screen == nil {
		return nil
	}
	return p.editInline(ctx, id, screen, response.GetClosed())
}

// editInline redraws an inline message, on the data center that holds it.
func (p *InlinePanel) editInline(ctx context.Context, id tg.InputBotInlineMessageIDClass, screen *settingspb.Screen, closed bool) error {
	invoker, err := p.bot.DC(ctx, id.GetDCID(), 1)
	if err != nil {
		return fmt.Errorf("reach the inline panel's data center %d: %w", id.GetDCID(), err)
	}
	defer func() { _ = invoker.Close() }()

	message, entities := Entities(screen.GetText(), 0)
	request := &tg.MessagesEditInlineBotMessageRequest{ID: id, Message: message, Entities: entities}
	if !closed {
		request.ReplyMarkup = markup(p.prefix, screen.GetRows())
	}
	if _, err := tg.NewClient(invoker).MessagesEditInlineBotMessage(ctx, request); err != nil && !tgerr.Is(err, "MESSAGE_NOT_MODIFIED") {
		return fmt.Errorf("edit the inline settings panel: %w", err)
	}
	return nil
}

// HandleMessage takes one of the account's own messages in peer as the
// answer to the panel's pending question, or as the number of a text
// panel's option. It returns false for every other message.
func (p *InlinePanel) HandleMessage(ctx context.Context, peer tg.InputPeerClass, msg *tg.Message) (bool, error) {
	if !msg.Out {
		return false, nil
	}
	key := inputPeerKey(peer)
	p.mu.Lock()
	question := p.question
	if question != nil && !p.now().Before(question.expires) {
		p.question, question = nil, nil
	}
	options, numbered := p.fallback[key]
	if numbered && !p.now().Before(options.expires) {
		delete(p.fallback, key)
		numbered = false
	}
	p.mu.Unlock()

	if question != nil && question.peer == key {
		return true, p.answer(ctx, peer, msg, question)
	}
	if numbered {
		choice, err := strconv.Atoi(strings.TrimSpace(msg.Message))
		if err != nil || choice < 1 || choice > len(options.tokens) {
			return false, nil
		}
		deleteMessages(ctx, p.user, peer, msg.ID)
		response, err := p.client.Press(ctx, p.actor(), p.view(), options.tokens[choice-1])
		if err != nil {
			return true, errors.Join(err, p.sendText(ctx, peer, plain(errorText(err)), 0))
		}
		return true, p.reactText(ctx, peer, 0, response)
	}
	return false, nil
}

// answer sends msg as the answer to question, then redraws the panel it
// came from.
func (p *InlinePanel) answer(ctx context.Context, peer tg.InputPeerClass, msg *tg.Message, question *inlineQuestion) error {
	input := settings.TextInput(msg.Message)
	if isCancel(msg.Message) {
		input = settings.CancelInput()
	}
	deleteMessages(ctx, p.user, peer, msg.ID)
	response, err := p.client.Answer(ctx, p.actor(), p.view(), question.question, input)
	if err != nil {
		return errors.Join(err, p.sendText(ctx, peer, plain(errorText(err)), 0))
	}
	if response.Rejected == nil {
		p.mu.Lock()
		if p.question == question {
			p.question = nil
		}
		p.mu.Unlock()
	}
	if question.message == nil {
		return p.reactText(ctx, peer, 0, response)
	}
	if rejected := response.GetRejected(); rejected != "" {
		// The question stays open; say why on the panel.
		return p.editInline(ctx, question.message, withNotice(&settingspb.Screen{Text: plain("Send another value, or /cancel.")}, rejected), false)
	}
	if notice := response.GetNotice(); notice != "" && response.GetScreen() != nil {
		response.Screen = withNotice(response.GetScreen(), notice)
	}
	return p.reactInline(ctx, question.message, response)
}

// reactText shows a response as text panels sent from the account.
func (p *InlinePanel) reactText(ctx context.Context, peer tg.InputPeerClass, replyTo int, response *settingspb.ActResponse) error {
	if rejected := response.GetRejected(); rejected != "" {
		return p.sendText(ctx, peer, plain(rejected), replyTo)
	}
	var errs []error
	screen := response.GetScreen()
	if post := response.GetPost(); post != nil {
		screen = post
	}
	if notice := response.GetNotice(); notice != "" {
		if screen != nil {
			screen = withNotice(screen, notice)
		} else {
			errs = append(errs, p.sendText(ctx, peer, plain(notice), replyTo))
		}
	}
	if screen != nil {
		if response.GetClosed() {
			errs = append(errs, p.sendText(ctx, peer, screen.GetText(), replyTo))
		} else if response.GetPost() != nil && p.bot != nil {
			errs = append(errs, p.show(ctx, peer, screen))
		} else {
			errs = append(errs, p.showText(ctx, peer, screen, replyTo))
		}
	}
	if ask := response.GetAsk(); ask != nil {
		p.mu.Lock()
		p.question = &inlineQuestion{question: ask.GetQuestion(), peer: inputPeerKey(peer), expires: p.now().Add(inlineQuestionTTL)}
		p.mu.Unlock()
		errs = append(errs, p.sendText(ctx, peer, askText(ask), replyTo))
	}
	if reply := response.GetReply(); reply != nil {
		errs = append(errs, p.sendText(ctx, peer, reply, replyTo))
	}
	return errors.Join(errs...)
}

// showText sends screen as text with its buttons as numbered options, and
// remembers them so that a reply with a number presses one.
func (p *InlinePanel) showText(ctx context.Context, peer tg.InputPeerClass, screen *settingspb.Screen, replyTo int) error {
	spans := append([]*settingspb.Span(nil), screen.GetText().GetSpans()...)
	var tokens []string
	for _, row := range screen.GetRows() {
		for _, button := range row.GetButtons() {
			tokens = append(tokens, button.GetToken())
			if len(tokens) == 1 {
				spans = append(spans, &settingspb.Span{Text: "\n"})
			}
			spans = append(spans, &settingspb.Span{Text: fmt.Sprintf("\n%d. %s", len(tokens), button.GetLabel())})
		}
	}
	if len(tokens) > 0 {
		spans = append(spans, &settingspb.Span{Style: settingspb.Style_STYLE_ITALIC, Text: "\n\nSend a number to choose."})
	}
	p.mu.Lock()
	if len(tokens) > 0 {
		p.fallback[inputPeerKey(peer)] = fallbackPanel{tokens: tokens, expires: p.now().Add(inlineQuestionTTL)}
	} else {
		delete(p.fallback, inputPeerKey(peer))
	}
	p.mu.Unlock()
	return p.sendText(ctx, peer, &settingspb.Text{Spans: spans}, replyTo)
}

// sendText sends text from the account.
func (p *InlinePanel) sendText(ctx context.Context, peer tg.InputPeerClass, text *settingspb.Text, replyTo int) error {
	message, entities := Entities(text, 0)
	request := &tg.MessagesSendMessageRequest{Peer: peer, Message: message, Entities: entities, RandomID: randomID()}
	if replyTo != 0 {
		request.ReplyTo = &tg.InputReplyToMessage{ReplyToMsgID: replyTo}
	}
	if _, err := p.user.MessagesSendMessage(ctx, request); err != nil {
		return fmt.Errorf("send settings text: %w", err)
	}
	return nil
}

// newInlineToken is a random, unguessable handle for one inline query: 16
// bytes as 22 base64url characters.
func newInlineToken() string {
	var bytes [16]byte
	_, _ = rand.Read(bytes[:])
	return base64.RawURLEncoding.EncodeToString(bytes[:])
}
