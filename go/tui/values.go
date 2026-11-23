package tui

import (
	"encoding/json"
	"reflect"
)

// get is the value at path, or nil.
func get(root map[string]any, path []seg) any {
	var value any = root
	for _, s := range path {
		switch current := value.(type) {
		case map[string]any:
			if s.isIndex {
				return nil
			}
			value = current[s.key]
		case []any:
			if !s.isIndex || s.index >= len(current) {
				return nil
			}
			value = current[s.index]
		default:
			return nil
		}
	}
	return value
}

// set sets the value at path, making the tables on the way.
func set(root map[string]any, path []seg, value any) {
	if len(path) > 0 {
		setIn(root, path, value)
	}
}

// setIn sets the value at path below container and gives the container (a
// new table when container was not one).
func setIn(container any, path []seg, value any) any {
	s := path[0]
	switch current := container.(type) {
	case map[string]any:
		if s.isIndex {
			return current
		}
		if len(path) == 1 {
			current[s.key] = value
			return current
		}
		next := current[s.key]
		if next == nil && !path[1].isIndex {
			next = map[string]any{}
		}
		if next != nil {
			current[s.key] = setIn(next, path[1:], value)
		}
		return current
	case []any:
		if !s.isIndex || s.index >= len(current) {
			return current
		}
		if len(path) == 1 {
			current[s.index] = value
		} else {
			current[s.index] = setIn(current[s.index], path[1:], value)
		}
		return current
	default:
		if s.isIndex {
			return container
		}
		return setIn(map[string]any{}, path, value)
	}
}

// remove removes the value at path; it reports whether there was one.
func remove(root map[string]any, path []seg) bool {
	if len(path) == 0 {
		return false
	}
	parentPath, last := path[:len(path)-1], path[len(path)-1]
	parent := get(root, parentPath)
	switch current := parent.(type) {
	case map[string]any:
		if last.isIndex {
			return false
		}
		_, had := current[last.key]
		delete(current, last.key)
		return had
	case []any:
		if !last.isIndex || last.index >= len(current) {
			return false
		}
		set(root, parentPath, append(append([]any(nil), current[:last.index]...), current[last.index+1:]...))
		return true
	}
	return false
}

// asList is a slice of any kind as []any.
func asList(value any) []any {
	if items, ok := value.([]any); ok {
		return items
	}
	reflected := reflect.ValueOf(value)
	if !reflected.IsValid() || reflected.Kind() != reflect.Slice {
		return nil
	}
	items := make([]any, reflected.Len())
	for index := range items {
		items[index] = reflected.Index(index).Interface()
	}
	return items
}

func jsonText(value any) string {
	encoded, err := json.Marshal(value)
	if err != nil {
		return ""
	}
	return string(encoded)
}
