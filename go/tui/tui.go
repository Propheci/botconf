// Package tui is an offline setup screen for a program's config file and the
// env file its options name, over the program's describe.Description: every
// option with its description, edited without the program running. It is
// the Go side of the Rust `botconf-tui`.
//
// The file is written from the description, so it always reads like the
// generated example; keys the description does not know are kept as
// comments. An option naming an environment variable sets that variable's
// value in the env file (masked when secret; a token can be generated); the
// env file keeps its other lines. Before anything is written the host checks
// the candidate with its own loader.
package tui

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/pelletier/go-toml/v2"
	"github.com/propheci/botconf/go/describe"
)

// Candidate is what a save is checked against: the config file and the env
// file's variables, as they would be written.
type Candidate struct {
	Config string
	Env    map[string]string
}

// Setup is what Run edits.
type Setup struct {
	Description *describe.Description
	ConfigPath  string
	EnvPath     string
	// Validate is the host's check of a candidate: its loader, say. An error
	// is shown and nothing is written.
	Validate func(Candidate) error
	// Suggest gives the values of a domain (`domain:"…"` tags), suggested by
	// name as they are typed; nil for none.
	Suggest SuggestFunc
	// Actions are the host's actions, on the root screen.
	Actions []Action
}

// Run opens the setup screen until the user quits. A missing file starts
// empty; a config file that is not TOML is an error (fix it first).
func Run(setup Setup) error {
	values, err := readConfig(setup.ConfigPath)
	if err != nil {
		return err
	}
	envText, err := readOptional(setup.EnvPath)
	if err != nil {
		return err
	}
	a := newApp(setup.Description, values, ParseEnv(envText), filepath.Base(setup.EnvPath), GenerateToken)
	a.suggestFunc = setup.Suggest
	for _, action := range setup.Actions {
		a.actions = append(a.actions, actionInfo{title: action.Title, description: action.Description})
	}
	model := &model{app: a, setup: setup, savedEnv: envText}
	model.program = tea.NewProgram(model, tea.WithAltScreen())
	_, err = model.program.Run()
	return err
}

// model adapts the app to bubbletea.
type model struct {
	app           *app
	setup         Setup
	savedEnv      string
	width, height int
	program       *tea.Program
	// reply answers the running action's open question; cancel stops it.
	reply  chan<- *string
	cancel *atomic.Bool
}

// actionMsg is what a running action's goroutine tells the screen.
type actionMsg struct {
	event actionEvent
	// reply, with an ask, takes the answer (nil: stopped).
	reply chan<- *string
}

func (m *model) Init() tea.Cmd { return nil }

func (m *model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height
	case actionMsg:
		if msg.reply != nil {
			m.reply = msg.reply
		}
		m.app.actionEvent(msg.event)
	case tea.KeyMsg:
		for _, press := range presses(msg) {
			switch m.app.key(press.key, press.runes) {
			case quitEffect:
				return m, tea.Quit
			case saveEffect:
				notice, err := m.save()
				m.app.saved(notice, err)
			case runActionEffect:
				m.runAction(m.app.pendingAction)
			case answerEffect:
				m.answer(&m.app.answer)
			case cancelEffect:
				if m.cancel != nil {
					m.cancel.Store(true)
				}
				m.answer(nil)
			}
		}
	}
	return m, nil
}

// answer answers the running action's open question (nil: stopped).
func (m *model) answer(text *string) {
	if m.reply == nil {
		return
	}
	reply := m.reply
	m.reply = nil
	if text != nil {
		answer := *text
		text = &answer
	}
	reply <- text
}

// runAction starts action index on the saved files (saving the changes
// first), on its own goroutine.
func (m *model) runAction(index int) {
	if m.app.dirty {
		notice, err := m.save()
		if err != nil {
			m.app.notStarted(err)
			return
		}
		m.app.saved(notice, nil)
	}
	config, env := m.app.candidate()
	candidate := Candidate{Config: config, Env: ParseEnv(env).Vars()}
	m.cancel = &atomic.Bool{}
	session := &channelSession{program: m.program, cancelled: m.cancel}
	action := m.setup.Actions[index]
	m.app.started(index)
	go func() {
		notice, err := func() (notice string, err error) {
			defer func() {
				if recovered := recover(); recovered != nil {
					err = fmt.Errorf("The action failed unexpectedly: %v", recovered)
				}
			}()
			return action.Run(session, candidate)
		}()
		m.program.Send(actionMsg{event: actionEvent{done: true, notice: notice, err: err}})
	}()
}

