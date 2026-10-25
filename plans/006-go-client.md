# Plan 006: The Go module and its settings client

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat v0.5.0..HEAD -- proto/ go/ rust/botconf-grpc`
> Expected: empty (plan 005 tagged `v0.5.0`). What must still hold:
> `proto/botconf/v1/settings.proto` declares `package botconf.v1`,
> `option go_package = "github.com/propheci/botconf/go/settingspb;settingspb"`
> and `service Settings` with `Describe, List, Get, Set, Unset, Add, Remove,
> Reload, Render, Act, Watch` (`Watch` server-streaming); `go/` does not
> exist. If the proto differs, STOP and re-plan the client's method list.

## Status

- **Priority**: P1 (wuphf's Go connectors use it)
- **Effort**: M
- **Risk**: LOW — a client over a fixed contract
- **Depends on**: 005
- **Category**: feature
- **Planned at**: before 001–005 landed, 2026-10-25, against plan 005's proto

## Why this matters

wuphf's connectors are Go programs. They need to read their own settings,
follow changes (`Watch`), and show the settings panel (`Render`/`Act`) on
their platform. A typed Go client in botconf, generated from the same proto
the Rust server is built from, keeps both sides on one contract, and the
panel renderers of plan 007 build on it.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | The Go SDK is a client (plus renderers in 007); the store stays in the host (Rust). A Go port of the store is only a roadmap item (008). |
| 2 | The module lives in `go/`, path `github.com/propheci/botconf/go`. Tags `go/v0.x.y` on the same commit as the Rust `v0.x.y`. |
| 3 | Git-based versions only (no proxy release beyond what `go get` does from tags). |

Decided while planning (flag to the owner; reviewers may push back):

- **Versions match wuphf's Go side**: Go 1.25, `google.golang.org/grpc
  v1.76.0`, `google.golang.org/protobuf v1.36.11`, generated with protoc
  34.1, protoc-gen-go 1.36.11 and protoc-gen-go-grpc 1.6.2 (what wuphf's
  nix shell provides), so a connector's module graph does not need two
  versions.
- **Generated code is committed** in `go/settingspb`, like wuphf's
  `proto/go`. CI regenerates and fails on a diff.
- **Credentials are options, not policy**: `WithBearer(token)` adds
  `authorization: Bearer <token>` (what wuphf uses); `WithDialOptions(…)`
  passes anything else through. The client never decides who may do what;
  the host's authorizer does.
- **`Watch` reconnects by itself** with capped exponential backoff and
  delivers a fresh initial event after each reconnect, so callers handle one
  kind of event ("here are the current values") and never miss state.
- **Question keeping** for panels lives here (`settings.Questions`), not in
  each renderer: one pending question per (conversation, user), expiring
  after 10 minutes, mirroring `botconf-telegram`'s `Prompts`.

## Current state

- Plan 005's proto (see its Step 1) and server (`rust/botconf-grpc`), with an
  in-process test harness.
- wuphf's Go code to copy idioms from (read for style, do not depend on it):
  `connectors/shared/functions/client.go` (`Dial` with a bearer token from an
  env var, `context` deadlines) and `connectors/shared/coreclient/client.go`
  (reconnect loop with backoff). Generation command used there:

  ```sh
  protoc --go_out=../../proto/go --go_opt=module=github.com/akshettrj/wuphf/proto/go \
    --go-grpc_out=../../proto/go --go-grpc_opt=module=github.com/akshettrj/wuphf/proto/go …
  ```

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Generate | `scripts/gen-go.sh` (Step 1) | exit 0; no diff on a clean tree |
| Build | `cd go && go build ./... && go vet ./...` | exit 0 |
| Tests | `cd go && go test -race ./...` | exit 0 |
| Format | `cd go && test -z "$(gofmt -l .)"` | exit 0 |
| Cross-language | `cd go && go test -tags integration ./settings/...` | exit 0 (builds and runs the Rust toy server) |

## Scope

**In scope**: `go/**` (create), `scripts/gen-go.sh` (create), `flake.nix`
(Go and protoc plugins in the dev shell), `rust/botconf-grpc/examples/toy_server.rs`
(create, for the cross-language test), `.github/workflows/ci.yml` (a Go
job), README, plans index.

**Out of scope**: panel renderers (007); changing the proto (if the client
needs a proto change, STOP); the store in Go (008).

## Git workflow

On `main`:
1. `feat(go): generate the botconf.v1 bindings`
2. `feat(go): add the settings client`
3. `test: run the Go client against the Rust server`
4. `ci: build and test the Go module`
Tag `v0.6.0` and `go/v0.6.0` on the same commit.

## Steps

### Step 1: Module and bindings

- `go/go.mod`: `module github.com/propheci/botconf/go`, `go 1.25.0`, requiring
  `google.golang.org/grpc v1.76.0` and `google.golang.org/protobuf v1.36.11`.
- `flake.nix` dev shell: add `go`, `protobuf`, `protoc-gen-go`,
  `protoc-gen-go-grpc` (check the versions with `protoc --version` etc.; if
  nixpkgs gives different ones than listed above, use them and note it).
- `scripts/gen-go.sh`:

  ```sh
  #!/usr/bin/env bash
  # Regenerates go/settingspb from proto/. Run from the repository root.
  set -euo pipefail
  protoc --proto_path=proto \
    --go_out=go --go_opt=module=github.com/propheci/botconf/go \
    --go-grpc_out=go --go-grpc_opt=module=github.com/propheci/botconf/go \
    proto/botconf/v1/settings.proto
  ```

**Verify**: `scripts/gen-go.sh` creates `go/settingspb/settings.pb.go` and
`settings_grpc.pb.go`; `cd go && go build ./...` → exit 0. Commit 1.

### Step 2: The client (`go/settings`)

```go
// Package settings is a client for a program's runtime settings, served by
// botconf-grpc: read and change them, follow changes, and drive the
// settings panel.
package settings

type Client struct{ /* conn, api settingspb.SettingsClient */ }

