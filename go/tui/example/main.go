// Command example tries the setup screen on a sample program:
//
//	go run ./tui/example /tmp/sample.toml /tmp/sample.env
package main

import (
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/pelletier/go-toml/v2"
	"github.com/propheci/botconf/go/describe"
	"github.com/propheci/botconf/go/tui"
)

type config struct {
	Listen    string          `toml:"listen" doc:"Where the bot's API listens." required:"true" example:"127.0.0.1:8080"`
	TokenEnv  string          `toml:"token_env" doc:"The environment variable holding the token its clients present." env:"token" hint:"SAMPLE_TOKEN"`
	Level     string          `toml:"level" doc:"How much it logs." default:"info" jsonschema:"enum=info,enum=debug,enum=trace"`
	Chats     []chat          `toml:"chats" doc:"The chats it answers in."`
	Notify    []string        `toml:"notify" doc:"Where it sends notices." domain:"sample.chat"`
	Forwards  map[string]rule `toml:"forwards" doc:"Forwarding rules by name."`
	MaxLength uint32          `toml:"max_reply_length" doc:"Longer replies are sent as a file; 0 means no limit." default:"0"`
}

type chat struct {
	ID    string   `toml:"id" doc:"The chat's id." required:"true" domain:"sample.chat"`
	Users []string `toml:"users" doc:"Answer only these users (empty: everyone)."`
}

type rule struct {
	From string `toml:"from" doc:"The chat forwarded." domain:"sample.chat"`
	To   string `toml:"to" doc:"Where it goes." domain:"sample.chat"`
}

// chats are the chats a "linked account" of the sample knows.
var chats = [][2]string{
	{"chat:100", "Family"},
	{"chat:200", "Work"},
	{"chat:300", "Football club"},
	{"chat:400", "Alice"},
}

func suggest(query tui.Query) ([]tui.Suggestion, error) {
	text := strings.ToLower(query.Text)
	var out []tui.Suggestion
	for _, c := range chats {
		if len(out) < query.Limit && (strings.Contains(c[0], text) || strings.Contains(strings.ToLower(c[1]), text)) {
			out = append(out, tui.Suggestion{Value: c[0], Label: c[1] + " (" + c[0] + ")"})
		}
	}
	return out, nil
}

func link(session tui.Session, _ tui.Candidate) (string, error) {
	stopped := errors.New("Stopped.")
	session.Say("Scan this with the app:")
	session.QR("https://example.com/link/sample")
	for step := 1; step <= 3; step++ {
		if session.Cancelled() {
			return "", stopped
		}
		session.Progress(fmt.Sprintf("Waiting for the phone… %d/3", step))
		time.Sleep(500 * time.Millisecond)
	}
	code, ok := session.Ask("The code the app shows:")
	if !ok {
		return "", stopped
	}
	if _, ok := session.AskSecret("Your password:"); !ok {
		return "", stopped
	}
	return fmt.Sprintf("Linked with code %s; %d chats fetched.", code, len(chats)), nil
}

func main() {
	configPath, envPath := "sample.toml", "sample.env"
	if len(os.Args) > 1 {
		configPath = os.Args[1]
	}
	if len(os.Args) > 2 {
		envPath = os.Args[2]
	}
	description, err := describe.Of("Sample bot", "A sample bot's settings.", config{})
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	err = tui.Run(tui.Setup{
		Description: description,
		ConfigPath:  configPath,
		EnvPath:     envPath,
		Validate: func(candidate tui.Candidate) error {
			var parsed config
			return toml.Unmarshal([]byte(candidate.Config), &parsed)
		},
		Suggest: suggest,
		Actions: []tui.Action{{
			Title:       "Link account",
			Description: "A pretend login: shows a QR code, asks for a code and a password.",
			Run:         link,
		}},
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
