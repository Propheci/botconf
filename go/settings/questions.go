package settings

import (
	"sync"
	"time"

	"github.com/propheci/botconf/go/settingspb"
)

// Questions keeps the panel's pending question per conversation and user, so
// that a renderer can route the user's next message to Answer. One question
// at a time per (conversation, user); a newer one replaces it. Questions
// expire after their time to live.
type Questions struct {
	mu      sync.Mutex
	pending map[questionKey]question
	ttl     time.Duration
	now     func() time.Time
}

type questionKey struct{ conversation, user string }

type question struct {
	question     string
	panelMessage string
	expires      time.Time
}

// NewQuestions keeps questions for ttl (10 minutes is a good default).
func NewQuestions(ttl time.Duration) *Questions {
	return &Questions{pending: map[questionKey]question{}, ttl: ttl, now: time.Now}
}

// Ask records that ask was put to user in conversation, with the panel
// message it came from (to redraw after the answer).
func (q *Questions) Ask(conversation, user string, ask *settingspb.Ask, panelMessage string) {
	q.mu.Lock()
	defer q.mu.Unlock()
	q.pending[questionKey{conversation, user}] = question{
		question:     ask.GetQuestion(),
		panelMessage: panelMessage,
		expires:      q.now().Add(q.ttl),
	}
}

// Pending returns the question waiting for user's answer in conversation.
func (q *Questions) Pending(conversation, user string) (questionToken string, panelMessage string, ok bool) {
	q.mu.Lock()
	defer q.mu.Unlock()
	key := questionKey{conversation, user}
	pending, ok := q.pending[key]
	if !ok {
		return "", "", false
	}
	if !q.now().Before(pending.expires) {
		delete(q.pending, key)
		return "", "", false
	}
	return pending.question, pending.panelMessage, true
}

// Finish forgets the question waiting for user in conversation.
func (q *Questions) Finish(conversation, user string) {
	q.mu.Lock()
	defer q.mu.Unlock()
	delete(q.pending, questionKey{conversation, user})
}
