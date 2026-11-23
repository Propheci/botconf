package describe

import (
	"reflect"
	"strings"
	"testing"

	"github.com/pelletier/go-toml/v2"
	"github.com/propheci/botconf/go/settingspb"
	"google.golang.org/protobuf/proto"
)

// file is a connector-like configuration using every tag.
type file struct {
	CoreAddress  string              `toml:"core_address" doc:"The core's gRPC address." required:"true" example:"127.0.0.1:50051"`
	CoreTokenEnv string              `toml:"core_token_env" doc:"The variable holding the core token." env:"token" hint:"WUPHF_TOKEN"`
	BotToken     string              `toml:"bot_token" secret:"true"`
	Backfill     *bool               `toml:"backfill_on_start" default:"true"`
	Chats        []string            `toml:"chats" title:"Watched chats"`
	Skipped      string              `toml:"-"`
	hidden       string              //nolint:unused // unexported: not described
	Panel        panel               `toml:"panel"`
	Providers    []provider          `toml:"providers"`
	Overrides    map[string]override `toml:"overrides"`
	runtime
}

type panel struct {
	Command string `toml:"command" doc:"The panel's command." default:"settings"`
}

type provider struct {
	ID string `toml:"id" required:"true"`
}

type override struct {
	Max uint32 `toml:"max"`
}

type runtime struct {
	LogLevel  string    `toml:"log_level" botconf:"Log level" doc:"How much it logs" jsonschema:"enum=debug,enum=info"`
	TimeoutMs uint32    `toml:"timeout_ms" botconf:"Timeout (ms)" doc:"0 means 20000"`
	Rendering rendering `toml:"rendering"`
}

type rendering struct {
	AsFiles bool   `toml:"as_files" botconf:"As files" doc:"Every attachment as a file"`
	Limit   uint16 `toml:"limit" botconf:"Limit" jsonschema:"minimum=1,maximum=100"`
}

func describeFile(t *testing.T) *Description {
	t.Helper()
	described, err := Of("Fixture", "A fixture.", file{})
	if err != nil {
		t.Fatal(err)
	}
	return described
}

func entry(t *testing.T, table *Table, key string) Entry {
	t.Helper()
	for _, entry := range table.Entries {
		if entry.Key == key {
			return entry
		}
	}
	t.Fatalf("no %s", key)
	return Entry{}
}

func TestTagsDescribeEntries(t *testing.T) {
	described := describeFile(t)
	var keys []string
	for _, entry := range described.Table.Entries {
		keys = append(keys, entry.Key)
	}
	want := []string{
		"core_address", "core_token_env", "bot_token", "backfill_on_start", "chats", "panel",
		"providers", "overrides", "log_level", "timeout_ms", "rendering",
	}
	if !reflect.DeepEqual(keys, want) {
		t.Fatalf("keys %v, want %v", keys, want)
	}
	address := entry(t, &described.Table, "core_address")
	if !address.Required || address.Example != "127.0.0.1:50051" || address.Title != "Core address" {
		t.Errorf("core_address: %+v", address)
	}
	token := entry(t, &described.Table, "core_token_env")
	if token.Node.Value.Kind != EnvVar || token.Node.Value.Env != EnvToken || token.Hint != "WUPHF_TOKEN" {
		t.Errorf("core_token_env: %+v", token)
	}
	if entry(t, &described.Table, "bot_token").Node.Value.Kind != Secret {
		t.Error("bot_token is a secret")
	}
	backfill := entry(t, &described.Table, "backfill_on_start")
	if !backfill.Nullable || backfill.Default != true {
		t.Errorf("backfill_on_start: %+v", backfill)
	}
	chats := entry(t, &described.Table, "chats")
	if chats.Node.Value.Kind != TextList || chats.Title != "Watched chats" {
		t.Errorf("chats: %+v", chats)
	}
	if level := entry(t, &described.Table, "log_level"); !reflect.DeepEqual(level.Node.Value.Choices, []string{"debug", "info"}) {
		t.Errorf("log_level: %+v", level)
	}
	providers := entry(t, &described.Table, "providers")
	if providers.Node.List == nil || providers.Node.List.Table == nil {
		t.Errorf("providers: %+v", providers.Node)
	}
	overrides := entry(t, &described.Table, "overrides")
	if overrides.Node.Map == nil || overrides.Node.Map.Table == nil {
		t.Errorf("overrides: %+v", overrides.Node)
	}
}

