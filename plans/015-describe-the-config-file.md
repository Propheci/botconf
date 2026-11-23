# Plan 015: Describe the whole config file from its type; examples and an offline setup TUI

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. When done, update the status row for this plan in
> `plans/README.md`.
>
> **Build limits (the owner's laptop)**: `nice -n 19`, `CARGO_BUILD_JOBS=2`,
> `-p` the crates touched until the gate; the gate runs once, in the dev
> shell (`nix develop -c`). Go: `GOMAXPROCS=2 GOFLAGS=-p=2 nice -n 19`. One
> build at a time. No `nix flake check`.

## Status

- **Priority**: P1 (wuphf plans 059 and 060 need it)
- **Effort**: XL
- **Risk**: MED — additive (new crates, a new Go package); the existing
  runtime-settings API is untouched, and a host may keep writing its
  `RuntimeSetting`s by hand
- **Category**: feature
- **Planned at**: `v0.12.0`, 2026-11-23

## Why this matters

A host describes each option several times today: the config type (with
its doc comments), the runtime settings' descriptors (`RuntimeSetting`,
`Kind`, `Form` consts in Rust; `botconf`/`doc` tags in wuphf's Go
connectors), and the commented `config.example.toml`. They drift, and a
first-time setup means reading the example and editing TOML by hand.

The owner decided (2026-11-23) that botconf describes the **whole config
file** from the config type itself, annotated once, and derives from that
description: the runtime settings, the example file, and an offline setup
TUI that edits the file (and the env file its options name) with every
option's description, without the program running.

## Design decisions (settled with the owner)

| # | Decision |
|---|---|
| 1 | **One annotated type is the source.** Rust: `#[derive(botconf::Describe)]` (new proc-macro crate `botconf-derive`, re-exported by `botconf` behind a `derive` feature) reads `///` doc comments as descriptions and serde's own attributes (`rename`, `rename_all`, `default`, `default = "…"`, `skip`, `flatten`, internally tagged enums), plus `#[botconf(…)]` for what serde does not say (title, runtime, env, secret, kind override, bounds, example). Go: reflection over the struct's tags (`toml`, `doc`, `title`, `default`, `env`, `secret`, `botconf` = runtime title, `jsonschema` enum/min/max), as wuphf's settings package already does for runtime keys, moved into botconf's Go module. |
| 2 | **The description** (`botconf::describe`): a tree of tables (struct), lists of tables, maps of tables, tagged maps and fields; a field has key, title, description, value kind (text, secret text, bool, integer with bounds, one-of, list of text, env-variable name, JSON), default, required, runtime flag, and an optional panel `Kind` override. |
| 3 | **Runtime settings are derived**: `Description::runtime_settings()` gives the `RuntimeSetting`s of the fields marked runtime, with a panel `Kind` from the value kind (or the override, e.g. a `Ref` or a `Form`). Hosts may still add hand-written ones. |
| 4 | **The example file is generated** (`Description::example()`): descriptions as comments, defaults and examples as values, absent optional keys commented out. A host tests that its checked-in example equals it. |
| 5 | **Offline setup TUI**: `botconf-tui` (ratatui) and Go `go/tui` (bubbletea), over a description, a config file and an env file, with a validator hook (the host's own loader). The file is written from the description (so it always reads like the example); keys it does not describe are kept as comments. An env-variable option sets its variable's value in the env file (masked when secret; a token can be generated); the env file keeps its other lines. |
| 6 | Release `v0.13.0` / `go/v0.13.0`. |

## Steps

1. **`botconf::describe`**: the model, `Describe` for the primitives,
   `Option`, `Vec`, `BTreeMap`/`HashMap<String, _>`; TOML read/render;
   `example()`; `runtime_settings()`. Tests on hand-built descriptions.
2. **`botconf-derive`**: the derive for structs (named fields) and enums
   (unit → one-of with serde's renames; internally tagged struct variants →
   tagged). Tests: a fixture config type covering every attribute, its
   description compared field by field; trybuild-free (unit tests on the
   generated description only).
3. **`botconf-tui`**: the app state (pure, tested by keys), the ratatui
   view, the env file. `botconf_tui::run(description, config_path,
   env_path, validate)`.
4. **Go**: `go/describe` (reflection, the same model, render, example,
   runtime descriptors) and `go/tui` (bubbletea), with tests.
5. **Docs, gate, release**: README sections, `v0.13.0` and `go/v0.13.0`,
   row 015 DONE.

## STOP conditions

- A serde attribute the wuphf configs use cannot be read by the derive
  without guessing (report it).
- A new Go dependency needs cgo.
