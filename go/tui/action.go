package tui

import "strings"

// Action is one of the host's actions, listed on the root screen. Run is the
// host's code (a login, a fetch), on the saved files, talking to the screen
// through the session; it gives a notice, or the reason it failed.
type Action struct {
	Title, Description string
	Run                func(Session, Candidate) (string, error)
}

// Session is how a running action talks to the screen. The action runs on
// its own goroutine; Ask blocks until the answer.
type Session interface {
	// Say adds a line to the action's log.
	Say(text string)
	// QR shows a QR code to scan, replacing the last one.
	QR(data string)
	// Progress says where it has got to, replacing the last progress.
	Progress(text string)
	// Ask is the answer to a question, or false when the user stopped the
	// action.
	Ask(question string) (string, bool)
	// AskSecret is Ask, the answer masked.
	AskSecret(question string) (string, bool)
	// Cancelled reports that the user asked to stop: return soon.
	Cancelled() bool
}

// actionLines is the most lines a running action's screen keeps.
const actionLines = 200

// running is a running (or finished) action's screen.
type running struct {
	title    string
	lines    []string
	progress string
	qr       string
	question *question
	// cancelling: asked to stop, not stopped yet.
	cancelling bool
	done       bool
	notice     string
	err        error
}

type question struct {
	prompt string
	secret bool
	input  input
}

// actionEvent is what a running action tells the screen; exactly one field is
// set (done with notice or err).
type actionEvent struct {
	say, qr, progress string
	ask               *question
	done              bool
	notice            string
	err               error
}

// started opens action index's screen.
func (a *app) started(index int) {
	a.mode = runningMode
	a.running = running{title: a.actions[index].title}
}

// notStarted: the action's save was refused.
func (a *app) notStarted(err error) {
	a.mode, a.message = messaging, "Not run: "+err.Error()
}

// actionEvent records what the running action said.
func (a *app) actionEvent(event actionEvent) {
	if a.mode != runningMode {
		return
	}
	r := &a.running
	switch {
	case event.say != "":
		r.lines = append(r.lines, event.say)
		if excess := len(r.lines) - actionLines; excess > 0 {
			r.lines = r.lines[excess:]
		}
	case event.qr != "":
		r.qr = event.qr
	case event.progress != "":
		r.progress = event.progress
	case event.ask != nil:
		r.question = event.ask
	case event.done:
		r.question, r.qr, r.progress, r.cancelling = nil, "", "", false
		r.done, r.notice, r.err = true, event.notice, event.err
	}
}

// runningKey is a key while an action runs: its question's input, or
// stopping it.
func (a *app) runningKey(key string, runes []rune) effect {
	r := &a.running
	if r.done {
		a.mode = browsing
		if r.err != nil {
			a.mode, a.message = messaging, r.err.Error()
		} else {
			a.status = r.notice
		}
		return noEffect
	}
	if key == "esc" || key == "ctrl+c" {
		r.question, r.cancelling = nil, true
		return cancelEffect
	}
	if r.question == nil {
		return noEffect
	}
	if key == "enter" {
		answer := string(r.question.input.text)
		line := r.question.prompt + " " + answer
		if r.question.secret {
			line = r.question.prompt + " " + strings.Repeat("•", 6)
		}
		r.lines = append(r.lines, line)
		r.question = nil
		a.answer = answer
		return answerEffect
	}
	if len(runes) > 0 && !strings.HasPrefix(key, "ctrl+") && !strings.HasPrefix(key, "alt+") {
		r.question.input.insert(runes)
	} else {
		r.question.input.key(key)
	}
	return noEffect
}

// actionInfo is an action as the screen lists it.
type actionInfo struct {
	title, description string
}
