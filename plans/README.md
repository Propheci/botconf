# Implementation Plans

Written on 2026-10-25, when this repository held only its README, license and
these plans. The Rust crates were then still in
`~/work/side_projects/telegram/assistants/assistant_bot_rs/main` (commit
`d772d66`), which the plans cite as the starting point. The design was settled
with the owner in one session for two programs: the assistant bot (Telegram,
Rust), which already uses botconf, and the wuphf messaging bridge (Rust core
with Go connectors), which will host its runtime settings with botconf and
show the settings panel from its connectors. wuphf has its own plans, in its
repository's `plans/`, that depend on these.

Execute in the order below, not in plan-number order. Each executor: read the
plan fully before starting, run its drift check, honor its STOP conditions,
and update your row when done.

Baseline at planning time (measured on a scratch copy of the crates, rustc
1.95.0, the assistant bot's lock file): `botconf` 13 unit + 10 integration + 1
doc test with `--all-features`; `botconf-telegram` 21 tests (with `takecell`
pinned to 0.1.1 or `--ignore-rust-version`, see 002).

## Execution order & status

| Order | Plan | Title | Priority | Effort | Depends on | Tag | Status |
|---|---|---|---|---|---|---|---|
| 1 | 001 | Move botconf and botconf-telegram here, with history; the assistant bot depends on this repo | P1 | M | — (Step 7 needs the owner to push) | `v0.1.0` | BLOCKED (Steps 1–6 done, `v0.1.0` tagged locally; Step 7 waits for the owner to push the tag) |
| 2 | 002 | Declare the minimum Rust versions (1.95, 1.96 for botconf-telegram) and test them in CI | P1 | S | 001 | `v0.1.1` | TODO |
| 3 | 003 | `Kind::Ref { domain, many }` replaces the Telegram id kinds; actors become strings | P1 | M | 001, 002 | `v0.2.0` | TODO |
| 4 | 009 | Owned descriptors (`Shared`) and sections registered at runtime | P1 | L | 003 | `v0.3.0` | TODO |
| 5 | 004 | Platform-neutral panel engine (`botconf-panel`) with hybrid tokens; botconf-telegram renders it | P1 | L | 003, 009 | `v0.4.0` | TODO |
| 6 | 005 | `botconf.v1.Settings` gRPC service (`botconf-grpc`) | P1 | L | 004, 009 | `v0.5.0` | TODO |
| 7 | 006 | Go module and settings client | P1 | M | 005 | `v0.6.0`, `go/v0.6.0` | TODO |
| 8 | 007 | Go panel renderers: Telegram bots, Telegram user accounts (inline-bot trick), Discord | P2 | L | 006 | `v0.7.0`, `go/v0.7.0` | TODO |
| 9 | 008 | SPIKE (roadmap): a Go port of the store | P3 | M | 005, 006, 009 | — | TODO |

Status values: TODO | IN PROGRESS | DONE | BLOCKED (with one-line reason) |
REJECTED (with one-line rationale).

Tags are created locally by the executor and pushed by the owner. Every
plan that a later plan's drift check names by tag must be tagged exactly as
listed.

## Dependency notes

- **009 runs before 004 although numbered after it.** It was added after
  004–008 were written, when wuphf's plans showed that connectors declare
  their settings at runtime. wuphf's plans cite 004 and 005 by number, so 009
  was appended instead of renumbering. It changes the core's descriptor
  types (owned `Shared` data instead of `&'static`) and makes the catalog
  append-only; 004's engine and 005's `Describe` are written against that
  result. Doing 004 first would mean adapting the engine twice.
- **003 before 009**: both edit `kind.rs`; 009's drift check expects 003's
  `Kind::Ref` and string actors.
- **005 needs 004 and 009**: it serves the engine's screens and serializes
  owned descriptors with resolved choices. `Choices::Dynamic` (a Rust
  function pointer) never goes on the wire; `Describe` resolves it.
- **001 Step 7, 003 Step 5, 009 Step 6, 004 Step 7** each switch the assistant bot to the
  new tag on a branch in that repository. They wait for the owner to push
  this repository and its tags. 004 Step 5 checks the assistant bot builds
  against a local path without committing there.
- **wuphf's plans** consume `v0.5.0` (service, dynamic sections, string
  actors) for the core and `go/v0.6.0`/`go/v0.7.0` for the connectors. wuphf
  builds with rustc 1.95, so every crate it uses (`botconf`,
  `botconf-panel`, `botconf-grpc`) must stay at a 1.95 minimum (002's
  `msrv` matrix).
