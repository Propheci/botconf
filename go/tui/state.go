package tui

import (
	"fmt"
	"sort"
	"strings"

	"github.com/propheci/botconf/go/describe"
)

// seg is one step of a path into the values: a key, or a list index.
type seg struct {
	key     string
	index   int
	isIndex bool
}

func keySeg(key string) seg          { return seg{key: key} }
func indexSeg(index int) seg         { return seg{index: index, isIndex: true} }
func extend(path []seg, s seg) []seg { return append(append([]seg(nil), path...), s) }

// screen is an open table, list or map.
type screen struct {
	path   []seg
	title  string
	cursor int
}

// row is one line of a screen: an entry of a table, or an item of a list or
// a map.
type row struct {
	entry *describe.Entry
	item  *seg
	label string
}

func (r row) title() string {
	if r.entry != nil {
		return r.entry.Title
	}
	return r.label
}

type mode int

const (
	browsing mode = iota
	editing
	choosing
	confirming
	messaging
	helping
)

type targetKind int

const (
	valueTarget targetKind = iota
	envTarget
	nameTarget
)

type edit struct {
	title  string
	target targetKind
	path   []seg
	node   describe.Node
	envVar string
	input  input
	secret bool
	// note is what an empty input means, or the last error.
	note string
}

type choose struct {
	title   string
	path    []seg
	options []string
	cursor  int
}

type confirmKind int

const (
	confirmQuit confirmKind = iota
	confirmDelete
	confirmRegenerate
)

type confirm struct {
	kind   confirmKind
	path   []seg
	envVar string
}

// effect is what the caller does after a key.
type effect int

const (
	noEffect effect = iota
	// saveEffect: check the candidate files and write them, then call saved.
	saveEffect
	quitEffect
)

// shownKind is how a row's value reads.
type shownKind int

const (
	shownSet shownKind = iota
	shownDefault
	shownUnset
	shownMissing
	shownOpens
)

type shown struct {
	kind shownKind
	text string
}

// input is a line editor.
type input struct {
	text   []rune
	cursor int
}

func newInput(text string) input {
	runes := []rune(text)
	return input{text: runes, cursor: len(runes)}
}

func (in *input) key(key string) {
	switch key {
	case "ctrl+u":
		in.text, in.cursor = nil, 0
	case "backspace":
		if in.cursor > 0 {
			in.text = append(in.text[:in.cursor-1], in.text[in.cursor:]...)
			in.cursor--
		}
	case "delete":
		if in.cursor < len(in.text) {
			in.text = append(in.text[:in.cursor], in.text[in.cursor+1:]...)
		}
	case "left":
		in.cursor = max(in.cursor-1, 0)
	case "right":
		in.cursor = min(in.cursor+1, len(in.text))
	case "home":
		in.cursor = 0
	case "end":
		in.cursor = len(in.text)
	}
}

func (in *input) insert(runes []rune) {
	tail := append([]rune(nil), in.text[in.cursor:]...)
	in.text = append(append(in.text[:in.cursor], runes...), tail...)
	in.cursor += len(runes)
}

// app is the setup screens' state: pure (no terminal, no files), so it is
// tested by keys.
type app struct {
	description *describe.Description
	values      map[string]any
	env         *EnvFile
	envName     string
	screens     []screen
	mode        mode
	edit        edit
	choose      choose
	confirm     confirm
	message     string
	dirty       bool
	// status is a one-line notice, until the next key.
	status string
	token  func() (string, error)
}

func newApp(description *describe.Description, values map[string]any, env *EnvFile, envName string,
	token func() (string, error)) *app {
	if values == nil {
		values = map[string]any{}
	}
	return &app{
		description: description,
		values:      values,
		env:         env,
		envName:     envName,
		screens:     []screen{{title: description.Title}},
		token:       token,
	}
}

func (a *app) screen() *screen { return &a.screens[len(a.screens)-1] }

// candidate is the config file and the env file a save writes.
func (a *app) candidate() (string, string) {
	return a.description.Render(a.values), a.env.Render()
}

