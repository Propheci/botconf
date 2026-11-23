# Plan 014: Section actions — buttons that run the host's commands

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. When done, update the status row for this plan in
> `plans/README.md`.
>
> **Build limits (the owner's laptop)**: prefix every cargo command with
> `nice -n 19` and set `CARGO_BUILD_JOBS=2`. Check and test only
> `botconf-panel` until the gate; run the workspace gate once, in the dev
> shell (`nix develop -c`). No `nix flake check`. One build at a time.
>
> **Drift check (run first)**: `git diff --stat v0.11.0..HEAD -- rust/ proto/`
> → empty. Must still hold: `Panel::section_note` takes a
> `Fn(&Snapshot<S>, &str) -> Option<String>`; the section page
> (`screens.rs`, `section`) lists its settings, then `‹ Back`; `Reaction` has
> `screen` and `notice`.

## Status

- **Priority**: P1 (wuphf plan 058 needs it)
- **Effort**: S–M
- **Risk**: LOW — additive: a new builder pair, a new token action, one
  more row group on section pages. No proto change, so the gRPC service and
  every Go renderer show it unchanged (a press's reaction carries a notice
  and a redraw already).
- **Category**: feature
- **Planned at**: `v0.11.0`, 2026-11-23

## Why this matters

wuphf's owner wants buttons in the settings panel that *do* something for a
section rather than change a setting: "🔄 Resync app state" and "🔗 Send a
new link code" on a WhatsApp connector's section. The panel engine only
edits settings. The host knows which actions a section offers (they may
come and go, e.g. while a connector is connected) and how to run them.

## Design decisions

| # | Decision |
|---|---|
| 1 | `Panel::section_actions(Fn(&Snapshot<S>, &str) -> Vec<SectionAction>)`: the actions a section offers now, by section id, like `section_note`. `SectionAction { id, title }`; `title` is the button's label. |
| 2 | `Panel::on_action(Fn(ActionCall) -> BoxFuture<'static, Result<String, String>>)`: runs one; `ActionCall { section, action, actor }`. `Ok(text)` becomes the press's notice; `Err(text)` the notice `❌ <text>`. Either way the section page is redrawn. |
| 3 | The section page shows one row per action after its settings, before `‹ Back`. |
| 4 | Token `Action::Run(section, action)`, encoded `X:<section>:<action>` with both escaped; a long one falls back to a stored handle, as every token does. |
| 5 | A press on an action the section no longer offers is stale ("This action is no longer offered") and redraws the section; with no `on_action` set it is rejected. The engine does not authorize further: the host's authorizer already admitted the viewer to the panel. |
| 6 | Release `v0.12.0`, with `go/v0.12.0` on the same commit (no Go change). |

## Steps

### Step 1: The engine

`rust/botconf-panel`:
- `lib.rs`: `SectionAction`, `ActionCall`, the two type aliases
  (`SectionActions<S>`, `ActionHandler`), the builders, a `section_actions`
  reader next to `note`; export them.
- `token.rs`: `Action::Run(String, String)`, encode/parse, the round-trip
  test case; `press.rs` excludes it from `edit_for`.
- `screens.rs`: the action rows on the section page.
- `press.rs`: `Action::Run` → decision 5, then the handler → decision 2.

Tests (`tests.rs`, with the testing panel):
1. `a_section_lists_its_actions` — the rows after the settings, labels as
   titled; a section offering none shows none.
2. `pressing_an_action_runs_it` — the handler got section, action and the
   viewer's actor; the reaction's notice is its text and the section is
   redrawn.
3. `a_failed_action_says_so` — `Err("down")` → notice `❌ down`.
4. `an_action_no_longer_offered_is_stale`.
5. `without_a_handler_an_action_is_rejected`.
6. Token round trip (extend `buttons_round_trip`) and a long id taking a
   stored handle.

**Verify**: `cargo test -p botconf-panel` → baseline + 6 (or 5 with the
round trip folded in), clippy clean. Mutation: drop the offered check → test
4 fails. Commit: `feat(panel): section actions that run the host's commands`.

### Step 2: Docs, gate and release

README (the panel section): section actions. Version 0.12.0. Gate in the dev
shell: `cargo test --workspace --all-features`, clippy `-D warnings`,
`cargo fmt --check`, `taplo fmt --check`. Tag `v0.12.0` and `go/v0.12.0`,
push main with the tags. Row 014 DONE.
Commit: `docs: section actions; version 0.12.0`.

## STOP conditions

- The drift check fails.
- A Go renderer would need a change to show the action or its notice.
