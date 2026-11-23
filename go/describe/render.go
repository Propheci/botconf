package describe

import (
	"fmt"
	"math"
	"reflect"
	"sort"
	"strconv"
	"strings"
)

// width is the longest a comment line of a rendered file gets, `# `
// included.
const width = 80

// Example is the example file: each option's description, the examples set,
// the rest commented out.
func (d *Description) Example() string {
	return d.Render(exampleValues(&d.Table))
}

// Render is the config file holding values, written from the description:
// each option preceded by its description, absent ones commented out. Keys
// the description does not know are kept, commented out, at the end.
func (d *Description) Render(values map[string]any) string {
	var out strings.Builder
	comment(&out, d.Title)
	if d.Description != "" {
		out.WriteString("#\n")
		comment(&out, d.Description)
	}
	renderTable(&out, &d.Table, values, nil)
	var unknown []keyValue
	collectUnknown(&d.Table, values, "", &unknown)
	if len(unknown) > 0 {
		out.WriteString("\n# Not in this program's description (kept, but it ignores them):\n")
		for _, pair := range unknown {
			fmt.Fprintf(&out, "# %s = %s\n", pair.key, TOMLValue(pair.value))
		}
	}
	return out.String()
}

type keyValue struct {
	key   string
	value any
}

func exampleValues(table *Table) map[string]any {
	values := map[string]any{}
	for _, entry := range table.Entries {
		switch {
		case entry.Example != nil:
			values[entry.Key] = entry.Example
		case entry.Node.Table != nil:
			if inner := exampleValues(entry.Node.Table); len(inner) > 0 {
				values[entry.Key] = inner
			}
		}
	}
	return values
}

// comment writes text as comment lines, wrapped; blank lines separate
// paragraphs.
func comment(out *strings.Builder, text string) {
	for index, paragraph := range strings.Split(text, "\n\n") {
		if index > 0 {
			out.WriteString("#\n")
		}
		line := "#"
		for _, word := range strings.Fields(paragraph) {
			if len(line) > 1 && len(line)+1+len(word) > width {
				out.WriteString(line + "\n")
				line = "#"
			}
			line += " " + word
		}
		if len(line) > 1 {
			out.WriteString(line + "\n")
		}
	}
}

// entryComment is the comment block before an option: its description, then
// what it defaults to and whether it is required.
func entryComment(out *strings.Builder, entry *Entry) {
	out.WriteString("\n")
	if entry.Description != "" {
		comment(out, entry.Description)
	}
	if v := entry.Node.Value; v != nil && v.Kind == OneOf {
		comment(out, "One of: "+strings.Join(v.Choices, ", ")+".")
	}
	if entry.Required {
		out.WriteString("# Required.\n")
	} else if entry.Default != nil && entry.Node.IsValue() {
		fmt.Fprintf(out, "# Default: %s.\n", TOMLValue(entry.Default))
	}
}

// renderTable writes the values first, then the tables: TOML puts a table's
// keys before its sub-tables.
func renderTable(out *strings.Builder, table *Table, values map[string]any, path []string) {
	for index := range table.Entries {
		entry := &table.Entries[index]
		if !entry.Node.IsValue() {
			continue
		}
		entryComment(out, entry)
		if value, ok := values[entry.Key]; ok && value != nil {
			fmt.Fprintf(out, "%s = %s\n", tomlKey(entry.Key), TOMLValue(value))
		} else {
			fmt.Fprintf(out, "# %s = %s\n", tomlKey(entry.Key), TOMLValue(entry.Placeholder()))
		}
	}
	for index := range table.Entries {
		entry := &table.Entries[index]
		if entry.Node.IsValue() {
			continue
		}
		entryPath := append(append([]string(nil), path...), entry.Key)
		value := values[entry.Key]
		entryComment(out, entry)
		switch {
		case entry.Node.Table != nil:
			inner, _ := value.(map[string]any)
			// An optional table (a pointer) left empty is written commented
			// out: its header alone would make it present.
			if entry.Nullable && len(inner) == 0 {
				var table strings.Builder
				fmt.Fprintf(&table, "[%s]\n", header(entryPath))
				renderTable(&table, entry.Node.Table, inner, entryPath)
				out.WriteString(commentOut(table.String()))
				continue
			}
			fmt.Fprintf(out, "[%s]\n", header(entryPath))
			renderTable(out, entry.Node.Table, inner, entryPath)
		case entry.Node.List != nil:
			items := asList(value)
			if len(items) == 0 {
				fmt.Fprintf(out, "# [[%s]]\n", header(entryPath))
			}
			for _, item := range items {
				fmt.Fprintf(out, "[[%s]]\n", header(entryPath))
				renderItem(out, entry.Node.List, item, entryPath)
			}
		case entry.Node.Map != nil:
			items, _ := value.(map[string]any)
			if len(items) == 0 {
				fmt.Fprintf(out, "# [%s.<name>]\n", header(entryPath))
			}
			for _, name := range sortedKeys(items) {
				itemPath := append(append([]string(nil), entryPath...), name)
				fmt.Fprintf(out, "[%s]\n", header(itemPath))
				renderItem(out, entry.Node.Map, items[name], itemPath)
			}
		}
	}
}