// saved records a save's outcome: clean, or the reason it was refused.
func (a *app) saved(notice string, err error) {
	if err != nil {
		a.mode, a.message = messaging, "Not saved: "+err.Error()
		return
	}
	a.dirty, a.status = false, notice
}

// nodeAt is the node the screen at path shows.
func (a *app) nodeAt(path []seg) *describe.Node {
	node := &describe.Node{Table: &a.description.Table}
	for _, s := range path {
		switch {
		case node.Table != nil && !s.isIndex:
			var next *describe.Node
			for index := range node.Table.Entries {
				if node.Table.Entries[index].Key == s.key {
					next = &node.Table.Entries[index].Node
				}
			}
			if next == nil {
				return nil
			}
			node = next
		case node.List != nil && s.isIndex:
			node = node.List
		case node.Map != nil && !s.isIndex:
			node = node.Map
		default:
			return nil
		}
	}
	return node
}

func (a *app) rows() []row {
	path := a.screen().path
	node := a.nodeAt(path)
	if node == nil {
		return nil
	}
	var rows []row
	switch {
	case node.Table != nil:
		for index := range node.Table.Entries {
			rows = append(rows, row{entry: &node.Table.Entries[index]})
		}
	case node.List != nil:
		items, _ := get(a.values, path).([]any)
		for index, item := range items {
			s := indexSeg(index)
			rows = append(rows, row{item: &s, label: fmt.Sprintf("%d. %s", index+1, labelOf(item))})
		}
	case node.Map != nil:
		items, _ := get(a.values, path).(map[string]any)
		names := make([]string, 0, len(items))
		for name := range items {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			s := keySeg(name)
			rows = append(rows, row{item: &s, label: name})
		}
	}
	return rows
}

func (a *app) isCollection() bool {
	node := a.nodeAt(a.screen().path)
	return node != nil && (node.List != nil || node.Map != nil)
}

func (a *app) child(s seg) []seg { return extend(a.screen().path, s) }

func (a *app) shown(r row) shown {
	if r.entry == nil {
		return shown{kind: shownOpens}
	}
	entry := r.entry
	value := get(a.values, a.child(keySeg(entry.Key)))
	switch {
	case entry.Node.Table != nil:
		return shown{kind: shownOpens}
	case !entry.Node.IsValue():
		count := len(asList(value))
		if items, ok := value.(map[string]any); ok {
			count = len(items)
		}
		switch count {
		case 0:
			return shown{shownOpens, "none"}
		case 1:
			return shown{shownOpens, "1 item"}
		default:
			return shown{shownOpens, fmt.Sprintf("%d items", count)}
		}
	case value != nil:
		return shown{shownSet, display(entry.Node, value)}
	case entry.Default != nil:
		return shown{shownDefault, display(entry.Node, entry.Default)}
	case entry.Required:
		return shown{kind: shownMissing}
	default:
		return shown{kind: shownUnset}
	}
}

// envOf is the variable an option names (its value, else its default or
// hint) and the env file's value of it.
func (a *app) envOf(entry *describe.Entry) (name, value string, named, set bool) {
	if entry.Node.Value == nil || entry.Node.Value.Kind != describe.EnvVar {
		return "", "", false, false
	}
	for _, candidate := range []any{get(a.values, a.child(keySeg(entry.Key))), entry.Default, entry.Hint} {
		if text, ok := candidate.(string); ok && text != "" {
			name = text
			break
		}
	}
	if name == "" {
		return "", "", false, false
	}
	value, set = a.env.Get(name)
	return name, value, true, set
}

