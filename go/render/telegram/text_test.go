package telegram

import (
	"strings"
	"testing"

	"github.com/gotd/td/tg"
	"github.com/propheci/botconf/go/settingspb"
)

func TestEntitiesUseUTF16Offsets(t *testing.T) {
	text := &settingspb.Text{Spans: []*settingspb.Span{
		{Text: "🎛 "},
		{Style: settingspb.Style_STYLE_BOLD, Text: "Logging"},
		{Text: ": "},
		{Style: settingspb.Style_STYLE_CODE, Text: "info"},
	}}
	message, entities := Entities(text, 0)
	if message != "🎛 Logging: info" {
		t.Fatalf("message = %q", message)
	}
	if len(entities) != 2 {
		t.Fatalf("entities = %#v, want bold and code", entities)
	}
	// The emoji is two UTF-16 code units, so the bold span starts at 3.
	bold, ok := entities[0].(*tg.MessageEntityBold)
	if !ok || bold.Offset != 3 || bold.Length != 7 {
		t.Fatalf("bold = %#v, want offset 3 length 7", entities[0])
	}
	code, ok := entities[1].(*tg.MessageEntityCode)
	if !ok || code.Offset != 12 || code.Length != 4 {
		t.Fatalf("code = %#v, want offset 12 length 4", entities[1])
	}
}

func TestEntitiesAreCutToTheLimit(t *testing.T) {
	text := &settingspb.Text{Spans: []*settingspb.Span{
		{Text: "abc"},
		{Style: settingspb.Style_STYLE_BOLD, Text: strings.Repeat("x", 20)},
		{Text: "never shown"},
	}}
	message, _ := Entities(text, 10)
	if message != "abcxxxxxx…" {
		t.Fatalf("message = %q, want it cut to 10 code units with an ellipsis", message)
	}
}

func TestKeyboardPrefixesTokens(t *testing.T) {
	rows := []*settingspb.Row{
		{Buttons: []*settingspb.Button{{Label: "A", Token: "o:1"}, {Label: "B", Token: "~abc"}}},
		{Buttons: []*settingspb.Button{{Label: "C", Token: "x"}}},
	}
	keyboard := Keyboard("cfg:", rows)
	if keyboard == nil || len(keyboard.Rows) != 2 {
		t.Fatalf("keyboard = %#v, want two rows", keyboard)
	}
	if got := callbackData(keyboard); strings.Join(got, " ") != "cfg:o:1 cfg:~abc cfg:x" {
		t.Fatalf("callback data = %v", got)
	}
	if label := keyboard.Rows[0].Buttons[1].(*tg.KeyboardButtonCallback).Text; label != "B" {
		t.Fatalf("label = %q", label)
	}
}

func TestEmptyRowsGiveNoMarkup(t *testing.T) {
	if keyboard := Keyboard("cfg:", nil); keyboard != nil {
		t.Fatalf("keyboard = %#v, want nil", keyboard)
	}
	if keyboard := Keyboard("cfg:", []*settingspb.Row{{}}); keyboard != nil {
		t.Fatalf("keyboard of empty rows = %#v, want nil", keyboard)
	}
	if replyMarkup := markup("cfg:", nil); replyMarkup != nil {
		t.Fatalf("markup = %#v, want a nil interface", replyMarkup)
	}
}

func TestLimitsLeaveRoomForThePrefix(t *testing.T) {
	limits := Limits("cfg:")
	if limits.GetTokenBytes() != 60 || limits.GetTextChars() != 4096 {
		t.Fatalf("limits = %v, want 60-byte tokens and 4096 characters", limits)
	}
}
