package tui

import (
	"strings"

	"rsc.io/qr"
)

// qrQuietZone is the light border, in modules, a reader needs around a code.
const qrQuietZone = 2

// qrLines draws a QR code as lines of half blocks, two modules a character
// down and one across, light modules drawn (so phones read it on dark
// terminals); false when it cannot be encoded.
func qrLines(data string) ([]string, bool) {
	code, err := qr.Encode(data, qr.L)
	if err != nil {
		return nil, false
	}
	size := code.Size + 2*qrQuietZone
	dark := func(x, y int) bool {
		x, y = x-qrQuietZone, y-qrQuietZone
		return x >= 0 && y >= 0 && x < code.Size && y < code.Size && code.Black(x, y)
	}
	var lines []string
	for y := 0; y < size; y += 2 {
		var line strings.Builder
		for x := 0; x < size; x++ {
			// Past the last row is quiet zone: light.
			top, bottom := !dark(x, y), y+1 >= size || !dark(x, y+1)
			switch {
			case top && bottom:
				line.WriteRune('█')
			case top:
				line.WriteRune('▀')
			case bottom:
				line.WriteRune('▄')
			default:
				line.WriteRune(' ')
			}
		}
		lines = append(lines, line.String())
	}
	return lines, true
}