// renderItem writes one table of a list or a map, after its header.
func renderItem(out *strings.Builder, node *Node, item any, path []string) {
	if node.Table != nil {
		values, _ := item.(map[string]any)
		renderTable(out, node.Table, values, path)
		return
	}
	out.WriteString(TOMLValue(item) + "\n")
}

func collectUnknown(table *Table, values map[string]any, prefix string, out *[]keyValue) {
	for _, key := range sortedKeys(values) {
		path := key
		if prefix != "" {
			path = prefix + "." + key
		}
		var found *Entry
		for index := range table.Entries {
			if table.Entries[index].Key == key {
				found = &table.Entries[index]
				break
			}
		}
		switch {
		case found == nil:
			*out = append(*out, keyValue{path, values[key]})
		case found.Node.Table != nil:
			if inner, ok := values[key].(map[string]any); ok {
				collectUnknown(found.Node.Table, inner, path, out)
			}
		}
	}
}

func sortedKeys(values map[string]any) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

// asList is a slice of any kind as []any.
func asList(value any) []any {
	if items, ok := value.([]any); ok {
		return items
	}
	reflected := reflect.ValueOf(value)
	if !reflected.IsValid() || (reflected.Kind() != reflect.Slice && reflected.Kind() != reflect.Array) {
		return nil
	}
	items := make([]any, reflected.Len())
	for index := range items {
		items[index] = reflected.Index(index).Interface()
	}
	return items
}

// tomlKey is a key as TOML writes it: bare when it can be.
func tomlKey(key string) string {
	bare := key != ""
	for _, c := range key {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_' || c == '-') {
			bare = false
			break
		}
	}
	if bare {
		return key
	}
	return tomlString(key)
}

func header(path []string) string {
	keys := make([]string, len(path))
	for index, key := range path {
		keys[index] = tomlKey(key)
	}
	return strings.Join(keys, ".")
}

func tomlString(text string) string {
	var out strings.Builder
	out.WriteByte('"')
	for _, c := range text {
		switch {
		case c == '"':
			out.WriteString(`\"`)
		case c == '\\':
			out.WriteString(`\\`)
		case c == '\n':
			out.WriteString(`\n`)
		case c == '\t':
			out.WriteString(`\t`)
		case c == '\r':
			out.WriteString(`\r`)
		case c < 0x20 || c == 0x7f:
			fmt.Fprintf(&out, `\u%04X`, c)
		default:
			out.WriteRune(c)
		}
	}
	out.WriteByte('"')
	return out.String()
}

// TOMLValue is a value as TOML writes it inline.
func TOMLValue(value any) string {
	switch value := value.(type) {
	case nil:
		return `""`
	case string:
		return tomlString(value)
	case bool:
		return strconv.FormatBool(value)
	case float32:
		return formatFloat(float64(value))
	case float64:
		return formatFloat(value)
	case map[string]any:
		parts := []string{}
		for _, key := range sortedKeys(value) {
			if value[key] != nil {
				parts = append(parts, tomlKey(key)+" = "+TOMLValue(value[key]))
			}
		}
		return "{ " + strings.Join(parts, ", ") + " }"
	}
	reflected := reflect.ValueOf(value)
	switch reflected.Kind() {
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return strconv.FormatInt(reflected.Int(), 10)
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return strconv.FormatUint(reflected.Uint(), 10)
	case reflect.Slice, reflect.Array:
		parts := []string{}
		for _, item := range asList(value) {
			parts = append(parts, TOMLValue(item))
		}
		return "[" + strings.Join(parts, ", ") + "]"
	}
	return tomlString(fmt.Sprint(value))
}

func formatFloat(value float64) string {
	if value == math.Trunc(value) && !math.IsInf(value, 0) {
		return strconv.FormatFloat(value, 'f', 1, 64)
	}
	return strconv.FormatFloat(value, 'g', -1, 64)
}

// commentOut is text with every line that is not already a comment (or
// blank) commented out.
func commentOut(text string) string {
	var out strings.Builder
	for _, line := range strings.SplitAfter(text, "\n") {
		if line == "" {
			continue
		}
		if line != "\n" && !strings.HasPrefix(line, "#") {
			out.WriteString("# ")
		}
		out.WriteString(line)
	}
	return out.String()
}
