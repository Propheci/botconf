# botconf

Runtime-editable, typed settings for bots and other long-running programs.

A program describes its configuration once, as a typed struct. botconf builds
the effective configuration by layering, from the lowest to the highest
priority:

1. **the config file** (e.g. TOML),
2. **the environment** (e.g. `MYBOT_TELEGRAM__TOKEN`),
3. **stored overrides**: the keys declared as runtime settings, changed while
   the program runs and kept in a database.

Every change is deserialized and validated exactly like the file, then
atomically replaces the snapshot readers see, so it applies immediately.
Invalid changes are rejected with the reason. Each runtime setting declares
a *kind* (text, number, one of some choices, a list of users, a form, a
map, ...), so front ends can offer a fitting editor: a chat settings panel
with buttons, a command line, or a remote client.

## Layout

```
rust/      the Rust crates
  botconf/            the core: schema, layering, store, storage, commands
  botconf-panel/      the settings panel engine: screens, buttons, answers
  botconf-grpc/       the botconf.v1.Settings gRPC service (tonic)
  botconf-telegram/   a Telegram (teloxide) front end for the panel
proto/     the wire contract (botconf.v1.Settings), shared by Rust and Go
go/        the Go module: a client for remote settings and panel renderers
plans/     implementation plans (see plans/README.md)
```

## Status

The Rust crates live in `rust/`. They moved here from
[assistant_bot_rs](https://github.com/akshettrj/assistant_bot_rs) with their
history ([plan 001](plans/001-extract-crates.md)); their earlier life inside
that bot's `src/settings/` stays in its repository. The panel engine
(`botconf-panel`) landed in plan 004 and the gRPC service (`botconf-grpc`) in
plan 005, and the Go client (`go/settings`) in plan 006; the Go panel
renderers follow in plan 007.
[`plans/README.md`](plans/README.md) has the order and the status of each.

Develop inside the Nix dev shell (`nix develop`, or `direnv allow`), which
provides the toolchain, a nightly rustfmt (`rustfmt.toml` uses unstable
options), taplo, protoc (`botconf-grpc` compiles `proto/`), and Go with the
protoc plugins for the Go module.
`nix flake check` runs clippy, the tests and the format checks, as CI does.

## Using it

Until the API settles, botconf is used as a git dependency pinned to a tag,
not from crates.io or a Go proxy release:

```toml
# Cargo.toml
# The core. Features: `sea-orm` (keep the overrides in a SQL table with
# SeaORM) and `cli` (a clap subcommand to view and change the settings).
botconf = { git = "https://github.com/propheci/botconf", tag = "v0.6.0", features = ["cli", "sea-orm"] }
# The settings panel engine, for writing a front end. Feature: `sea-orm`
# (keep the handles of long buttons in a SQL table).
botconf-panel = { git = "https://github.com/propheci/botconf", tag = "v0.6.0" }
# A Telegram settings panel (teloxide) over a botconf store, drawn by the
# engine. Feature: `sea-orm` (as above).
botconf-telegram = { git = "https://github.com/propheci/botconf", tag = "v0.6.0", features = ["sea-orm"] }
# The gRPC service (tonic 0.12) serving a store and its panel to remote
# clients. Building it needs `protoc` on the PATH.
botconf-grpc = { git = "https://github.com/propheci/botconf", tag = "v0.6.0" }
```

Minimum Rust: 1.95 for `botconf`, `botconf-panel` and `botconf-grpc`, 1.96
for `botconf-telegram` (its teloxide pulls in takecell 0.1.2, which declares
1.96). The dependency requirements
are floors, and botconf turns on only the features it uses, so a host keeps
its own lock file; for example, enable serde_json's `preserve_order` in the
host if you want JSON objects to keep their written order.

```sh
# Go: the settings client (see "Go" below)
go get github.com/propheci/botconf/go@go/v0.6.0
```

### Reference kinds

Settings that name things outside the configuration (users, chats,
accounts) use `Kind::Ref { domain, many }`. The **domain** says what the ids
refer to, so that each front end knows how to pick them and how to name
them; botconf itself never interprets the ids, and stores whatever JSON the
host's config type reads.

- `botconf-telegram` knows two domains, `botconf_telegram::domains::USER`
  (`telegram.user`) and `CHAT` (`telegram.chat`). Their ids are numbers,
  picked with Telegram's user and chat pickers or typed (`@username` for
  users, through `Names::resolve`).
