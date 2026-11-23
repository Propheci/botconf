package tui

import (
	"errors"
	"fmt"
	"reflect"
	"strings"
	"testing"

	"github.com/propheci/botconf/go/describe"
)

type chats struct {
	Home   string   `toml:"home" doc:"The home chat." domain:"t.chat"`
	Notify []string `toml:"notify" doc:"Where notices go." domain:"t.chat"`
}

var known = [][2]string{{"chat:1", "Family"}, {"chat:2", "Work"}, {"chat:3", "Football"}}

func knownChats(query Query) ([]Suggestion, error) {
	text := strings.ToLower(query.Text)
	var out []Suggestion
	for _, chat := range known {
		if len(out) < query.Limit && (strings.Contains(chat[0], text) || strings.Contains(strings.ToLower(chat[1]), text)) {
			out = append(out, Suggestion{Value: chat[0], Label: chat[1] + " (" + chat[0] + ")"})
		}
	}
	return out, nil
}

func newChats(t *testing.T, values map[string]any, suggest SuggestFunc) *app {
	t.Helper()
	description, err := describe.Of("Chats", "", chats{})
	if err != nil {
		t.Fatal(err)
	}
	a := newApp(description, values, ParseEnv(""), ".env", fakeToken)
	a.suggestFunc = suggest
	return a
}

func withAction(a *app) *app {
	a.actions = []actionInfo{{title: "Link account", description: "Log in."}}
	return a
}

func TestADomainValueIsPickedBySearch(t *testing.T) {
	a := newChats(t, nil, knownChats)
	press(a, "enter")
	if len(a.edit.suggestions) != 3 {
		t.Fatalf("everything, before typing: %v", a.edit.suggestions)
	}
	typing(a, "fo")
	if len(a.edit.suggestions) != 1 || a.edit.suggestions[0].Label != "Football (chat:3)" {
		t.Fatalf("suggestions %v", a.edit.suggestions)
	}
	press(a, "down")
	if a.edit.highlight != 0 {
		t.Fatalf("highlight %d", a.edit.highlight)
	}
	press(a, "enter")
	if !reflect.DeepEqual(a.values, map[string]any{"home": "chat:3"}) {
		t.Fatalf("values %v", a.values)
	}
	// Without a highlight, Enter takes what is typed.
	press(a, "enter", "ctrl+u")
	typing(a, "chat:9")
	press(a, "enter")
	if !reflect.DeepEqual(a.values, map[string]any{"home": "chat:9"}) {
		t.Fatalf("values %v", a.values)
	}
	if got := a.shown(a.rows()[0]); got.text != "chat:9" {
		t.Fatalf("an unknown value reads as itself: %+v", got)
	}
}

func TestTheHostSaysWhyNothingIsSuggested(t *testing.T) {
	a := newChats(t, nil, func(Query) ([]Suggestion, error) { return nil, errors.New("no chats known yet") })
	press(a, "enter")
	if len(a.edit.suggestions) != 0 || a.edit.unsuggested != "no chats known yet" {
		t.Fatalf("edit %+v", a.edit)
	}
	if out := view(a, 120, 30); !strings.Contains(out, "no chats known yet") {
		t.Fatalf("the reason is shown:\n%s", out)
	}
}

func TestADomainListAddsAndEditsItsItemsBySearch(t *testing.T) {
	a := newChats(t, map[string]any{"notify": []any{"chat:2"}}, knownChats)
	selectRow(t, a, "Notify")
	press(a, "enter")
	if !a.isCollection() || !reflect.DeepEqual(titles(a), []string{"Work (chat:2)"}) {
		t.Fatalf("items read by their labels: %v", titles(a))
	}
	press(a, "a")
	typing(a, "fam")
	press(a, "down", "enter")
	if !reflect.DeepEqual(a.values["notify"], []any{"chat:2", "chat:1"}) {
		t.Fatalf("values %v", a.values)
	}
	if !reflect.DeepEqual(titles(a), []string{"Work (chat:2)", "Family (chat:1)"}) {
		t.Fatalf("titles %v", titles(a))
	}
	// An item is edited by the same search.
	press(a, "home", "enter")
	if string(a.edit.input.text) != "chat:2" {
		t.Fatalf("editing %q", string(a.edit.input.text))
	}
	press(a, "ctrl+u")
	typing(a, "foot")
	press(a, "down", "enter")
	if !reflect.DeepEqual(a.values["notify"], []any{"chat:3", "chat:1"}) {
		t.Fatalf("values %v", a.values)
	}
	press(a, "d", "y")
	if !reflect.DeepEqual(a.values["notify"], []any{"chat:1"}) {
		t.Fatalf("values %v", a.values)
	}
}

