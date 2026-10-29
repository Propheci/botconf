// Package telegram draws botconf settings panels on Telegram with gotd: for a
// bot session with inline keyboards (BotPanel), and for a user account
// through a helper bot in inline mode (InlinePanel).
//
// The renderers only map screens to messages and presses, replies and
// commands back to the settings client. They decide nothing about settings
// and never parse tokens: a token goes back exactly as the server sent it.
package telegram

import (
	"unicode/utf16"

	"github.com/gotd/td/telegram/message/entity"
	"github.com/gotd/td/tg"
	"github.com/propheci/botconf/go/settingspb"
)

// Entities turns styled text into a Telegram message and its entities, cut
// to at most maxChars UTF-16 code units (Telegram's measure); 0 means 4096.
func Entities(text *settingspb.Text, maxChars int) (string, []tg.MessageEntityClass) {
	if maxChars <= 0 {
		maxChars = MaxMessageChars
	}
	var builder entity.Builder
	left := maxChars
	for _, span := range text.GetSpans() {
		if left <= 0 {
			break
		}
		content, cut := truncateUTF16(span.GetText(), left)
		if cut {
			content = trimLastUnit(content) + "…"
		}
		left -= entity.ComputeLength(content)
		switch span.GetStyle() {
		case settingspb.Style_STYLE_BOLD:
			builder.Format(content, entity.Bold())
		case settingspb.Style_STYLE_ITALIC:
			builder.Format(content, entity.Italic())
		case settingspb.Style_STYLE_CODE:
			builder.Format(content, entity.Code())
		case settingspb.Style_STYLE_CODE_BLOCK:
			builder.Format(content, entity.Pre(""))
		default:
			builder.Plain(content)
		}
		if cut {
			break
		}
	}
	return builder.Raw()
}

// truncateUTF16 returns the longest prefix of s that fits in max UTF-16 code
// units, and whether s was cut.
func truncateUTF16(s string, max int) (string, bool) {
	units := 0
	for index, r := range s {
		n := utf16.RuneLen(r)
		if n < 0 {
			n = 1
		}
		if units+n > max {
			return s[:index], true
		}
		units += n
	}
	return s, false
}

// trimLastUnit drops the last rune of s, making room for the ellipsis.
func trimLastUnit(s string) string {
	runes := []rune(s)
	if len(runes) == 0 {
		return s
	}
	return string(runes[:len(runes)-1])
}
