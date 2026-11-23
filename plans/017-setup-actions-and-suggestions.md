# Plan 017: Setup actions (interactive host tasks) and value suggestions in the setup screens

> **Executor instructions**: as plan 015. Build limits: `nice -n 19`,
> `CARGO_BUILD_JOBS=2`, `-p` the crates touched until the gate; the gate runs
> once, in the dev shell. Go: `GOMAXPROCS=2 GOFLAGS=-p=2 nice -n 19`. One
> build at a time.

## Status

- **Priority**: P1 (wuphf plans 062 and 063 need it)
- **Effort**: M–L
- **Risk**: LOW–MED — additive to `describe` and both setup screens; an
  action runs host code (a login) on a worker, so cancelling and the screen's
  redraw while it runs need care
- **Depends on**: 015 (`v0.13.x`)
- **Category**: feature
- **Planned at**: `v0.13.4`, 2026-11-23

## Why this matters

The owner wants `setup` to be the whole first setup (2026-11-23): link a
program's account from the setup screen (a QR code, a code typed in, a
password), and pick values such as chats by name instead of typing raw ids.
botconf provides the two generic pieces; the host supplies the platform code
and the names.

## Design decisions (settled with the owner, 2026-11-23)

| # | Decision |
|---|---|
| 1 | **Domains on values**: a value (or a list of values) may name a domain, the same names the panel's reference kinds use: Rust `#[botconf(domain = "wuphf.chat")]`, Go `domain:"wuphf.chat"`. `Entry::domain`. With no `panel` override, a runtime option with a domain gets `Kind::reference(domain, many)`, so a host stops writing both. |
| 2 | **Suggestions**: `Setup` takes an optional `suggest(domain, query, limit) -> [(value, label)]`. Editing a value with a domain shows the matches under the input as it is typed (the label and the value); Enter picks the highlighted one, and a typed value is still accepted. A list with a domain adds an item by the same search, and removes one from a list of its items. No suggestions: the input says why (the host's text, e.g. "no chats known yet: link a connector"). |
| 3 | **Actions**: `Setup` takes a list of actions (title, description). An action runs host code on a worker thread with a `Session`: `say(text)`, `qr(data)` (drawn with half blocks, readable by a phone), `progress(text)`, `ask(question) -> Option<String>` and `ask_secret(question)` (masked; `None` when the user cancelled), `cancelled()`. Esc cancels (the host's code sees `cancelled()` / a `None` answer and returns). It returns a notice or an error, shown like a save's. An action runs on the saved files: with unsaved changes the screen saves first (checked as a save), or the action does not start. |
| 4 | The actions are listed on the root screen below the options (and in the help); one runs at a time. |
| 5 | Release `v0.14.0` / `go/v0.14.0` (before plan 016's export/import). |

## Steps

1. **`describe`**: `Entry::domain` (Rust derive attribute, Go tag), the
   reference kind derived for runtime options, tests in both languages.
2. **`botconf-tui`**: the suggestion list in the editor (pure state, tested
   by keys), list items added and removed by search, the action screen (a
   `Session` over channels; the worker's events redraw the screen), QR
   drawing, save-before-run, cancel. Tests: picking a suggestion, typing past
   suggestions, an action asking and answering, cancelling an action, a QR
   drawn at a known size; the `setup` example gains a fake "link" action.
3. **`go/tui`**: the same (a bubbletea command per action event), with tests.
4. **Docs, gate, release**, row 017 DONE.

## STOP conditions

- A QR code drawn in the terminal does not scan from a phone at 80 columns
  (report the smallest size that works).
