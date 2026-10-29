package telegram

import "github.com/propheci/botconf/go/settingspb"

const (
	// MaxCallbackBytes is the most callback data a Telegram button carries.
	MaxCallbackBytes = 64
	// MaxMessageChars is the longest Telegram message, in UTF-16 code units.
	MaxMessageChars = 4096
	// maxPerRow is how many buttons the engine puts in a row.
	maxPerRow = 8
)

// Limits are what a panel shown on Telegram with callback data prefix can
// hold: the token is what is left of the 64 bytes after the prefix.
func Limits(prefix string) *settingspb.Limits {
	tokenBytes := MaxCallbackBytes - len(prefix)
	if tokenBytes < 0 {
		tokenBytes = 0
	}
	return &settingspb.Limits{
		TokenBytes: uint32(tokenBytes),
		TextChars:  MaxMessageChars,
		MaxRows:    0,
		MaxPerRow:  maxPerRow,
	}
}