// details is the details pane: the selected row's description and facts.
func (a *app) details(r row) (string, []string) {
	if r.entry == nil {
		return r.label, []string{"Enter: open · d: delete"}
	}
	entry := r.entry
	var paragraphs []string
	if entry.Description != "" {
		paragraphs = append(paragraphs, strings.Split(entry.Description, "\n\n")...)
	}
	facts := []string{"Key: " + entry.Key}
	if v := entry.Node.Value; v != nil {
		switch v.Kind {
		case describe.OneOf:
			facts = append(facts, "One of: "+strings.Join(v.Choices, ", "))
		case describe.Int:
			facts = append(facts, fmt.Sprintf("A whole number from %d to %d", v.Min, v.Max))
		case describe.TextList:
			facts = append(facts, "A list: values separated by commas")
		case describe.JSON:
			facts = append(facts, "Written as JSON")
		case describe.EnvVar:
			facts = append(facts, "Names an environment variable; its value is set in "+a.envName)
			if name, value, named, set := a.envOf(entry); named {
				switch {
				case !set:
					facts = append(facts, name+" is not set")
				case v.Env == describe.EnvPlain:
					facts = append(facts, name+" = "+value)
				default:
					facts = append(facts, name+" is set (hidden)")
				}
			}
		}
	}
	if entry.Required {
		facts = append(facts, "Required")
	} else if entry.Default != nil && entry.Node.IsValue() {
		facts = append(facts, "Default: "+display(entry.Node, entry.Default))
	}
	if entry.Runtime {
		facts = append(facts, "Can also change while the program runs (settings panel)")
	}
	return entry.Title, append(paragraphs, strings.Join(facts, "\n"))
}

// hints is the key help of the current screen.
func (a *app) hints() string {
	hints := []string{"↑↓ move", "enter edit/open"}
	if len(a.screens) > 1 {
		hints = append(hints, "esc back")
	}
	rows := a.rows()
	if cursor := a.screen().cursor; cursor < len(rows) {
		r := rows[cursor]
		switch {
		case r.entry != nil && r.entry.Node.IsValue():
			hints = append(hints, "u unset")
			if v := r.entry.Node.Value; v != nil && v.Kind == describe.EnvVar {
				hints = append(hints, "e set value")
				if v.Env == describe.EnvToken {
					hints = append(hints, "g generate")
				}
			}
		case r.item != nil:
			hints = append(hints, "d delete")
		}
	}
	if a.isCollection() {
		hints = append(hints, "a add")
	}
	return strings.Join(append(hints, "s save", "q quit", "? help"), " · ")
}

// key handles one key, as bubbletea names it ("a", "enter", "ctrl+c");
// runes are typed text.
func (a *app) key(key string, runes []rune) effect {
	a.status = ""
	if key == "ctrl+c" {
		return quitEffect
	}
	current := a.mode
	a.mode = browsing
	switch current {
	case browsing:
		return a.browse(key)
	case editing:
		a.editKey(key, runes)
	case choosing:
		a.chooseKey(key)
	case confirming:
		return a.confirmKey(key)
	}
	return noEffect
}

func (a *app) browse(key string) effect {
	rows := a.rows()
	last := max(len(rows)-1, 0)
	s := a.screen()
	s.cursor = min(s.cursor, last)
	var current *row
	if s.cursor < len(rows) {
		current = &rows[s.cursor]
	}
	switch key {
	case "up", "k":
		s.cursor = max(s.cursor-1, 0)
	case "down", "j":
		s.cursor = min(s.cursor+1, last)
	case "pgup":
		s.cursor = max(s.cursor-10, 0)
	case "pgdown":
		s.cursor = min(s.cursor+10, last)
	case "home":
		s.cursor = 0
	case "end":
		s.cursor = last
	case "enter", "right", "l":
		if current != nil {
			a.activate(*current)
		}
	case "esc", "left", "backspace", "h":
		if len(a.screens) > 1 {
			a.screens = a.screens[:len(a.screens)-1]
		}
	case "s", "ctrl+s":
		return saveEffect
	case "q":
		if !a.dirty {
			return quitEffect
		}
		a.mode, a.confirm = confirming, confirm{kind: confirmQuit}
	case "?":
		a.mode = helping
	case "u", "delete":
		switch {
		case current != nil && current.entry != nil && current.entry.Node.IsValue():
			if remove(a.values, a.child(keySeg(current.entry.Key))) {
				a.dirty = true
			}
		case current != nil && current.item != nil:
			a.mode, a.confirm = confirming, confirm{kind: confirmDelete, path: a.child(*current.item)}
		}
	case "d":
		if current != nil && current.item != nil {
			a.mode, a.confirm = confirming, confirm{kind: confirmDelete, path: a.child(*current.item)}
		}
	case "a":
		a.add()
	case "e":
		if current != nil && current.entry != nil {
			a.editEnv(current.entry)
		}
	case "g":
		if current != nil && current.entry != nil {
			a.generate(current.entry, false)
		}
	}
	return noEffect
}

