// Package describe describes a program's whole config file from its config
// struct, so that each option is described once: its key, title,
// description, value kind, default and whether it can change at runtime.
// From the description it derives the runtime settings (RuntimeSettings), the
// example file (Example) and the file an offline setup writes (Render, used
// by the tui package). It is the Go side of the Rust `botconf::describe`.
//
// Of reads the struct's tags:
//
//   - `toml:"name"`: the key (`-` skips the field; an embedded struct without
//     a name adds its fields to the table, as go-toml reads it).
//   - `doc:"…"`: the description. `title:"…"`: the title, else the key in
//     words.
//   - `botconf:"Title"`: the option can change at runtime (a runtime
//     setting), with this title.
//   - `default:"…"`, `example:"…"`, `hint:"…"`: the default the program
//     applies, what the generated example sets, and what an absent option
//     shows commented out; written as the value's kind reads text (a list as
//     `a,b` or JSON).
//   - `required:"true"`: the program refuses to start without it.
//   - `env:"token"`, `env:"secret"`, `env:"plain"`: the option names an
//     environment variable holding the value (a token can be generated).
//   - `secret:"true"`: a secret value, masked on screens.
//   - `domain:"wuphf.chat"`: where its values come from: the setup screens
//     suggest them by name, and a runtime option is a reference.
//   - `jsonschema:"enum=a,enum=b,minimum=0,maximum=9"`: the allowed values.
//
// A pointer field is optional (nullable). Values are JSON-like: maps of
// string keys, slices, strings, bools and numbers.
package describe

import (
	"encoding/json"
	"fmt"
	"math"
	"reflect"
	"strconv"
	"strings"
)

// Kind is a single value's kind.
type Kind int

const (
	Text Kind = iota
	// Secret is text the file holds but a screen masks.
	Secret
	Bool
	Int
	Float
	// OneOf is one of the Choices.
	OneOf
	// TextList is a list of text values.
	TextList
	// EnvVar names an environment variable whose value is the setting.
	EnvVar
	// JSON is any value (the fallback).
	JSON
)

// EnvValue is what an EnvVar option's variable holds.
type EnvValue int

const (
	// EnvPlain is a plain value.
	EnvPlain EnvValue = iota
	// EnvToken is a shared secret any random value serves: masked, can be
	// generated.
	EnvToken
	// EnvSecret is a secret issued elsewhere (an API hash, a bot token):
	// masked.
	EnvSecret
)

// Value describes a single value.
type Value struct {
	Kind Kind
	// Min and Max bound an Int.
	Min, Max int64
	// Choices are a OneOf's values.
	Choices []string
	// Env is what an EnvVar's variable holds.
	Env EnvValue
}

// Node is what a key holds: exactly one of its fields is set.
type Node struct {
	Value *Value
	// Table is `[key]`.
	Table *Table
	// List is `[[key]]` for tables, an array for values.
	List *Node
	// Map is `[key.<name>]` for tables, `[key]` of values for values.
	Map *Node
}

// Table is the options of a table, in file order.
type Table struct {
	Entries []Entry
}

// Entry is one key of a table: an option, or a table of them.
type Entry struct {
	Key, Title, Description string
	Node                    Node
	Required                bool
	// Default is what the program uses when the key is absent.
	Default any
	// Example is what the generated example sets.
	Example any
	// Hint is shown commented out when the key is absent and has no default.
	Hint any
	// Runtime: the option can change while the program runs.
	Runtime bool
	// Nullable: a runtime setting of it can be cleared (a pointer).
	Nullable bool
	// Domain is where its values come from (a chat, a user): the setup
	// screens suggest them by name, and a runtime option is a reference.
	Domain string
}

// Description is a program's whole configuration file.
type Description struct {
	Title, Description string
	Table              Table
}

// Of describes config's type (a struct, or a pointer to one) from its tags.
func Of(title, description string, config any) (*Description, error) {
	kind := reflect.TypeOf(config)
	for kind != nil && kind.Kind() == reflect.Pointer {
		kind = kind.Elem()
	}
	if kind == nil || kind.Kind() != reflect.Struct {
		return nil, fmt.Errorf("a configuration is described by a struct, not %v", kind)
	}
	table, err := tableOf(kind)
	if err != nil {
		return nil, err
	}
	return &Description{Title: title, Description: description, Table: *table}, nil
}

// MustOf is Of, panicking on a type it cannot describe (a programming error).
func MustOf(title, description string, config any) *Description {
	described, err := Of(title, description, config)
	if err != nil {
		panic(err)
	}
	return described
}

func tableOf(kind reflect.Type) (*Table, error) {
	table := &Table{}
	for index := range kind.NumField() {
		field := kind.Field(index)
		name, _, _ := strings.Cut(field.Tag.Get("toml"), ",")
		if name == "-" {
			continue
		}
		embedded := field.Type
		for embedded.Kind() == reflect.Pointer {
			embedded = embedded.Elem()
		}
		if field.Anonymous && name == "" && embedded.Kind() == reflect.Struct {
			inner, err := tableOf(embedded)
			if err != nil {
				return nil, err
			}
			table.Entries = append(table.Entries, inner.Entries...)
			continue
		}
		if !field.IsExported() {
			continue
		}
		if name == "" {
			name = field.Name
		}
		entry, err := entryOf(field, name)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", name, err)
		}
		table.Entries = append(table.Entries, entry)
	}
	return table, nil
}

