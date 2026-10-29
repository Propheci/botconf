package discord

import (
	"strings"

	"github.com/propheci/botconf/go/settingspb"
)

// markdownEscaper escapes the characters Discord's markdown gives a meaning
// to, so that plain text shows as typed.
var markdownEscaper = strings.NewReplacer(
	`\`, `\\`, `*`, `\*`, `_`, `\_`, `~`, `\~`, "`", "\\`", `|`, `\|`,
	`>`, `\>`, `#`, `\#`, `[`, `\[`, `]`, `\]`, `(`, `\(`, `)`, `\)`,
)

// Markdown turns styled text into Discord markdown, cut to Discord's message
// length.
func Markdown(text *settingspb.Text) string {
	var out strings.Builder
	for _, span := range text.GetSpans() {
		content := span.GetText()
		switch span.GetStyle() {
		case settingspb.Style_STYLE_BOLD:
			out.WriteString("**" + markdownEscaper.Replace(content) + "**")
		case settingspb.Style_STYLE_ITALIC:
			out.WriteString("_" + markdownEscaper.Replace(content) + "_")
		case settingspb.Style_STYLE_CODE:
			if strings.Contains(content, "`") {
				out.WriteString("`` " + content + " ``")
			} else {
				out.WriteString("`" + content + "`")
			}
		case settingspb.Style_STYLE_CODE_BLOCK:
			out.WriteString("```\n" + strings.ReplaceAll(content, "```", "`​``") + "\n```")
		default:
			out.WriteString(markdownEscaper.Replace(content))
		}
	}
	return cut(out.String(), MaxMessageChars)
}

// plainText is styled text without its styles, e.g. for a placeholder.
func plainText(text *settingspb.Text) string {
	var out strings.Builder
	for _, span := range text.GetSpans() {
		out.WriteString(span.GetText())
	}
	return out.String()
}
