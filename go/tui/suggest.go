package tui

// Suggestion is a value the host offers for a domain, and how it reads.
type Suggestion struct {
	Value, Label string
}

// Query is what a host's suggestions are asked for.
type Query struct {
	Domain string
	// Text is what is typed so far.
	Text string
	// Values are the values being edited (the host may narrow by them, e.g.
	// to the connectors the file lists).
	Values map[string]any
	Limit  int
}

// SuggestFunc gives the host's matches for a query, or why there are none.
type SuggestFunc func(Query) ([]Suggestion, error)

// suggestions is how many matches an input shows.
const suggestions = 8

// suggest asks the host for matches, remembering their labels.
func (a *app) suggest(domain, text string) ([]Suggestion, error) {
	if a.suggestFunc == nil {
		return nil, nil
	}
	found, err := a.suggestFunc(Query{Domain: domain, Text: text, Values: a.values, Limit: suggestions})
	if err != nil {
		return nil, err
	}
	for _, suggestion := range found {
		a.labels[domain+"\x00"+suggestion.Value] = suggestion.Label
	}
	return found, nil
}

// labelFor is how a domain's value reads: the host's label for it, else the
// value.
func (a *app) labelFor(domain, value string) string {
	key := domain + "\x00" + value
	if label, ok := a.labels[key]; ok {
		return label
	}
	label := value
	if found, err := a.suggest(domain, value); err == nil {
		for _, suggestion := range found {
			if suggestion.Value == value {
				label = suggestion.Label
			}
		}
	}
	a.labels[key] = label
	return label
}

// searching makes the editor search domain as it is typed.
func (a *app) searching(domain string) {
	a.edit.domain = domain
	a.edit.highlight = -1
	a.refreshSuggestions()
}

func (a *app) refreshSuggestions() {
	if a.edit.domain == "" {
		return
	}
	found, err := a.suggest(a.edit.domain, string(a.edit.input.text))
	a.edit.suggestions, a.edit.unsuggested = found, ""
	if err != nil {
		a.edit.suggestions, a.edit.unsuggested = nil, err.Error()
	}
	if a.edit.highlight >= len(a.edit.suggestions) {
		a.edit.highlight = -1
	}
}
