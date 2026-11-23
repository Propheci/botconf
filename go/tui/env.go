package tui

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"strings"
)

// EnvFile is an env file (`KEY=value` lines, as dotenv and shells read
// them): its variables read and set, its other lines kept as they are.
type EnvFile struct {
	lines []envLine
}

type envLine struct {
	// pair: a variable; else raw is a comment, a blank line, anything else.
	pair       bool
	key, value string
	// raw is the line as read, written back unless the value changed.
	raw      string
	rewrites bool
}

// ParseEnv reads an env file.
func ParseEnv(text string) *EnvFile {
	file := &EnvFile{}
	for _, line := range strings.Split(strings.TrimSuffix(text, "\n"), "\n") {
		if text == "" {
			break
		}
		if key, value, ok := parsePair(line); ok {
			file.lines = append(file.lines, envLine{pair: true, key: key, value: value, raw: line})
		} else {
			file.lines = append(file.lines, envLine{raw: line})
		}
	}
	return file
}

// Get is the value of key: its last assignment, as dotenv reads it.
func (f *EnvFile) Get(key string) (string, bool) {
	for index := len(f.lines) - 1; index >= 0; index-- {
		if line := f.lines[index]; line.pair && line.key == key {
			return line.value, true
		}
	}
	return "", false
}

// Set sets key, in place when it is already assigned, else at the end.
func (f *EnvFile) Set(key, value string) {
	for index := len(f.lines) - 1; index >= 0; index-- {
		line := &f.lines[index]
		if line.pair && line.key == key {
			if line.value != value {
				line.value = value
				line.rewrites = true
			}
			return
		}
	}
	f.lines = append(f.lines, envLine{pair: true, key: key, value: value, rewrites: true})
}

// Vars is every variable, its last value.
func (f *EnvFile) Vars() map[string]string {
	vars := map[string]string{}
	for _, line := range f.lines {
		if line.pair {
			vars[line.key] = line.value
		}
	}
	return vars
}

// Render is the file's text.
func (f *EnvFile) Render() string {
	var out strings.Builder
	for _, line := range f.lines {
		if line.pair && line.rewrites {
			out.WriteString(line.key + "=" + quoteEnv(line.value))
		} else {
			out.WriteString(line.raw)
		}
		out.WriteByte('\n')
	}
	return out.String()
}

// parsePair reads `KEY=value`, `export KEY=value`, the value bare (up to a
// ` #` comment), "double quoted" (with `\` escapes) or 'single quoted'.
func parsePair(line string) (string, string, bool) {
	line = strings.TrimLeft(line, " \t")
	if strings.HasPrefix(line, "#") {
		return "", "", false
	}
	line = strings.TrimPrefix(line, "export ")
	key, value, ok := strings.Cut(line, "=")
	key = strings.TrimSpace(key)
	if !ok || key == "" || key[0] >= '0' && key[0] <= '9' {
		return "", "", false
	}
	for _, c := range key {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_') {
			return "", "", false
		}
	}
	value = strings.TrimLeft(value, " \t")
	switch {
	case strings.HasPrefix(value, `"`):
		var out strings.Builder
		runes := []rune(value[1:])
	loop:
		for index := 0; index < len(runes); index++ {
			switch c := runes[index]; c {
			case '"':
				break loop
			case '\\':
				index++
				if index >= len(runes) {
					break loop
				}
				switch runes[index] {
				case 'n':
					out.WriteByte('\n')
				case 't':
					out.WriteByte('\t')
				default:
					out.WriteRune(runes[index])
				}
			default:
				out.WriteRune(c)
			}
		}
		return key, out.String(), true
	case strings.HasPrefix(value, "'"):
		inner, _, _ := strings.Cut(value[1:], "'")
		return key, inner, true
	default:
		if end := strings.Index(value, " #"); end >= 0 {
			value = value[:end]
		}
		return key, strings.TrimRight(value, " \t"), true
	}
}

// quoteEnv writes a value bare when it can be, else double quoted.
func quoteEnv(value string) string {
	bare := value != ""
	for _, c := range value {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' ||
			strings.ContainsRune("-_.:/@+=,", c)) {
			bare = false
			break
		}
	}
	if bare {
		return value
	}
	replacer := strings.NewReplacer(`"`, `\"`, `\`, `\\`, "\n", `\n`, "$", `\$`)
	return `"` + replacer.Replace(value) + `"`
}

// GenerateToken is a new random token: 32 bytes from the OS, as 64 hex
// digits.
func GenerateToken() (string, error) {
	bytes := make([]byte, 32)
	if _, err := rand.Read(bytes); err != nil {
		return "", fmt.Errorf("no randomness: %w", err)
	}
	return hex.EncodeToString(bytes), nil
}
