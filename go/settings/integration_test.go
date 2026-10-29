//go:build integration

package settings

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/propheci/botconf/go/settingspb"
)

// toyServer builds and starts botconf-grpc's toy_server example and returns
// its address. It needs cargo, and protoc for botconf-grpc's build script.
func toyServer(t *testing.T) string {
	t.Helper()
	root, err := filepath.Abs("../..")
	if err != nil {
		t.Fatal(err)
	}
	build := exec.Command("cargo", "build", "--quiet", "-p", "botconf-grpc", "--features", "testing",
		"--example", "toy_server", "--message-format=json-render-diagnostics")
	build.Dir = root
	build.Stderr = os.Stderr
	output, err := build.Output()
	if err != nil {
		t.Fatalf("build toy_server: %v", err)
	}
	executable := ""
	for _, line := range strings.Split(string(output), "\n") {
		var message struct {
			Reason     string  `json:"reason"`
			Executable *string `json:"executable"`
		}
		if json.Unmarshal([]byte(line), &message) == nil && message.Reason == "compiler-artifact" && message.Executable != nil {
			executable = *message.Executable
		}
	}
	if executable == "" {
		t.Fatal("cargo reported no toy_server executable")
	}

	server := exec.Command(executable, "--listen", "127.0.0.1:0")
	server.Stderr = os.Stderr
	stdout, err := server.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := server.Start(); err != nil {
		t.Fatalf("start toy_server: %v", err)
	}
	t.Cleanup(func() {
		_ = server.Process.Kill()
		_ = server.Wait()
	})
	line, err := bufio.NewReader(stdout).ReadString('\n')
	if err != nil {
		t.Fatalf("read toy_server's address: %v", err)
	}
	address, ok := strings.CutPrefix(strings.TrimSpace(line), "listening on ")
	if !ok {
		t.Fatalf("toy_server said %q", line)
	}
	return address
}

func button(t *testing.T, screen *settingspb.Screen, label string) string {
	t.Helper()
	var labels []string
	for _, row := range screen.GetRows() {
		for _, button := range row.GetButtons() {
			if strings.Contains(button.GetLabel(), label) {
				return button.GetToken()
			}
			labels = append(labels, button.GetLabel())
		}
	}
	t.Fatalf("no %q button in %q", label, labels)
	return ""
}

func TestAgainstTheRustServer(t *testing.T) {
	client, err := Dial(toyServer(t), WithActor("telegram.user", "42"))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	description, err := client.Describe(ctx, nil)
	if err != nil {
		t.Fatalf("Describe() error = %v", err)
	}
	var keys []string
	for _, setting := range description.GetSettings() {
		keys = append(keys, setting.GetKey())
	}
	if strings.Join(keys, ",") != "greeting,limit,disabled" {
		t.Fatalf("settings = %v", keys)
	}

	events := client.Watch(ctx, nil, "limit")
	if event := <-events; !event.Initial || string(event.Values[0].JSON) != "5" {
		t.Fatalf("initial event = %+v", event)
	}

	if _, err := client.Set(ctx, nil, "limit", "7"); err != nil {
		t.Fatalf("Set(limit, 7) error = %v", err)
	}
	if event := <-events; event.Initial || string(event.Values[0].JSON) != "7" {
		t.Fatalf("change event = %+v", event)
	}

	_, err = client.Set(ctx, nil, "limit", "50")
	var invalid *InvalidValueError
	if !errors.As(err, &invalid) || !strings.Contains(invalid.Reason, "1 to 10") {
		t.Fatalf("Set(limit, 50) error = %v, want the server's reason", err)
	}
	if _, err := client.Get(ctx, nil, "nope"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("Get(nope) error = %v, want ErrNotFound", err)
	}

	home, err := client.Render(ctx, nil, Home(), View{})
	if err != nil {
		t.Fatalf("Render(home) error = %v", err)
	}
	features, err := client.Press(ctx, nil, View{}, button(t, home, "Features"))
	if err != nil {
		t.Fatalf("Press(Features) error = %v", err)
	}
	if _, err := client.Press(ctx, nil, View{}, button(t, features.GetScreen(), "Lamp")); err != nil {
		t.Fatalf("Press(Lamp) error = %v", err)
	}
	disabled, err := client.Get(ctx, nil, "disabled")
	if err != nil || string(disabled.JSON) != `["lamp"]` {
		t.Fatalf("disabled = %s (error %v), want [\"lamp\"]", disabled.JSON, err)
	}
}
