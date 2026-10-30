package telegram

import (
	"crypto/rand"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"

	"github.com/gotd/td/tg"
	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settingspb"
)

const (
	// UserDomain is the reference domain of Telegram users, as actors and in
	// pickers.
	UserDomain = "telegram.user"
	// ChatDomain is the reference domain of Telegram groups and channels, as
	// Bot API chat ids (e.g. -1001234567890).
	ChatDomain = "telegram.chat"

	// cancelCommand gives up on a question.
	cancelCommand = "/cancel"
	// cancelButton gives up on a question asked with Telegram's picker,
	// whose reply keyboard cannot carry ‹ Back.
	cancelButton = "✖️ Cancel"
)

// actorFor is the settings actor for a Telegram user.
func actorFor(user int64) *settings.Actor {
	return &settings.Actor{Domain: UserDomain, ID: strconv.FormatInt(user, 10)}
}

// randomID is a message's random id, which Telegram uses to drop duplicates.
func randomID() int64 {
	var bytes [8]byte
	_, _ = rand.Read(bytes[:])
	return int64(binary.LittleEndian.Uint64(bytes[:]))
}

// inputPeerKey names a conversation, to key pending questions.
func inputPeerKey(peer tg.InputPeerClass) string {
	switch peer := peer.(type) {
	case *tg.InputPeerUser:
		return "u" + strconv.FormatInt(peer.UserID, 10)
	case *tg.InputPeerChat:
		return "c" + strconv.FormatInt(peer.ChatID, 10)
	case *tg.InputPeerChannel:
		return "ch" + strconv.FormatInt(peer.ChannelID, 10)
	case *tg.InputPeerSelf:
		return "self"
	default:
		return fmt.Sprintf("%T", peer)
	}
}

// ZeroHashPeer turns a peer into an input peer with no access hash, which a
// bot may use for the users and chats it has met. It is BotPanel's default
// way to address the chat a button was pressed in.
func ZeroHashPeer(peer tg.PeerClass) (tg.InputPeerClass, error) {
	switch peer := peer.(type) {
	case *tg.PeerUser:
		return &tg.InputPeerUser{UserID: peer.UserID}, nil
	case *tg.PeerChat:
		return &tg.InputPeerChat{ChatID: peer.ChatID}, nil
	case *tg.PeerChannel:
		return &tg.InputPeerChannel{ChannelID: peer.ChannelID}, nil
	default:
		return nil, fmt.Errorf("unsupported peer %T", peer)
	}
}

// senderOf is the user who sent msg: its sender, or the other side of a
// private chat.
func senderOf(msg *tg.Message) (int64, bool) {
	if from, ok := msg.FromID.(*tg.PeerUser); ok {
		return from.UserID, true
	}
	if peer, ok := msg.PeerID.(*tg.PeerUser); ok && !msg.Out {
		return peer.UserID, true
	}
	return 0, false
}

// isCancel says whether text gives up on a question.
func isCancel(text string) bool {
	text = strings.TrimSpace(text)
	return text == cancelCommand || strings.HasPrefix(text, cancelCommand+"@") || text == cancelButton
}

// askText is a question with how to give up appended: ‹ Back when the
// question shows its buttons, else /cancel.
func askText(ask *settingspb.Ask, buttons bool) *settingspb.Text {
	stop := "\n\nSend " + cancelCommand + " to cancel."
	if buttons && ask.GetBack() != nil {
		stop = "\n\nTap ‹ Back or send " + cancelCommand + " to cancel."
	}
	spans := append([]*settingspb.Span(nil), ask.GetText().GetSpans()...)
	spans = append(spans, &settingspb.Span{Style: settingspb.Style_STYLE_ITALIC, Text: stop})
	return &settingspb.Text{Spans: spans}
}

// askRows are a question's buttons: its suggestions, then ‹ Back.
func askRows(ask *settingspb.Ask) []*settingspb.Row {
	rows := append([]*settingspb.Row(nil), ask.GetRows()...)
	if back := ask.GetBack(); back != nil {
		rows = append(rows, &settingspb.Row{Buttons: []*settingspb.Button{back}})
	}
	return rows
}

// withNotice puts a notice above a screen's text, for places that cannot
// show a toast.
func withNotice(screen *settingspb.Screen, notice string) *settingspb.Screen {
	if notice == "" || screen == nil {
		return screen
	}
	spans := append([]*settingspb.Span{{Style: settingspb.Style_STYLE_ITALIC, Text: notice + "\n\n"}}, screen.GetText().GetSpans()...)
	return &settingspb.Screen{Text: &settingspb.Text{Spans: spans}, Rows: screen.GetRows()}
}

// plain is unstyled text.
func plain(text string) *settingspb.Text {
	return &settingspb.Text{Spans: []*settingspb.Span{{Text: text}}}
}

// botAPIChatID is the Bot API id of a picked group or channel, the
// telegram.chat domain's form.
func botAPIChatID(peer tg.PeerClass) (json.RawMessage, string, bool) {
	switch peer := peer.(type) {
	case *tg.PeerUser:
		return json.RawMessage(strconv.FormatInt(peer.UserID, 10)), UserDomain, true
	case *tg.PeerChat:
		return json.RawMessage(strconv.FormatInt(-peer.ChatID, 10)), ChatDomain, true
	case *tg.PeerChannel:
		return json.RawMessage(strconv.FormatInt(-1_000_000_000_000-peer.ChannelID, 10)), ChatDomain, true
	default:
		return nil, "", false
	}
}
