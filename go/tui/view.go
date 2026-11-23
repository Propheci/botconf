package tui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// sideBySide is wide enough for the details beside the list.
const sideBySide = 90

var (
	bold     = lipgloss.NewStyle().Bold(true)
	dim      = lipgloss.NewStyle().Foreground(lipgloss.Color("8"))
	warn     = lipgloss.NewStyle().Foreground(lipgloss.Color("3"))
	bad      = lipgloss.NewStyle().Foreground(lipgloss.Color("1"))
	opens    = lipgloss.NewStyle().Foreground(lipgloss.Color("6"))
	selected = lipgloss.NewStyle().Reverse(true)
	box      = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).Padding(0, 1)
)

var helpLines = []string{
	"↑ ↓ (j k): move · esc (← h): back",
	"enter (→ l): edit an option, open a table or an item",
	"u: unset (the default applies) · a: add an item · d: delete an item",
	"e: set an environment variable's value · g: generate a token",
	"s: save (checked first) · q: quit",
}

// view draws the app in width × height cells.
func view(a *app, width, height int) string {
	if width <= 0 || height <= 0 {
		width, height = 80, 24
	}
	crumbs := make([]string, len(a.screens))
	for index, s := range a.screens {
		crumbs[index] = s.title
	}
	header := bold.Render(strings.Join(crumbs, " › "))
	if a.dirty {
		header += warn.Render("  (unsaved)")
	}
	footer := dim.Render(a.hints())
	if a.status != "" {
		footer = warn.Render(a.status)
	}
	bodyHeight := max(height-2, 3)

	var body string
	if popup := popupView(a, min(width-4, 64)); popup != "" {
		body = lipgloss.Place(width, bodyHeight, lipgloss.Center, lipgloss.Center, popup)
	} else if width >= sideBySide {
		half := width / 2
		body = lipgloss.JoinHorizontal(lipgloss.Top,
			listView(a, half-2, bodyHeight-2), detailsView(a, width-half-2, bodyHeight-2))
	} else {
		detailsHeight := min(9, bodyHeight/2)
		body = lipgloss.JoinVertical(lipgloss.Left,
			listView(a, width-2, bodyHeight-detailsHeight-2), detailsView(a, width-2, detailsHeight-2))
	}
	return lipgloss.JoinVertical(lipgloss.Left, truncate(header, width), body, truncate(footer, width))
}

func listView(a *app, width, height int) string {
	width, height = max(width, 1), max(height, 1)
	rows := a.rows()
	if len(rows) == 0 {
		text := "No options."
		if a.isCollection() {
			text = "Nothing here yet: a adds one."
		}
		return box.Width(width).Height(height).Render(text)
	}
	cursor := min(a.screen().cursor, len(rows)-1)
	titleWidth := 0
	for _, r := range rows {
		titleWidth = max(titleWidth, len([]rune(r.title())))
	}
	titleWidth = min(titleWidth, 32)
	// Keep the cursor in view.
	first := max(0, cursor-height+1)
	var lines []string
	for index := first; index < len(rows) && index < first+height; index++ {
		r := rows[index]
		value := a.shown(r)
		var text string
		switch value.kind {
		case shownSet:
			text = value.text
		case shownDefault:
			text = dim.Render(value.text + " (default)")
		case shownUnset:
			text = dim.Render("—")
		case shownMissing:
			text = bad.Render("required")
		case shownOpens:
			text = opens.Render(strings.TrimSpace(value.text + " ›"))
		}
		line := fmt.Sprintf("%-*s  %s", titleWidth, r.title(), text)
		line = truncate(line, width-2)
		if index == cursor {
			line = selected.Render(line)
		}
		lines = append(lines, line)
	}
	return box.Width(width).Height(height).Render(strings.Join(lines, "\n"))
}

func detailsView(a *app, width, height int) string {
	width, height = max(width, 1), max(height, 1)
	rows := a.rows()
	if len(rows) == 0 {
		return box.Width(width).Height(height).Render("")
	}
	title, paragraphs := a.details(rows[min(a.screen().cursor, len(rows)-1)])
	text := bold.Render(title) + "\n\n" + strings.Join(paragraphs, "\n\n")
	wrapped := lipgloss.NewStyle().Width(max(width-2, 1)).Render(text)
	lines := strings.Split(wrapped, "\n")
	if len(lines) > height {
		lines = lines[:height]
	}
	return box.Width(width).Height(height).Render(strings.Join(lines, "\n"))
}

// popupView is the open popup, or "".
func popupView(a *app, width int) string {
	width = max(width, 10)
	frame := func(title, text, bottom string) string {
		content := bold.Render(title) + "\n" + text
		if bottom != "" {
			content += "\n" + dim.Render(bottom)
		}
		return box.Width(width).Render(lipgloss.NewStyle().Width(width - 4).Render(content))
	}
	switch a.mode {
	case editing:
		shown := string(a.edit.input.text)
		if a.edit.secret {
			shown = strings.Repeat("•", len(a.edit.input.text))
		}
		runes := []rune(shown)
		cursor := a.edit.input.cursor
		// Keep the cursor in view on long input.
		skip := max(0, cursor-(width-6))
		line := string(runes[skip:cursor]) + selected.Render(" ")
		if cursor < len(runes) {
			line = string(runes[skip:cursor]) + selected.Render(string(runes[cursor])) + string(runes[cursor+1:])
		}
		return frame(a.edit.title, truncate(line, width-4)+"\n"+dim.Render(a.edit.note),
			"enter save · esc cancel · ctrl+u clear")
	case choosing:
		var lines []string
		for index, option := range a.choose.options {
			if index == a.choose.cursor {
				option = selected.Render(option)
			}
			lines = append(lines, option)
		}
		return frame(a.choose.title, strings.Join(lines, "\n"), "enter pick · esc cancel")
	case confirming:
		text := "Delete this item?"
		switch a.confirm.kind {
		case confirmQuit:
			text = "Quit without saving the changes?"
		case confirmRegenerate:
			text = a.confirm.envVar + " is already set. A new value must be given to the other side too. Replace it?"
		}
		return frame("Sure?", text, "y yes · any other key no")
	case messaging:
		return frame("Note", a.message, "any key")
	case helping:
		return frame("Keys", strings.Join(helpLines, "\n"), "any key")
	}
	return ""
}

// truncate cuts a line to width cells.
func truncate(line string, width int) string {
	if width <= 0 {
		return ""
	}
	return lipgloss.NewStyle().MaxWidth(width).Render(line)
}