func TestAnActionRunsFromTheRootScreen(t *testing.T) {
	a := withAction(newChats(t, nil, knownChats))
	press(a, "end")
	if got := a.rows()[2].title(); got != "▶ Link account" {
		t.Fatalf("title %q", got)
	}
	if press(a, "enter") != runActionEffect || a.pendingAction != 0 {
		t.Fatal("enter runs it")
	}
	a.started(0)
	a.actionEvent(actionEvent{say: "Scan this:"})
	a.actionEvent(actionEvent{qr: "link"})
	a.actionEvent(actionEvent{ask: &question{prompt: "Code:"}})
	typing(a, "123")
	if press(a, "enter") != answerEffect || a.answer != "123" {
		t.Fatalf("answer %q", a.answer)
	}
	a.actionEvent(actionEvent{ask: &question{prompt: "Password:", secret: true}})
	typing(a, "pw")
	if press(a, "enter") != answerEffect || a.answer != "pw" {
		t.Fatalf("answer %q", a.answer)
	}
	if want := []string{"Scan this:", "Code: 123", "Password: ••••••"}; !reflect.DeepEqual(a.running.lines, want) {
		t.Fatalf("lines %v", a.running.lines)
	}
	if a.running.qr != "link" {
		t.Fatalf("qr %q", a.running.qr)
	}
	a.actionEvent(actionEvent{done: true, notice: "Linked."})
	if press(a, "x") != noEffect || a.mode != browsing || a.status != "Linked." {
		t.Fatalf("mode %v status %q", a.mode, a.status)
	}
}

func TestAnActionIsStoppedWithEscAndItsFailureShown(t *testing.T) {
	a := withAction(newChats(t, nil, knownChats))
	a.started(0)
	a.actionEvent(actionEvent{ask: &question{prompt: "Code:"}})
	if press(a, "esc") != cancelEffect || a.running.question != nil || !a.running.cancelling {
		t.Fatal("esc stops it")
	}
	// Ctrl-c stops it too, rather than quitting.
	if press(a, "ctrl+c") != cancelEffect {
		t.Fatal("ctrl+c stops it")
	}
	a.actionEvent(actionEvent{done: true, err: errors.New("Stopped.")})
	press(a, "x")
	if a.mode != messaging || a.message != "Stopped." {
		t.Fatalf("mode %v message %q", a.mode, a.message)
	}
	// What a finished action says is no longer shown.
	a.actionEvent(actionEvent{say: "late"})
	if a.mode != messaging {
		t.Fatal("a late event reopened the action")
	}
}

func TestARefusedSaveDoesNotStartTheAction(t *testing.T) {
	a := withAction(newChats(t, nil, knownChats))
	a.notStarted(errors.New("home: missing"))
	if a.mode != messaging || a.message != "Not run: home: missing" {
		t.Fatalf("mode %v message %q", a.mode, a.message)
	}
}

const linkCode = "2@abcdefghijklmnopqrstuvwxyz,0123456789,ABCDEF"

func TestARunningActionDrawsWithItsQRCode(t *testing.T) {
	a := withAction(newChats(t, nil, knownChats))
	a.started(0)
	a.actionEvent(actionEvent{qr: linkCode})
	a.actionEvent(actionEvent{ask: &question{prompt: "Code:", secret: true}})
	typing(a, "pw")
	for _, size := range [][2]int{{120, 40}, {40, 10}, {1, 1}, {0, 0}} {
		out := view(a, size[0], size[1])
		if size[0] == 120 && (!strings.Contains(out, "Link account") || !strings.Contains(out, "••") || strings.Contains(out, "pw")) {
			t.Errorf("drawn:\n%s", out)
		}
	}
	lines, ok := qrLines(linkCode)
	if !ok {
		t.Fatal("not encoded")
	}
	// Two modules a character down, one across.
	width := len([]rune(lines[0]))
	if len(lines) >= width {
		t.Fatalf("%d lines of %d", len(lines), width)
	}
	for _, line := range lines {
		if len([]rune(line)) != width {
			t.Fatalf("ragged: %q", line)
		}
	}
}