type Option func(*options)
func WithBearer(token string) Option
func WithDialOptions(opts ...grpc.DialOption) Option
func WithActor(domain, id string) Option          // default actor for calls without one

func Dial(address string, opts ...Option) (*Client, error) // insecure transport by default (local binds); TLS via WithDialOptions
func (c *Client) Close() error

// An actor a request acts for (claimed; the host decides).
type Actor struct{ Domain, ID string }

// Values are raw JSON.
type Value struct {
	Key    string
	JSON   json.RawMessage // nil: no value
	Source Source
	SourceName string
}

func (c *Client) Describe(ctx context.Context, actor *Actor) (*settingspb.DescribeResponse, error)
func (c *Client) List(ctx context.Context, actor *Actor) (*settingspb.ListResponse, error)
func (c *Client) Get(ctx context.Context, actor *Actor, key string) (Value, error)
func (c *Client) Set(ctx context.Context, actor *Actor, key, value string) (*settingspb.ChangeResponse, error)
func (c *Client) Unset(…) / Add(…) / Remove(…) / Reload(…)

// Panel
func (c *Client) Render(ctx context.Context, actor *Actor, page Page, limits *settingspb.Limits) (*settingspb.Screen, error)
func (c *Client) Press(ctx context.Context, actor *Actor, limits *settingspb.Limits, token string) (*settingspb.ActResponse, error)
func (c *Client) Answer(ctx context.Context, actor *Actor, limits *settingspb.Limits, question string, input Input) (*settingspb.ActResponse, error)
func (c *Client) Command(ctx context.Context, actor *Actor, limits *settingspb.Limits, text string) (*settingspb.ActResponse, error)

type Page struct{ /* Home, Section(id) or Token(token) */ }
func Home() Page; func Section(id string) Page; func PageToken(token string) Page
type Input struct{ /* Text(string) | Picked(domain, ids []json.RawMessage) | Cancel */ }

// Errors: gRPC statuses become typed errors callers can test with errors.Is/As.
var ErrNotFound, ErrPermissionDenied, ErrUnavailable error
type InvalidValueError struct{ Reason string }
```

`Watch`:

```go
type Event struct {
	Values         []Value
	Initial        bool // true after every (re)connect: Values holds everything under the prefixes
	CatalogChanged bool
}