- **tonic**: `botconf-grpc` pins tonic 0.12 / prost 0.13 because wuphf mounts
  its service on a tonic 0.12 server. Upgrading tonic is a coordinated change
  across both repositories.
- **008** is a roadmap spike: it produces a design document, not code.
- **Hybrid panel tokens (004)**: buttons carry the stateless encoding of
  their action when it fits, and otherwise a short handle kept in a
  `TokenStore` the host provides. 004 adds an in-memory store, a sea-orm one,
  and Step 7 gives the assistant bot a migration for its table. wuphf writes
  a sqlx store in its own plans and passes it to the `Panel` it serves
  through 005.

## Decisions confirmed by the owner on 2026-10-25

These were decided while planning and then confirmed by the owner. Each plan
keeps its reasoning under "Decided while planning".

- 001: the Cargo workspace is at the repository root (`members = ["rust/*"]`);
  history moves with `git subtree split` (the four commits under `crates/`;
  the earlier history stays in the assistant bot); the dev toolchain stays at
  1.98.1, with separate jobs for the minimum versions (002).
- 003: actors are strings (`Option<String>`), and the SQL column is text.
- 004: hybrid tokens (stateless when they fit, otherwise a stored handle;
  never a position into a list that can change); text is a span model;
  renderers pass their `Limits` and the engine pages screens for them;
  prompts are kept by the front ends.
- 005: the server keeps no per-conversation state (the client keeps the
  pending question; handles live in the host's token store); one `Act` RPC;
  `SettingsStore::subscribe` feeds `Watch`; no default authorizer.
- 006: Go dependency versions match wuphf's; `Watch` reconnects on its own.
- 007: packages `go/render/telegram` and `go/render/discord`; the inline-bot
  token is a random single-use handle answered only for the user account's
  own id; Discord prompts use modals.
- 009: `Shared<T>` (static or `Arc`) owned descriptors that keep `const`
  declarations.

Still open to review (decided while planning, not raised with the owner;
each is marked in its plan):

- 003: how a domain stores its ids (whatever JSON the host's type reads),
  and the nouns domains give.
- 005: the tonic 0.12 / prost 0.13 pin (forced by wuphf's server, see
  above), `Describe` resolving descriptors, and `Watch`'s prefix scope.
- 006: committed generated code, credentials as options, and question
  keeping in `settings.Questions`.
- 007: inline edits going to the message's data center, gotd entity text,
  and the per-platform limits.
- 009: `const` builders through `mem::forget`, the append-only catalog,
  dynamic sections outside the host's figment, parked overrides under
  dynamic namespaces, and validators returning the normalized value.

## Execution notes

- **001 Steps 1–6 done 2026-10-25.** The crates are under `rust/` with their
  history, the workspace builds and `nix flake check` passes (clippy with
  warnings denied, the tests, rustfmt, taplo). Tests match the Step 0
  baseline taken in the assistant bot on its 1.98.1 toolchain: `botconf`
  13 unit + 10 integration + 1 doc (1 ignored) with `--all-features`,
  `botconf-telegram` 21. Code under `rust/*/src` is unchanged from the split;
  only the two manifests changed (`botconf = { workspace = true }`,
  `repository.workspace = true`). `Cargo.lock` is the assistant bot's with
  the unused entries dropped; no version moved. `v0.1.0` is tagged locally.
  Deviations:
  - **Path-limited history.** The carried commits are the subtree merge's
    second parent, with their files at the split's root (`botconf/…`), so
    `git log -- rust` and `git log --follow rust/botconf/src/store.rs` show
    only the merge commit. `git log --graph` shows all four, and
    `git log <merge>^2 -- botconf/src/store.rs` follows a file through them.
    This is how `git subtree add` records history, not a lost commit.
  - **`flake.lock` copied** from the assistant bot instead of created fresh,
    so the toolchain hash, nightly rustfmt and nixpkgs match what the crates
    were tested with. `nix flake lock` left it unchanged.
  - **`.envrc` copied** from the assistant bot, including its nix-direnv
    bootstrap, rather than only `use flake`.
  - **Drift check:** this repository had three planning commits, not one,
    before the move (two later plan revisions). No `rust/` existed, so the
    check's intent held.
  - **Step 7 not started.** The assistant bot is unchanged, except that the
    temporary `botconf-split` branch from Step 1 still exists there; Step 7
    deletes it.
