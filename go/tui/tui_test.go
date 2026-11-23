package tui

import (
	"reflect"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/pelletier/go-toml/v2"
	"github.com/propheci/botconf/go/describe"
)

type sample struct {
	Listen    string            `toml:"listen" doc:"Where to listen." required:"true"`
	Workers   uint8             `toml:"workers" default:"0" jsonschema:"minimum=0,maximum=10"`
	Verbose   bool              `toml:"verbose"`
	Level     string            `toml:"level" default:"info" jsonschema:"enum=info,enum=debug"`
	TokenEnv  string            `toml:"token_env" doc:"The core's token." env:"token" hint:"APP_TOKEN"`
	Connector []connector       `toml:"connectors"`
	Rules     map[string]rule   `toml:"rules"`
	Limits    map[string]uint32 `toml:"limits"`
}

type connector struct {
	ID    string   `toml:"id" required:"true"`
	Names []string `toml:"names"`
}

type rule struct {
	Source string `toml:"source"`
}

func fakeToken() (string, error) { return "generated", nil }

func newSample(t *testing.T, values map[string]any, env string) *app {
	t.Helper()
	description, err := describe.Of("Sample", "", sample{})
	if err != nil {
		t.Fatal(err)
	}
	return newApp(description, values, ParseEnv(env), ".env.local", fakeToken)
}

func press(a *app, keys ...string) effect {
	var last effect
	for _, key := range keys {
		last = a.key(key, nil)
	}
	return last
}

func typing(a *app, text string) {
	for _, r := range text {
		a.key(string(r), []rune{r})
	}
}

func selectRow(t *testing.T, a *app, title string) {
	t.Helper()
	for index, r := range a.rows() {
		if r.title() == title {
			a.screen().cursor = index
			return
		}
	}
	t.Fatalf("no row %s", title)
}

func titles(a *app) []string {
	var out []string
	for _, r := range a.rows() {
		out = append(out, r.title())
	}
	return out
}

func TestAnOptionIsEditedInPlace(t *testing.T) {
	a := newSample(t, nil, "")
	if got := a.shown(a.rows()[0]); got.kind != shownMissing {
		t.Fatalf("listen shows %+v", got)
	}
	press(a, "enter")
	typing(a, "127.0.0.1:9")
	press(a, "enter")
	if a.mode != browsing || a.values["listen"] != "127.0.0.1:9" || !a.dirty {
		t.Fatalf("mode %v values %v", a.mode, a.values)
	}
	press(a, "enter")
	if string(a.edit.input.text) != "127.0.0.1:9" {
		t.Fatalf("editing starts from %q", string(a.edit.input.text))
	}
	press(a, "ctrl+u", "enter")
	if len(a.values) != 0 {
		t.Fatalf("an empty input unsets: %v", a.values)
	}
}

func TestABadValueKeepsTheEditorOpen(t *testing.T) {
	a := newSample(t, nil, "")
	selectRow(t, a, "Workers")
	if got := a.shown(a.rows()[1]); got != (shown{shownDefault, "0"}) {
		t.Fatalf("workers shows %+v", got)
	}
	press(a, "enter")
	typing(a, "11")
	press(a, "enter")
	if a.mode != editing || a.edit.note != "From 0 to 10." {
		t.Fatalf("mode %v note %q", a.mode, a.edit.note)
	}
	press(a, "backspace", "enter")
	if a.values["workers"] != int64(1) {
		t.Fatalf("values %v", a.values)
	}
}

func TestBooleansToggleChoicesArePickedAndUUnsets(t *testing.T) {
	a := newSample(t, nil, "")
	selectRow(t, a, "Verbose")
	press(a, "enter")
	if a.values["verbose"] != true {
		t.Fatal("not toggled")
	}
	selectRow(t, a, "Level")
	press(a, "enter")
	if a.mode != choosing || a.choose.cursor != 0 {
		t.Fatalf("the default is picked: %v %d", a.mode, a.choose.cursor)
	}
	press(a, "down", "enter")
	if a.values["level"] != "debug" {
		t.Fatalf("values %v", a.values)
	}
	press(a, "u")
	selectRow(t, a, "Verbose")
	press(a, "u")
	if len(a.values) != 0 {
		t.Fatalf("values %v", a.values)
	}
}

