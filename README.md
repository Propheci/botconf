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
  botconf-telegram/   a Telegram (teloxide) settings panel
proto/     the wire contract (botconf.v1.Settings), shared by Rust and Go
go/        the Go module: a client for remote settings and panel renderers
plans/     implementation plans (see plans/README.md)
```

## Status

The Rust crates live in `rust/`. They moved here from
[assistant_bot_rs](https://github.com/akshettrj/assistant_bot_rs) with their
history ([plan 001](plans/001-extract-crates.md)); their earlier life inside
that bot's `src/settings/` stays in its repository. The remote settings
service, the shared panel engine and the Go SDK follow in plans 002–009.
[`plans/README.md`](plans/README.md) has the order and the status of each.

Develop inside the Nix dev shell (`nix develop`, or `direnv allow`), which
provides the toolchain, a nightly rustfmt (`rustfmt.toml` uses unstable
options) and taplo. `nix flake check` runs clippy, the tests and the format
checks, as CI does.

## Using it

Until the API settles, botconf is used as a git dependency pinned to a tag,
not from crates.io or a Go proxy release:

```toml
# Cargo.toml
# The core. Features: `sea-orm` (keep the overrides in a SQL table with
# SeaORM) and `cli` (a clap subcommand to view and change the settings).
botconf = { git = "https://github.com/propheci/botconf", tag = "v0.1.0", features = ["cli", "sea-orm"] }
# A Telegram settings panel (teloxide) over a botconf store.
botconf-telegram = { git = "https://github.com/propheci/botconf", tag = "v0.1.0" }
```

```sh
# Go (once go/ exists)
go get github.com/propheci/botconf/go@go/v0.1.0
```

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