// Watch follows the keys under prefixes until ctx ends. Events arrive in
// order on the returned channel, which closes when ctx ends. Reconnects
// with backoff (1s doubling to 30s); each reconnect starts with an Initial event.
func (c *Client) Watch(ctx context.Context, actor *Actor, prefixes ...string) <-chan Event

// Decode assembles the values under prefix into a JSON object and unmarshals
// it into out, e.g. a connector's own settings struct.
func Decode(values []Value, prefix string, out any) error
```

`Questions` (for renderers):

```go
// Questions keeps the panel's pending question per conversation and user.
type Questions struct{ /* mu, map[key]entry, ttl */ }
func NewQuestions(ttl time.Duration) *Questions
func (q *Questions) Ask(conversation, user string, ask *settingspb.Ask, panelMessage string)
func (q *Questions) Pending(conversation, user string) (question string, panelMessage string, ok bool)
func (q *Questions) Finish(conversation, user string)
```

### Step 3: Tests

`go/settings/settings_test.go` with a fake `settingspb.SettingsServer`
over `bufconn` (record requests; scripted responses):

1. `TestBearerTokenIsSent` — metadata has `authorization: Bearer t`.
2. `TestDefaultActorIsUsed` and an explicit actor overrides it.
3. `TestStatusesBecomeTypedErrors` — `NOT_FOUND`, `INVALID_ARGUMENT` (reason
   preserved), `PERMISSION_DENIED`, `UNAVAILABLE`.
4. `TestWatchReconnectsWithAnInitialEvent` — the fake ends the stream after
   one change; the client reconnects (fake clock or a short backoff injected
   through an unexported option; no real sleeps over 50 ms) and the next
   event is `Initial`.
5. `TestWatchStopsWithContext` — channel closes after cancel.
6. `TestDecodeBuildsNestedObjects` — values `a.limit=5`, `a.mode="x"`,
   `a.nested.k=true` under prefix `a` decode into a struct.
7. `TestQuestionsExpire` — injected clock.
8. `TestActVariants` — `Press`, `Answer` (text, picked, cancel) and
   `Command` fill the right `oneof`.

Commit 2.

### Step 4: Cross-language test

- `rust/botconf-grpc/examples/toy_server.rs`: the toy schema from plan 005's
  tests with `MemoryStorage`, an authorizer allowing everything, listening on
  `--listen` (default `127.0.0.1:0`) and printing `listening on <addr>` on
  stdout.
- `go/settings/integration_test.go` (`//go:build integration`): `cargo build
  -p botconf-grpc --example toy_server`, start it, read the address, then:
  `Describe` has the toy's settings; `Set` a valid value and an invalid one
  (typed error with the server's reason); `Watch` sees the change; render
  home and press a toggle token.

**Verify**: `cd go && go test -tags integration ./settings/...` → exit 0.
Commit 3.

### Step 5: CI and docs

`.github/workflows/ci.yml`, a `go` job: checkout; install Go 1.25 and the
pinned protoc + plugins (`arduino/setup-protoc` and `go install
google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.11
google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.6.2`), run
`scripts/gen-go.sh && git diff --exit-code go/settingspb`, `gofmt -l`,
`go vet ./...`, `go test -race ./...`, and the integration tests (needs a
Rust toolchain: add `dtolnay/rust-toolchain@stable`).

README: "Go" section (install with `go get
github.com/propheci/botconf/go@go/v0.6.0`, a `Dial` + `Watch` + `Decode`
example). Commit 4; tag `v0.6.0` and `go/v0.6.0`.

## Test plan

The 8 unit tests and the cross-language test above.

## Done criteria

- [ ] `go/settingspb` regenerates without a diff
- [ ] `go vet`, `gofmt`, `go test -race ./...` and the integration test pass
- [ ] CI has the Go job
- [ ] Tags `v0.6.0` and `go/v0.6.0` on the same commit, locally
- [ ] README and `plans/README.md` updated

## STOP conditions

- The proto needs a change for the client (report what and why).
- Regeneration with the pinned tool versions differs from the committed
  files on a clean tree.
- A test needs real sleeps longer than 50 ms.

## Maintenance notes

- Plan 007's renderers depend on `Client`, `Questions` and the panel types.
- If wuphf's Go side moves to a newer grpc or protobuf, bump them here in the
  same change set.