func TestListItemsAreAddedOpenedAndDeleted(t *testing.T) {
	a := newSample(t, nil, "")
	selectRow(t, a, "Connectors")
	press(a, "enter")
	if !a.isCollection() || len(a.rows()) != 0 {
		t.Fatal("an empty list")
	}
	press(a, "a")
	if got := titles(a); !reflect.DeepEqual(got, []string{"Id", "Names"}) {
		t.Fatalf("titles %v", got)
	}
	press(a, "enter")
	typing(a, "tg")
	press(a, "enter")
	selectRow(t, a, "Names")
	press(a, "enter")
	typing(a, "a, b")
	press(a, "enter", "esc")
	if got := titles(a); !reflect.DeepEqual(got, []string{"1. tg"}) {
		t.Fatalf("titles %v", got)
	}
	want := []any{map[string]any{"id": "tg", "names": []any{"a", "b"}}}
	if !reflect.DeepEqual(a.values["connectors"], want) {
		t.Fatalf("values %v", a.values)
	}
	press(a, "d")
	if a.mode != confirming || a.confirm.kind != confirmDelete {
		t.Fatal("asks first")
	}
	press(a, "y")
	if items := a.values["connectors"].([]any); len(items) != 0 {
		t.Fatalf("values %v", a.values)
	}
}

func TestMapEntriesAreNamed(t *testing.T) {
	a := newSample(t, nil, "")
	selectRow(t, a, "Rules")
	press(a, "enter", "a")
	typing(a, "r1")
	press(a, "enter")
	if got := titles(a); !reflect.DeepEqual(got, []string{"Source"}) {
		t.Fatalf("the new entry opens: %v", got)
	}
	press(a, "esc", "a")
	typing(a, "r1")
	press(a, "enter")
	if a.mode != editing || a.edit.note != "r1 already exists." {
		t.Fatalf("mode %v note %q", a.mode, a.edit.note)
	}
	// A map of plain values is edited in place, as JSON.
	press(a, "esc", "esc")
	selectRow(t, a, "Limits")
	press(a, "enter")
	typing(a, `{"x": 0}`)
	press(a, "enter")
	if !reflect.DeepEqual(a.values["limits"], map[string]any{"x": int64(0)}) {
		t.Fatalf("values %v", a.values)
	}
}

func TestATokenIsGeneratedIntoTheEnvFile(t *testing.T) {
	a := newSample(t, nil, "# mine\nOTHER=1\n")
	selectRow(t, a, "Token env")
	press(a, "g")
	if a.values["token_env"] != "APP_TOKEN" {
		t.Fatalf("the hinted name is written: %v", a.values)
	}
	if _, env := a.candidate(); env != "# mine\nOTHER=1\nAPP_TOKEN=generated\n" {
		t.Fatalf("env %q", env)
	}
	press(a, "g")
	if a.mode != confirming || a.confirm.kind != confirmRegenerate {
		t.Fatal("replacing asks first")
	}
	press(a, "n", "e")
	if a.mode != editing || !a.edit.secret {
		t.Fatal("e edits the value, masked")
	}
	press(a, "ctrl+u")
	typing(a, "mine")
	press(a, "enter")
	if value, _ := a.env.Get("APP_TOKEN"); value != "mine" {
		t.Fatalf("value %q", value)
	}
	_, details := a.details(a.rows()[4])
	if !strings.Contains(details[len(details)-1], "APP_TOKEN is set (hidden)") {
		t.Fatalf("details %v", details)
	}
}

func TestQuittingWithChangesAsks(t *testing.T) {
	a := newSample(t, nil, "")
	if press(a, "q") != quitEffect {
		t.Fatal("clean: quits")
	}
	selectRow(t, a, "Verbose")
	press(a, "enter")
	if press(a, "q") != noEffect || press(a, "y") != quitEffect {
		t.Fatal("dirty: asks")
	}
	if press(a, "s") != saveEffect {
		t.Fatal("s saves")
	}
	a.saved("", errString("listen: missing"))
	if a.mode != messaging || a.message != "Not saved: listen: missing" {
		t.Fatalf("message %q", a.message)
	}
	press(a, "x")
	a.saved("Saved", nil)
	if a.dirty || a.status != "Saved" {
		t.Fatal("saved")
	}
}

type errString string

func (e errString) Error() string { return string(e) }