- Ids of any other domain (for example a host's `wuphf.chat`) are typed: the
  panel asks the host's `Names::resolve(domain, text)` first, then reads the
  text as JSON or as a string (`"42"`, quoted, stays a string). The panel
  names them through `Names::name(bot, domain, id)` and shows the id when
  there is no name.

```rust
RuntimeSetting::new("admins", "Who's in charge")
    .kind(Kind::reference(botconf_telegram::domains::USER, true))
```

Who made a change is recorded as a string (`StoredOverride.by`), since not
every platform's user ids are numbers. The SeaORM table's `updated_by`
column is `text`. Tables made before `v0.2.0` declare it `bigint`: on SQLite
they keep loading (botconf casts the column to text), and a migration to
`text` is still worth doing; on Postgres they need one
(`ALTER TABLE settings ALTER COLUMN updated_by TYPE text USING updated_by::text`).

### Declaring settings

Descriptors (`Kind`, `Form`, `Field`, `Choices`, `FixedChoice`,
`RuntimeSetting`) hold `Shared` data: a `'static` reference for settings
declared in the code, or an `Arc` for settings described at runtime (see
below). Compiled-in settings stay `const`, through `const fn` constructors:

```rust
const LEVELS: &[FixedChoice] = &[FixedChoice::new("info", "info"), FixedChoice::new("debug", "debug")];
const FIELDS: &[Field] = &[
    Field::new("brightness", "Brightness", Kind::number(1, 100, "%", &[25, 50, 100], true)),
    Field::new("white", "White", Kind::one_of(Choices::fixed(LEVELS), true, true)).group("look"),
];
// A nested reference to a descriptor is not promoted to `'static` (descriptors
// have drop glue), so a form or a kind used by a map gets its own `const`.
const PRESET_FORM: &Form = &Form::new(FIELDS, r#"{"brightness": 100}"#);
const PRESET: &Kind = &Kind::form(PRESET_FORM);

const SETTINGS: &[RuntimeSetting] = &[
    RuntimeSetting::new("level", "How much to log")
        .titled("Logging")
        .kind(Kind::one_of(Choices::fixed(LEVELS), true, false)),
    RuntimeSetting::per_entry("presets", "Named looks", PRESET),
];
```

Settings described at runtime are built with `RuntimeSetting::owned` or by
setting the fields (`Kind::Ref { domain: id.into(), many }`, ...); the
builders (`titled`, `kind`, ...) are meant for `const` declarations.

### Sections registered at runtime

A host whose settings are partly declared by someone else while it runs (a
bridge whose connectors each bring their own settings, say) lists the key
prefixes where that happens in `Schema::dynamic_namespaces`, e.g.
`connectors`, and registers each part as a `DynamicSection` when it
arrives:

```rust
store.register_section(DynamicSection {
    id: "connector:telegram-1".into(),
    path: "connectors.telegram-1".into(),
    title: "Telegram".into(),
    description: "The Telegram connector".into(),
    settings: vec![RuntimeSetting::owned("rendering.max_length", "Longest message", Kind::number(1, 4096, "", &[], false))],
    validator: Arc::new(json_schema_validator), // any `Validator`
    base: connector_values,                      // the connector's own config
    base_source: "connector telegram-1".into(),
}).await?;
```

- **Validation.** The section is checked by its `Validator`
  (`fn validate(&self, &Value) -> Result<Value, String>`, closures included)
  instead of a Rust type: for example a JSON Schema the section's owner sent.
  botconf ships no JSON Schema implementation; plug one in.
- **Base values.** The section's values below the overrides come with it
  (`base`, shown as `base_source`), not from the host's config file, and its
  keys never reach the host's configuration type.
- **Waiting overrides.** A stored override below a dynamic namespace whose
  section is not registered (the connector is offline at startup, or was
  removed) waits for it instead of being ignored: `Snapshot::parked()`, and
  listed as "Waiting for their section". When the section registers, the
  waiting overrides apply one at a time; an invalid one is ignored with the
  reason, as at load. `unset` clears waiting overrides too.
- **Positions.** The catalog only grows. Registering a section again with
  unchanged settings keeps their positions; changed or removed settings
  leave tombstones, so a front end holding an old position finds nothing
  rather than another setting.
- `remove_section(id)` drops the section; its stored overrides wait for it to
  come back.
- `dynamic_sections()` lists them, e.g. for a settings panel.

### Writing a front end

`botconf-panel` computes the settings panel once, as platform-neutral
screens: a home screen, one per section (registered ones included), and one
per setting with an editor fitting its kind. A front end only draws them and
reports what the user did:

1. Build a `Panel` over the store, and register what the front end knows
   about reference domains with `Panel::domain` (how ids are typed, counted
   and asked for, and whether it has a picker for them).
2. For each user, build a `Viewer`: who is acting (recorded as the author of
   changes), the front end's `Limits` (the longest button token, the longest
   message, how many rows and buttons per row fit), its `Names` for
   references, and whether its pickers work in this conversation.
3. Draw `Panel::render`'s `Screen`: its `Text` is a list of styled spans
   (plain, bold, italic, code, code block) for the front end to turn into
   its own markup; its rows are buttons, each with a label and a token.
4. Send a pressed button's token back to `Panel::press` unread, and act on the
   `Reaction`: redraw the panel with `screen`, post `post` as a new message,
   show `notice`, delete the panel when `closed`, or put the `Ask` to the
   user.
5. Keep the `Ask`'s `Question` until the user answers, then pass the answer to
   `Panel::answer` as an `Input` (typed text, ids picked with a picker, or a
   cancel). The engine keeps no conversation state; `Question::encode` gives a
   compact form for front ends that keep it as text.
6. On `PanelError::Stale`, show the notice and redraw the panel with the
   screen it carries; on `PanelError::Rejected`, show the reason and keep the
   panel.

`Panel::command_text` handles the text form of the command (`list`, `get`,
`set`, ...). Screens longer than `Limits::max_rows` are cut into parts with
◀ / ▶ buttons. `botconf-telegram` is the reference front end.

### Tokens

A button's token is the stateless encoding of its action (e.g. `o:h`,
`t:0:1`) when that fits the front end's `Limits::token_bytes`, and otherwise a
handle: `~` and 10 characters, kept in a `TokenStore` with the action it
stands for. Actions name the exact values they act on (a picked choice from
a computed list, a list item to remove), never a position in a list that may
have changed, so a press can't act on the wrong value; if the value is gone,
the press is `Stale` and the panel redraws.

The same action always gets the same handle, so redrawing a screen renews it
rather than adding one. Handles expire after 30 days without being drawn
(`Panel::token_ttl`), and expired ones are purged at most hourly while new
ones are stored (or with `Panel::purge_tokens`). An expired or unknown
handle redraws the home screen with "This panel is out of date".

`Panel::new` keeps handles in memory, so they stop working when the program
restarts. A host whose ids are long (most buttons then need handles) should
keep them in its database: with the `sea-orm` feature, create the table in a
migration with `create_tokens_table(DEFAULT_TOKENS_TABLE)` (`key`, `action`,
`expires_at` in milliseconds) and pass `SeaOrmTokenStore` to
`Panel::token_store`, or implement `TokenStore` over the host's own storage.

### Serving settings over gRPC

`botconf-grpc` serves a store and its panel as the `botconf.v1.Settings`
service ([`proto/botconf/v1/settings.proto`](proto/botconf/v1/settings.proto)),
so programs in other languages, other processes, or a host's own connectors
can read the settings, change them, watch them and show the panel:

| RPC | What it does |
|---|---|
| `Describe` | The runtime settings and their kinds, with every list of choices resolved now; the sections, those registered at runtime marked `dynamic` |
| `List`, `Get` | Values, as JSON text, with where each comes from |
| `Set`, `Unset`, `Add`, `Remove` | Change a key, like the text commands; values are read like typed input (JSON if it parses, text otherwise) |
| `Reload` | Re-read the config file, the environment and the stored overrides |
| `Render` | A page of the panel, as a `Screen` |
| `Act` | Press a button (its token), answer a question, or run a text command; the response is the engine's reaction, and a refused or stale press comes back in it (`rejected`, `stale`) rather than as an error |
| `Watch` | The values under some key prefixes, then every change to them; `catalog_changed` means sections were registered or removed, so `Describe` again |

A host mounts it next to its other tonic services:

```rust
let panel = Arc::new(
    botconf_panel::Panel::new(Arc::clone(&store))
        .token_store(MyPersistentTokenStore::new(db)) // see "Tokens"
        .domain(MY_DOMAIN),
);
let settings = botconf_grpc::SettingsService::new(panel, MyAuthorizer::new(…))
    .names(MyNames::new(…)) // names and suggestions for references
    .on_change(|change| { /* refresh what depends on the settings */ })
    .into_server();
Server::builder().add_service(settings) /* … */;
```

**Authorization is the host's.** Every request carries the actor the client
claims (`Actor { domain, id }`, e.g. the Telegram user who pressed a button),
and every RPC first asks the host's `Authorizer` with the request's metadata,
that actor and the `Operation` it wants: `Describe`, `Read { key }`,
`Change { key }`, `Reload`, `Panel` (rendering and acting, which can change any
setting) or `Watch { prefixes }`. The authorizer returns the `Principal` to
record as the author of changes, or the status to refuse with; a refused
request touches nothing. There is no default authorizer: refuse what you
don't recognise. (`AllowAll`, behind the `testing` feature, is for tests.)

The server keeps nothing per client: a question travels to the client in
`Ask.question` and comes back with the answer, and long buttons' handles are
in the panel's `TokenStore`. Give the panel a persistent store, so that
buttons keep working across restarts and for every client.

**tonic versions.** A service is added to the host's tonic server, so the
host must use the same tonic major version as `botconf-grpc` (tonic 0.12,
prost 0.13). Upgrading tonic happens here first, then in the hosts.

### Go

The Go module (`github.com/propheci/botconf/go`) holds the generated
bindings (`settingspb`) and a typed client (`settings`) for a
`botconf.v1.Settings` server, e.g. a connector reading its own settings
from wuphf's core:

```go
client, err := settings.Dial("127.0.0.1:50051",
	settings.WithBearer(os.Getenv("CORE_TOKEN")),     // the host decides who may do what
	settings.WithActor("telegram.user", "123456789"), // default actor for calls
)
if err != nil { /* … */ }
defer client.Close()

// The values under a prefix now, then each change; after a broken stream
// the client reconnects and starts again with an Initial event. Decoding
// into the same struct lays each change over what it holds.
var mine struct {
	Rendering struct {
		EditMode string `json:"edit_mode"`
	} `json:"rendering"`
}
const prefix = "connector_settings.telegram-main"
for event := range client.Watch(ctx, nil, prefix) {
	if event.Initial {
		mine = defaults // the whole state follows
	}
	if err := settings.Decode(event.Values, prefix, &mine); err != nil { /* … */ }
}
```

The server's statuses come back as typed errors: `errors.Is(err,
settings.ErrNotFound)` (`ErrPermissionDenied`, `ErrUnavailable`), and
`errors.As(err, &invalid)` with `*settings.InvalidValueError` for a refused
value, whose `Reason` is the server's explanation. `Render`, `Press`,
`Answer` and `Command` drive the panel, and `Questions` keeps a renderer's
pending question per conversation and user.

Regenerate the bindings with `scripts/gen-go.sh` (protoc-gen-go 1.36.11 and
protoc-gen-go-grpc 1.6.2, as CI checks). `go test -tags integration
./settings/...` runs the client against the Rust server (`botconf-grpc`'s
`toy_server` example; needs cargo and protoc).

### Versions and tags

The Rust crates and the Go module are released together, from the same
commit:

- Rust: `v0.x.y`
- Go: `go/v0.x.y`

The Go module lives in the `go/` subdirectory, so Go needs the `go/` tag
prefix to find its versions. The Go module path is
`github.com/propheci/botconf/go`.

## License

MIT, see [LICENSE](LICENSE).