func entryOf(field reflect.StructField, key string) (Entry, error) {
	node, nullable, err := fieldNode(field)
	if err != nil {
		return Entry{}, err
	}
	runtimeTitle, runtime := field.Tag.Lookup("botconf")
	title := field.Tag.Get("title")
	if title == "" {
		title = runtimeTitle
	}
	if title == "" {
		title = TitleOf(key)
	}
	entry := Entry{
		Key:         key,
		Title:       title,
		Description: field.Tag.Get("doc"),
		Node:        node,
		Required:    field.Tag.Get("required") == "true",
		Runtime:     runtime,
		Nullable:    nullable,
		Domain:      field.Tag.Get("domain"),
	}
	for _, slot := range []struct {
		tag  string
		into *any
	}{{"default", &entry.Default}, {"example", &entry.Example}, {"hint", &entry.Hint}} {
		text, ok := field.Tag.Lookup(slot.tag)
		if !ok {
			continue
		}
		value, err := ParseText(node, text)
		if err != nil {
			return Entry{}, fmt.Errorf("%s tag: %w", slot.tag, err)
		}
		*slot.into = value
	}
	return entry, nil
}

// fieldNode is the node of a field's type, with its tags applied.
func fieldNode(field reflect.StructField) (Node, bool, error) {
	kind := field.Type
	nullable := false
	for kind.Kind() == reflect.Pointer {
		kind = kind.Elem()
		nullable = true
	}
	switch env := field.Tag.Get("env"); env {
	case "":
	case "plain", "token", "secret":
		what := map[string]EnvValue{"plain": EnvPlain, "token": EnvToken, "secret": EnvSecret}[env]
		return Node{Value: &Value{Kind: EnvVar, Env: what}}, nullable, nil
	default:
		return Node{}, false, fmt.Errorf("env tag %q: not token, secret or plain", env)
	}
	if field.Tag.Get("secret") == "true" {
		return Node{Value: &Value{Kind: Secret}}, nullable, nil
	}
	node, err := typeNode(kind)
	if err != nil {
		return Node{}, false, err
	}
	enum, minimum, maximum := schemaTags(field)
	if node.Value != nil {
		switch {
		case len(enum) > 0 && node.Value.Kind == Text:
			node.Value = &Value{Kind: OneOf, Choices: enum}
		case node.Value.Kind == Int:
			bounded := *node.Value
			if minimum != nil {
				bounded.Min = *minimum
			}
			if maximum != nil {
				bounded.Max = *maximum
			}
			node.Value = &bounded
		}
	}
	return node, nullable, nil
}

// typeNode is the node of a Go type, without tags.
func typeNode(kind reflect.Type) (Node, error) {
	for kind.Kind() == reflect.Pointer {
		kind = kind.Elem()
	}
	value := func(v Value) (Node, error) { return Node{Value: &v}, nil }
	switch kind.Kind() {
	case reflect.String:
		return value(Value{Kind: Text})
	case reflect.Bool:
		return value(Value{Kind: Bool})
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64,
		reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		low, high := bounds(kind)
		return value(Value{Kind: Int, Min: low, Max: high})
	case reflect.Float32, reflect.Float64:
		return value(Value{Kind: Float})
	case reflect.Interface:
		return value(Value{Kind: JSON})
	case reflect.Slice, reflect.Array:
		inner, err := typeNode(kind.Elem())
		if err != nil {
			return Node{}, err
		}
		if inner.Value != nil && inner.Value.Kind == Text {
			return value(Value{Kind: TextList})
		}
		return Node{List: &inner}, nil
	case reflect.Map:
		if kind.Key().Kind() != reflect.String {
			return Node{}, fmt.Errorf("a map's keys must be strings, not %s", kind.Key())
		}
		inner, err := typeNode(kind.Elem())
		if err != nil {
			return Node{}, err
		}
		return Node{Map: &inner}, nil
	case reflect.Struct:
		// A struct that reads itself from text (a time, an address) is text.
		if reflect.PointerTo(kind).Implements(textUnmarshaler) {
			return value(Value{Kind: Text})
		}
		table, err := tableOf(kind)
		if err != nil {
			return Node{}, err
		}
		return Node{Table: table}, nil
	default:
		return Node{}, fmt.Errorf("no description for %s", kind)
	}
}

var textUnmarshaler = reflect.TypeOf((*interface{ UnmarshalText([]byte) error })(nil)).Elem()

// schemaTags reads the `jsonschema` tag's enum, minimum and maximum.
func schemaTags(field reflect.StructField) (enum []string, minimum, maximum *int64) {
	for _, part := range strings.Split(field.Tag.Get("jsonschema"), ",") {
		name, value, _ := strings.Cut(part, "=")
		switch name {
		case "enum":
			enum = append(enum, value)
		case "minimum", "maximum":
			number, err := strconv.ParseInt(value, 10, 64)
			if err != nil {
				continue
			}
			if name == "minimum" {
				minimum = &number
			} else {
				maximum = &number
			}
		}
	}
	return enum, minimum, maximum
}