// The runtime settings are what wuphf's connectors declared from the same
// tags before this package.
func TestRuntimeSettings(t *testing.T) {
	got := describeFile(t).RuntimeSettings()
	text := func(kind *settingspb.Kind) string { return kind.String() }
	want := []*settingspb.SettingDescriptor{
		{Key: "log_level", Title: "Log level", Description: "How much it logs", Kind: &settingspb.Kind{
			Kind: &settingspb.Kind_OneOf{OneOf: &settingspb.OneOfKind{Choices: []*settingspb.Choice{
				{Value: "debug", Label: "debug"}, {Value: "info", Label: "info"},
			}}},
		}},
		{Key: "timeout_ms", Title: "Timeout (ms)", Description: "0 means 20000", Kind: &settingspb.Kind{
			Kind: &settingspb.Kind_Number{Number: &settingspb.NumberKind{Min: 0, Max: 1<<32 - 1}},
		}},
		{Key: "rendering.as_files", Title: "As files", Description: "Every attachment as a file", Kind: &settingspb.Kind{
			Kind: &settingspb.Kind_OneOf{OneOf: &settingspb.OneOfKind{Choices: []*settingspb.Choice{
				{Value: "true", Label: "✅ Yes"}, {Value: "false", Label: "❌ No"},
			}}},
		}},
		{Key: "rendering.limit", Title: "Limit", Kind: &settingspb.Kind{
			Kind: &settingspb.Kind_Number{Number: &settingspb.NumberKind{Min: 1, Max: 100}},
		}},
	}
	if len(got) != len(want) {
		t.Fatalf("%d settings, want %d", len(got), len(want))
	}
	for index := range want {
		if !proto.Equal(got[index], want[index]) {
			t.Errorf("setting %d: %v, want %v", index, text(got[index].Kind)+" "+got[index].Key, want[index])
		}
	}
}

func TestExample(t *testing.T) {
	example := describeFile(t).Example()
	for _, part := range []string{
		"# Fixture\n#\n# A fixture.\n",
		"\n# The core's gRPC address.\n# Required.\ncore_address = \"127.0.0.1:50051\"\n",
		"\n# The variable holding the core token.\n# core_token_env = \"WUPHF_TOKEN\"\n",
		"\n# Default: true.\n# backfill_on_start = true\n",
		"\n# How much it logs\n# One of: debug, info.\n# log_level = \"debug\"\n",
		"\n[panel]\n\n# The panel's command.\n# Default: \"settings\".\n# command = \"settings\"\n",
		"\n# [[providers]]\n",
		"\n# [overrides.<name>]\n",
		"\n[rendering]\n",
	} {
		if !strings.Contains(example, part) {
			t.Errorf("no %q in\n%s", part, example)
		}
	}
	for _, line := range strings.Split(example, "\n") {
		if len(line) > width {
			t.Errorf("too long: %q", line)
		}
	}
}

