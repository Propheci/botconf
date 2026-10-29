package telegram

import (
	"github.com/gotd/td/tg"
	"github.com/propheci/botconf/go/settingspb"
)

// Keyboard turns rows of panel buttons into an inline keyboard whose callback
// data is prefix followed by each button's token. It returns nil for no rows.
func Keyboard(prefix string, rows []*settingspb.Row) *tg.ReplyInlineMarkup {
	var keyboard []tg.KeyboardButtonRow
	for _, row := range rows {
		var buttons []tg.KeyboardButtonClass
		for _, button := range row.GetButtons() {
			buttons = append(buttons, &tg.KeyboardButtonCallback{
				Text: button.GetLabel(),
				Data: []byte(prefix + button.GetToken()),
			})
		}
		if len(buttons) > 0 {
			keyboard = append(keyboard, tg.KeyboardButtonRow{Buttons: buttons})
		}
	}
	if len(keyboard) == 0 {
		return nil
	}
	return &tg.ReplyInlineMarkup{Rows: keyboard}
}

// markup is Keyboard as a ReplyMarkupClass: a nil interface for no rows, so
// that a request leaves the markup out instead of sending an empty one.
func markup(prefix string, rows []*settingspb.Row) tg.ReplyMarkupClass {
	if keyboard := Keyboard(prefix, rows); keyboard != nil {
		return keyboard
	}
	return nil
}