func TestAQRCodeThatDoesNotFitSaysWhatItNeeds(t *testing.T) {
	a := withAction(newChats(t, nil, knownChats))
	a.started(0)
	a.actionEvent(actionEvent{qr: linkCode})
	if out := view(a, 80, 16); !strings.Contains(out, "The QR code needs a terminal of at least") {
		t.Fatalf("drawn:\n%s", out)
	}
	if out := view(a, 120, 40); strings.Contains(out, "The QR code needs") {
		t.Fatalf("it fits:\n%s", out)
	}
}

func TestManyMatchesScrollWithTheHighlight(t *testing.T) {
	a := newChats(t, nil, func(query Query) ([]Suggestion, error) {
		var out []Suggestion
		for n := 1; n <= 30 && len(out) < query.Limit; n++ {
			out = append(out, Suggestion{Value: fmt.Sprintf("chat:%d", n), Label: fmt.Sprintf("Chat number %02d", n)})
		}
		return out, nil
	})
	press(a, "enter")
	if len(a.edit.suggestions) != 30 {
		t.Fatalf("more than a window's worth: %d", len(a.edit.suggestions))
	}
	for range 15 {
		press(a, "down")
	}
	if a.edit.highlight != 14 {
		t.Fatalf("highlight %d", a.edit.highlight)
	}
	drawn := view(a, 100, 40)
	if !strings.Contains(drawn, "Chat number 15") || strings.Contains(drawn, "Chat number 01") || !strings.Contains(drawn, "15 of 30") {
		t.Fatalf("the window follows the highlight:\n%s", drawn)
	}
	// A short terminal shows fewer, the highlight still among them.
	if short := view(a, 100, 12); !strings.Contains(short, "Chat number 15") {
		t.Fatalf("short:\n%s", short)
	}
	press(a, "pgdown")
	if a.edit.highlight != 22 {
		t.Fatalf("pgdown: %d", a.edit.highlight)
	}
	press(a, "pgdown", "pgdown")
	if a.edit.highlight != 29 {
		t.Fatalf("the last: %d", a.edit.highlight)
	}
	press(a, "pgup", "enter")
	if a.values["home"] != "chat:22" {
		t.Fatalf("values %v", a.values)
	}
}

func TestTheProgramsOwnVariablesAreSetOnTheEnvironmentScreen(t *testing.T) {
	description, err := describe.Of("Chats", "", chats{})
	if err != nil {
		t.Fatal(err)
	}
	description.WithEnv(
		describe.Variable{Name: "APP_ID", Title: "App id", Description: "From my.telegram.org.", Required: true},
		describe.Variable{Name: "APP_HASH", Secret: true},
	)
	a := newApp(description, nil, ParseEnv("# mine\nAPP_HASH=abc\n"), ".env", fakeToken)
	if got := titles(a); !reflect.DeepEqual(got, []string{"Home", "Notify", "Environment"}) {
		t.Fatalf("root %v", got)
	}
	if got := a.shown(a.rows()[2]); got.text != "1 of 2 set" {
		t.Fatalf("environment row %+v", got)
	}
	selectRow(t, a, "Environment")
	press(a, "enter")
	if got := titles(a); !reflect.DeepEqual(got, []string{"App id", "APP_HASH"}) {
		t.Fatalf("variables %v", got)
	}
	if a.shown(a.rows()[0]).kind != shownMissing || a.shown(a.rows()[1]).text != "•••••• (set)" {
		t.Fatalf("shown %+v %+v", a.shown(a.rows()[0]), a.shown(a.rows()[1]))
	}
	if title, details := a.details(a.rows()[0]); title != "App id" || !strings.Contains(details[1], "Variable: APP_ID, set in .env") {
		t.Fatalf("details %q %v", title, details)
	}
	press(a, "enter")
	typing(a, "12345")
	press(a, "enter")
	press(a, "down", "enter")
	if !a.edit.secret {
		t.Fatal("a secret variable is masked")
	}
	press(a, "esc", "esc")
	if _, env := a.candidate(); env != "# mine\nAPP_HASH=abc\nAPP_ID=12345\n" || !a.dirty {
		t.Fatalf("env %q", env)
	}
	if out := view(a, 120, 30); !strings.Contains(out, "Environment") {
		t.Fatalf("drawn:\n%s", out)
	}
}