func TestRenderedValuesReadBack(t *testing.T) {
	values := map[string]any{
		"core_address":   "0.0.0.0:1",
		"core_token_env": "TOKEN",
		"chats":          []any{"a", "b \"c\""},
		"panel":          map[string]any{"command": "cfg"},
		"providers":      []any{map[string]any{"id": "p1"}, map[string]any{"id": "p2"}},
		"overrides":      map[string]any{"a.b": map[string]any{"max": int64(3)}},
		"log_level":      "info",
		"rendering":      map[string]any{"as_files": true, "limit": int64(5)},
	}
	rendered := describeFile(t).Render(values)
	var parsed file
	if err := toml.Unmarshal([]byte(rendered), &parsed); err != nil {
		t.Fatalf("%v\n%s", err, rendered)
	}
	if parsed.CoreAddress != "0.0.0.0:1" || parsed.Chats[1] != "b \"c\"" || len(parsed.Providers) != 2 ||
		parsed.Overrides["a.b"].Max != 3 || parsed.Rendering.Limit != 5 || !parsed.Rendering.AsFiles ||
		parsed.Panel.Command != "cfg" {
		t.Errorf("read back %+v from\n%s", parsed, rendered)
	}
	if !strings.Contains(rendered, "[overrides.\"a.b\"]\n") {
		t.Errorf("a name that is not a bare key is quoted:\n%s", rendered)
	}
}

func TestUnknownKeysAreKeptAsComments(t *testing.T) {
	rendered := describeFile(t).Render(map[string]any{
		"stray": int64(1),
		"panel": map[string]any{"old": "y"},
	})
	want := "# Not in this program's description (kept, but it ignores them):\n# panel.old = \"y\"\n# stray = 1\n"
	if !strings.HasSuffix(rendered, want) {
		t.Errorf("no unknown keys in\n%s", rendered)
	}
}

func TestBadTagsAreRefused(t *testing.T) {
	type bad struct {
		Count int `toml:"count" default:"many"`
	}
	if _, err := Of("Bad", "", bad{}); err == nil || !strings.Contains(err.Error(), "count: default tag") {
		t.Errorf("err = %v", err)
	}
}

func TestTitles(t *testing.T) {
	if got := TitleOf("max_message_length"); got != "Max message length" {
		t.Error(got)
	}
}

func TestAnOptionalTableLeftEmptyStaysAbsent(t *testing.T) {
	type nested struct {
		Deep bool `toml:"deep"`
	}
	type inner struct {
		ProviderID string `toml:"provider_id" doc:"Who serves."`
		Nested     nested `toml:"nested"`
	}
	type outer struct {
		Functions *inner `toml:"functions"`
	}
	described := MustOf("Outer", "", outer{})
	for _, values := range []map[string]any{{}, {"functions": map[string]any{}}} {
		rendered := described.Render(values)
		if !strings.Contains(rendered, "# [functions]\n") || !strings.Contains(rendered, "# [functions.nested]\n") {
			t.Fatalf("not commented out:\n%s", rendered)
		}
		var parsed outer
		if err := toml.Unmarshal([]byte(rendered), &parsed); err != nil || parsed.Functions != nil {
			t.Fatalf("read back %+v, %v from\n%s", parsed.Functions, err, rendered)
		}
	}
	rendered := described.Render(map[string]any{"functions": map[string]any{"provider_id": "p"}})
	var parsed outer
	if err := toml.Unmarshal([]byte(rendered), &parsed); err != nil || parsed.Functions == nil ||
		parsed.Functions.ProviderID != "p" {
		t.Fatalf("read back %+v, %v from\n%s", parsed.Functions, err, rendered)
	}
}

func TestADomainMakesAReference(t *testing.T) {
	type runtime struct {
		Notify []string `toml:"notify" botconf:"Notify" domain:"app.chat"`
		Home   string   `toml:"home" botconf:"Home" domain:"app.chat"`
	}
	described := MustOf("R", "", runtime{})
	if described.Table.Entries[0].Domain != "app.chat" {
		t.Fatal("no domain")
	}
	settings := described.RuntimeSettings()
	for index, many := range []bool{true, false} {
		ref := settings[index].Kind.GetRef()
		if ref == nil || ref.Domain != "app.chat" || ref.Many != many {
			t.Errorf("setting %d: %v", index, settings[index].Kind)
		}
	}
}