func (a *app) activate(r row) {
	if r.item != nil {
		a.open(*r.item, r.label)
		return
	}
	entry := r.entry
	path := a.child(keySeg(entry.Key))
	value := get(a.values, path)
	if value == nil {
		value = entry.Default
	}
	switch v := entry.Node.Value; {
	case v != nil && v.Kind == describe.Bool:
		current, _ := value.(bool)
		set(a.values, path, !current)
		a.dirty = true
	case v != nil && v.Kind == describe.OneOf:
		cursor := 0
		for index, choice := range v.Choices {
			if choice == value {
				cursor = index
			}
		}
		a.mode = choosing
		a.choose = choose{title: entry.Title, path: path, options: v.Choices, cursor: cursor}
	case entry.Node.IsValue():
		text := ""
		if current := get(a.values, path); current != nil {
			text = editText(entry.Node, current)
		}
		note := "Empty: unset."
		switch {
		case entry.Required:
			note = "Required."
		case entry.Default != nil:
			note = "Empty: the default, " + display(entry.Node, entry.Default) + "."
		}
		a.mode = editing
		a.edit = edit{
			title:  entry.Title,
			target: valueTarget,
			path:   path,
			node:   entry.Node,
			input:  newInput(text),
			secret: v != nil && v.Kind == describe.Secret,
			note:   note,
		}
	default:
		a.open(keySeg(entry.Key), entry.Title)
	}
}

func (a *app) open(s seg, title string) {
	a.screens = append(a.screens, screen{path: a.child(s), title: title})
}

func (a *app) add() {
	path := a.screen().path
	node := a.nodeAt(path)
	switch {
	case node == nil:
	case node.List != nil:
		items := asList(get(a.values, path))
		items = append(append([]any(nil), items...), map[string]any{})
		set(a.values, path, items)
		a.dirty = true
		a.screen().cursor = len(items) - 1
		a.open(indexSeg(len(items)-1), fmt.Sprintf("%d.", len(items)))
	case node.Map != nil:
		a.mode = editing
		a.edit = edit{title: "New entry's name", target: nameTarget, path: path, note: "Its key in the file."}
	}
}

func (a *app) editEnv(entry *describe.Entry) {
	if entry.Node.Value == nil || entry.Node.Value.Kind != describe.EnvVar {
		return
	}
	name, value, named, _ := a.envOf(entry)
	if !named {
		a.status = "Name the variable first (enter)."
		return
	}
	a.mode = editing
	a.edit = edit{
		title:  name + " in " + a.envName,
		target: envTarget,
		envVar: name,
		input:  newInput(value),
		secret: entry.Node.Value.Env != describe.EnvPlain,
	}
}

func (a *app) generate(entry *describe.Entry, replace bool) {
	if v := entry.Node.Value; v == nil || v.Kind != describe.EnvVar || v.Env != describe.EnvToken {
		return
	}
	name, value, named, _ := a.envOf(entry)
	if !named {
		a.status = "Name the variable first (enter)."
		return
	}
	if value != "" && !replace {
		a.mode, a.confirm = confirming, confirm{kind: confirmRegenerate, envVar: name}
		return
	}
	// A name only hinted becomes the option's value.
	path := a.child(keySeg(entry.Key))
	if get(a.values, path) == nil && entry.Default == nil {
		set(a.values, path, name)
	}
	a.setToken(name)
}

func (a *app) setToken(name string) {
	token, err := a.token()
	if err != nil {
		a.mode, a.message = messaging, err.Error()
		return
	}
	a.env.Set(name, token)
	a.dirty = true
	a.status = fmt.Sprintf("Generated a new %s in %s; give the other side the same value.", name, a.envName)
}