// bounds is the range an integer type holds, capped to int64.
func bounds(kind reflect.Type) (int64, int64) {
	bits := kind.Bits()
	switch kind.Kind() {
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		if bits >= 64 {
			return 0, math.MaxInt64
		}
		return 0, int64(1)<<bits - 1
	default:
		if bits >= 64 {
			return math.MinInt64, math.MaxInt64
		}
		return -(int64(1) << (bits - 1)), int64(1)<<(bits-1) - 1
	}
}

// TitleOf is a key in words: `max_message_length` → `Max message length`.
func TitleOf(key string) string {
	words := strings.NewReplacer("_", " ", "-", " ").Replace(key)
	if words == "" {
		return ""
	}
	first := []rune(words)
	return strings.ToUpper(string(first[0])) + string(first[1:])
}

// ParseText reads a value of node's kind written as text (a tag, a screen's
// input): text as is, numbers and bools parsed, a list as JSON or `a, b`,
// anything else as JSON.
func ParseText(node Node, text string) (any, error) {
	if node.Value == nil {
		return decodeJSON(text)
	}
	trimmed := strings.TrimSpace(text)
	switch node.Value.Kind {
	case Text, Secret:
		return text, nil
	case EnvVar:
		return trimmed, nil
	case Bool:
		switch trimmed {
		case "true", "yes":
			return true, nil
		case "false", "no":
			return false, nil
		}
		return nil, fmt.Errorf("true or false")
	case Int:
		number, err := strconv.ParseInt(trimmed, 10, 64)
		if err != nil {
			return nil, fmt.Errorf("a whole number, please")
		}
		if number < node.Value.Min || number > node.Value.Max {
			return nil, fmt.Errorf("from %d to %d", node.Value.Min, node.Value.Max)
		}
		return number, nil
	case Float:
		number, err := strconv.ParseFloat(trimmed, 64)
		if err != nil || math.IsInf(number, 0) || math.IsNaN(number) {
			return nil, fmt.Errorf("a number, please")
		}
		return number, nil
	case OneOf:
		for _, choice := range node.Value.Choices {
			if choice == trimmed {
				return trimmed, nil
			}
		}
		return nil, fmt.Errorf("one of: %s", strings.Join(node.Value.Choices, ", "))
	case TextList:
		if strings.HasPrefix(trimmed, "[") {
			var items []string
			if err := json.Unmarshal([]byte(trimmed), &items); err != nil {
				return nil, fmt.Errorf("a JSON list of texts: %w", err)
			}
			return toAnys(items), nil
		}
		items := []any{}
		for _, item := range strings.Split(trimmed, ",") {
			if item = strings.TrimSpace(item); item != "" {
				items = append(items, item)
			}
		}
		return items, nil
	default:
		return decodeJSON(trimmed)
	}
}

func toAnys(items []string) []any {
	out := make([]any, len(items))
	for index, item := range items {
		out[index] = item
	}
	return out
}

// IsValue reports whether a node is written in place (`key = value`), not
// as a table of its own.
func (n Node) IsValue() bool {
	switch {
	case n.Value != nil:
		return true
	case n.List != nil:
		return n.List.Value != nil
	case n.Map != nil:
		return n.Map.Value != nil
	}
	return false
}

// Placeholder is the value shown commented out when the key is absent.
func (e Entry) Placeholder() any {
	if e.Hint != nil {
		return e.Hint
	}
	if e.Default != nil {
		return e.Default
	}
	if e.Node.List != nil {
		return []any{}
	}
	if e.Node.Value == nil {
		return ""
	}
	switch v := e.Node.Value; v.Kind {
	case Bool:
		return false
	case Int:
		return max(v.Min, 0)
	case Float:
		return 0.0
	case TextList:
		return []any{}
	case OneOf:
		if len(v.Choices) > 0 {
			return v.Choices[0]
		}
	}
	return ""
}

// decodeJSON reads JSON with its whole numbers as int64, so TOML writes
// them as integers (`1`, not `1.0`) and integer fields read them.
func decodeJSON(text string) (any, error) {
	decoder := json.NewDecoder(strings.NewReader(text))
	decoder.UseNumber()
	var value any
	if err := decoder.Decode(&value); err != nil {
		return nil, fmt.Errorf("not JSON: %w", err)
	}
	if decoder.More() {
		return nil, fmt.Errorf("not JSON: more than one value")
	}
	return numbers(value), nil
}

func numbers(value any) any {
	switch value := value.(type) {
	case json.Number:
		if whole, err := value.Int64(); err == nil {
			return whole
		}
		float, _ := value.Float64()
		return float
	case map[string]any:
		for key, item := range value {
			value[key] = numbers(item)
		}
	case []any:
		for index, item := range value {
			value[index] = numbers(item)
		}
	}
	return value
}