// channelSession is a Session over the program's messages.
type channelSession struct {
	program   *tea.Program
	cancelled *atomic.Bool
}

func (s *channelSession) send(event actionEvent) { s.program.Send(actionMsg{event: event}) }

func (s *channelSession) Say(text string)      { s.send(actionEvent{say: text}) }
func (s *channelSession) QR(data string)       { s.send(actionEvent{qr: data}) }
func (s *channelSession) Progress(text string) { s.send(actionEvent{progress: text}) }
func (s *channelSession) Cancelled() bool      { return s.cancelled.Load() }

func (s *channelSession) Ask(prompt string) (string, bool) { return s.ask(prompt, false) }

func (s *channelSession) AskSecret(prompt string) (string, bool) { return s.ask(prompt, true) }

func (s *channelSession) ask(prompt string, secret bool) (string, bool) {
	if s.Cancelled() {
		return "", false
	}
	reply := make(chan *string, 1)
	s.program.Send(actionMsg{event: actionEvent{ask: &question{prompt: prompt, secret: secret}}, reply: reply})
	answer := <-reply
	if answer == nil {
		return "", false
	}
	return *answer, true
}

// keyPress is one key as the app takes it: its name and the text it types.
type keyPress struct {
	key   string
	runes []rune
}

// presses splits a key message into the app's keys: bubbletea reads
// characters typed in quick succession as one message ("jjg"), which are as
// many keys; a paste is text, typed into an input and ignored elsewhere.
func presses(msg tea.KeyMsg) []keyPress {
	switch {
	case msg.Paste:
		return []keyPress{{key: "", runes: msg.Runes}}
	case msg.Type == tea.KeyRunes && len(msg.Runes) > 1 && !msg.Alt:
		out := make([]keyPress, len(msg.Runes))
		for index, r := range msg.Runes {
			out[index] = keyPress{key: string(r), runes: []rune{r}}
		}
		return out
	case msg.Type == tea.KeyRunes || msg.Type == tea.KeySpace:
		return []keyPress{{key: msg.String(), runes: msg.Runes}}
	default:
		return []keyPress{{key: msg.String()}}
	}
}

func (m *model) View() string { return view(m.app, m.width, m.height) }

// save checks, then writes the config file and (when it changed) the env
// file.
func (m *model) save() (string, error) {
	config, env := m.app.candidate()
	if m.setup.Validate != nil {
		if err := m.setup.Validate(Candidate{Config: config, Env: ParseEnv(env).Vars()}); err != nil {
			return "", err
		}
	}
	if err := write(m.setup.ConfigPath, config, false); err != nil {
		return "", fmt.Errorf("writing %s: %w", m.setup.ConfigPath, err)
	}
	notice := "Saved " + m.setup.ConfigPath
	if env != m.savedEnv {
		if err := write(m.setup.EnvPath, env, true); err != nil {
			return "", fmt.Errorf("writing %s: %w", m.setup.EnvPath, err)
		}
		m.savedEnv = env
		notice += " and " + m.setup.EnvPath
	}
	return notice, nil
}

func readConfig(path string) (map[string]any, error) {
	text, err := readOptional(path)
	if err != nil {
		return nil, err
	}
	values := map[string]any{}
	if strings.TrimSpace(text) == "" {
		return values, nil
	}
	if err := toml.Unmarshal([]byte(text), &values); err != nil {
		return nil, fmt.Errorf("%s is not valid TOML: %w", path, err)
	}
	return values, nil
}

func readOptional(path string) (string, error) {
	text, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	return string(text), err
}

// write writes through a temporary file, so a crash leaves the old file
// whole. A new private file is readable by its owner only.
func write(path, text string, private bool) error {
	mode := os.FileMode(0o644)
	if private {
		mode = 0o600
	}
	if info, err := os.Stat(path); err == nil {
		mode = info.Mode().Perm()
	}
	temporary := path + ".tmp"
	if err := os.WriteFile(temporary, []byte(text), mode); err != nil {
		return err
	}
	if err := os.Chmod(temporary, mode); err != nil {
		return err
	}
	return os.Rename(temporary, path)
}