func TestTheCandidateReadsBackAsTheType(t *testing.T) {
	a := newSample(t, map[string]any{
		"listen":     "0.0.0.0:1",
		"connectors": []any{map[string]any{"id": "tg"}},
		"rules":      map[string]any{"r": map[string]any{"source": "x"}},
		"stray":      true,
	}, "")
	config, _ := a.candidate()
	var parsed sample
	if err := toml.Unmarshal([]byte(config), &parsed); err != nil {
		t.Fatalf("%v\n%s", err, config)
	}
	if len(parsed.Connector) != 1 || parsed.Rules["r"].Source != "x" {
		t.Fatalf("parsed %+v", parsed)
	}
	if !strings.Contains(config, "# stray = true") {
		t.Fatal(config)
	}
}

func TestEveryModeDraws(t *testing.T) {
	reach := map[string][]string{
		"browse":     nil,
		"edit":       {"enter"},
		"choose":     {"down", "down", "down", "enter"},
		"help":       {"?"},
		"collection": {"end", "up", "up", "enter"},
	}
	for name, keys := range reach {
		for _, size := range [][2]int{{120, 30}, {60, 12}, {10, 3}, {1, 1}, {0, 0}} {
			a := newSample(t, map[string]any{"connectors": []any{map[string]any{"id": "tg"}}}, "")
			press(a, keys...)
			out := view(a, size[0], size[1])
			if size[0] == 120 && !strings.Contains(out, "Sample") {
				t.Errorf("%s: no title in\n%s", name, out)
			}
		}
	}
}

func TestEnvFile(t *testing.T) {
	const file = "# The core's tokens\nexport WUPHF_TOKEN=abc # set by hand\nQUOTED=\"a \\\"b\\\" c\"\nSINGLE='x y'\n\nnot a pair\n"
	env := ParseEnv(file)
	for key, want := range map[string]string{"WUPHF_TOKEN": "abc", "QUOTED": `a "b" c`, "SINGLE": "x y"} {
		if got, _ := env.Get(key); got != want {
			t.Errorf("%s = %q, want %q", key, got, want)
		}
	}
	if env.Render() != file {
		t.Fatalf("unchanged, it writes back as read:\n%s", env.Render())
	}
	env.Set("SINGLE", "x y")
	env.Set("WUPHF_TOKEN", "new value")
	env.Set("ADDED", "1")
	rendered := env.Render()
	if !strings.Contains(rendered, "\nWUPHF_TOKEN=\"new value\"\nQUOTED=") ||
		!strings.HasSuffix(rendered, "not a pair\nADDED=1\n") || !strings.Contains(rendered, "SINGLE='x y'") {
		t.Fatalf("rendered:\n%s", rendered)
	}
	for _, value := range []string{"", "a b", `q"uote`, `back\slash`, "$HOME", "multi\nline"} {
		env := ParseEnv("")
		env.Set("V", value)
		if got, _ := ParseEnv(env.Render()).Get("V"); got != value {
			t.Errorf("%q read back as %q", value, got)
		}
	}
	first, _ := GenerateToken()
	second, _ := GenerateToken()
	if len(first) != 64 || first == second {
		t.Fatalf("tokens %q %q", first, second)
	}
}

func TestQuickKeysAndPastesAreSplit(t *testing.T) {
	quick := presses(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("jjg")})
	if len(quick) != 3 || quick[2].key != "g" || string(quick[2].runes) != "g" {
		t.Fatalf("quick keys %+v", quick)
	}
	paste := presses(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("a b"), Paste: true})
	if len(paste) != 1 || paste[0].key != "" || string(paste[0].runes) != "a b" {
		t.Fatalf("paste %+v", paste)
	}
	if enter := presses(tea.KeyMsg{Type: tea.KeyEnter}); enter[0].key != "enter" || enter[0].runes != nil {
		t.Fatalf("enter %+v", enter)
	}
	// A paste types into an input; elsewhere it does nothing.
	a := newSample(t, nil, "")
	a.key("", []rune("ignored"))
	if a.mode != browsing || len(a.values) != 0 {
		t.Fatal("a paste while browsing acted")
	}
	press(a, "enter")
	a.key("", []rune("127.0.0.1:9"))
	press(a, "enter")
	if a.values["listen"] != "127.0.0.1:9" {
		t.Fatalf("values %v", a.values)
	}
}