func (a *app) editKey(key string, runes []rune) {
	switch key {
	case "esc":
		return
	case "enter":
		if err := a.commit(); err != nil {
			a.edit.note = err.Error()
			a.mode = editing
		}
		return
	}
	if len(runes) > 0 && !strings.HasPrefix(key, "ctrl+") && !strings.HasPrefix(key, "alt+") {
		a.edit.input.insert(runes)
	} else {
		a.edit.input.key(key)
	}
	a.mode = editing
}

func (a *app) commit() error {
	text := string(a.edit.input.text)
	switch a.edit.target {
	case valueTarget:
		if strings.TrimSpace(text) == "" {
			remove(a.values, a.edit.path)
		} else {
			value, err := describe.ParseText(a.edit.node, text)
			if err != nil {
				return capitalized(err)
			}
			set(a.values, a.edit.path, value)
		}
	case envTarget:
		a.env.Set(a.edit.envVar, text)
	case nameTarget:
		name := strings.TrimSpace(text)
		if name == "" {
			return fmt.Errorf("A name is needed.")
		}
		items, _ := get(a.values, a.edit.path).(map[string]any)
		if _, taken := items[name]; taken {
			return fmt.Errorf("%s already exists.", name)
		}
		if items == nil {
			items = map[string]any{}
		}
		items[name] = map[string]any{}
		set(a.values, a.edit.path, items)
		a.open(keySeg(name), name)
	}
	a.dirty = true
	return nil
}

// capitalized is a parse error as a sentence.
func capitalized(err error) error {
	text := err.Error()
	if text == "" {
		return err
	}
	return fmt.Errorf("%s.", strings.ToUpper(text[:1])+text[1:])
}

func (a *app) chooseKey(key string) {
	switch key {
	case "esc":
		return
	case "up", "k":
		a.choose.cursor = max(a.choose.cursor-1, 0)
	case "down", "j":
		a.choose.cursor = min(a.choose.cursor+1, len(a.choose.options)-1)
	case "enter":
		if a.choose.cursor < len(a.choose.options) {
			set(a.values, a.choose.path, a.choose.options[a.choose.cursor])
			a.dirty = true
		}
		return
	}
	a.mode = choosing
}

func (a *app) confirmKey(key string) effect {
	if key != "y" && key != "Y" && key != "enter" {
		return noEffect
	}
	switch a.confirm.kind {
	case confirmQuit:
		return quitEffect
	case confirmDelete:
		remove(a.values, a.confirm.path)
		a.dirty = true
		a.screen().cursor = min(a.screen().cursor, max(len(a.rows())-1, 0))
	case confirmRegenerate:
		a.setToken(a.confirm.envVar)
	}
	return noEffect
}

// labelOf is a list item's label: its id, name or title, else its first
// text.
func labelOf(item any) string {
	object, ok := item.(map[string]any)
	if !ok {
		return describe.TOMLValue(item)
	}
	for _, key := range []string{"id", "name", "title"} {
		if text, ok := object[key].(string); ok {
			return text
		}
	}
	keys := make([]string, 0, len(object))
	for key := range object {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		if text, ok := object[key].(string); ok {
			return text
		}
	}
	return "(new)"
}

func display(node describe.Node, value any) string {
	if text, ok := value.(string); ok {
		if node.Value != nil && node.Value.Kind == describe.Secret && text != "" {
			return "••••••"
		}
		return text
	}
	if node.Value != nil && node.Value.Kind == describe.TextList {
		var parts []string
		for _, item := range asList(value) {
			parts = append(parts, fmt.Sprint(item))
		}
		return strings.Join(parts, ", ")
	}
	return describe.TOMLValue(value)
}

func editText(node describe.Node, value any) string {
	if text, ok := value.(string); ok {
		return text
	}
	if node.Value != nil && node.Value.Kind == describe.TextList {
		var parts []string
		for _, item := range asList(value) {
			text, ok := item.(string)
			if !ok || strings.Contains(text, ",") {
				return jsonText(value)
			}
			parts = append(parts, text)
		}
		return strings.Join(parts, ", ")
	}
	if node.Value != nil && node.Value.Kind != describe.JSON {
		return describe.TOMLValue(value)
	}
	return jsonText(value)
}
