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
| 1 | 001 | Move botconf and botconf-telegram here, with history; the assistant bot depends on this repo | P1 | M | — (Step 7 needs the owner to push) | `v0.1.0` | DONE (see execution notes) |
| 2 | 002 | Declare the minimum Rust versions (1.95, 1.96 for botconf-telegram) and test them in CI | P1 | S | 001 | `v0.1.1` | DONE |
| 3 | 003 | `Kind::Ref { domain, many }` replaces the Telegram id kinds; actors become strings | P1 | M | 001, 002 | `v0.2.0` | DONE (see execution notes; the assistant bot moved to `v0.2.0` in its `e6f8e83`) |
| 4 | 009 | Owned descriptors (`Shared`) and sections registered at runtime | P1 | L | 003 | `v0.3.0` | DONE (see execution notes; the assistant bot moved to `v0.3.0` in its `a815f3b`) |
| 5 | 004 | Platform-neutral panel engine (`botconf-panel`) with hybrid tokens; botconf-telegram renders it | P1 | L | 003, 009 | `v0.4.0` | DONE (see execution notes; assistant bot branch `botconf-v0.4` pending the owner) |
| 6 | 005 | `botconf.v1.Settings` gRPC service (`botconf-grpc`) | P1 | L | 004, 009 | `v0.5.0` | DONE (see execution notes) |
| 7 | 006 | Go module and settings client | P1 | M | 005 | `v0.6.0`, `go/v0.6.0` | DONE (see execution notes) |
| 8 | 007 | Go panel renderers: Telegram bots, Telegram user accounts (inline-bot trick), Discord | P2 | L | 006 | `v0.7.0`, `go/v0.7.0` | DONE (see execution notes; one live test of the inline route is the owner's) |
| 9 | 008 | SPIKE (roadmap): a Go port of the store | P3 | M | 005, 006, 009 | — | TODO |
| 10 | 010 | `reload()` re-reads the config file (`BaseSource`) | P1 | S | 009 | `v0.8.0`, `go/v0.8.0` | TODO |
| 11 | 011 | Drafts in the settings panel (`required`/`when` fields, `SettingsStore::check`) | P1 | M–L | 004, 005, 009, 010 | `v0.9.0`, `go/v0.9.0` | TODO |

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
- **010 and 011 (added 2026-10-29 at `4cf325a`)** come from two gaps wuphf's
  plan 039 found, and the owner's decisions on them: a reload must re-read
  the config file (010), and the panel needs drafts so a value that is only
  valid once several fields are set (a new wuphf rule, a rule switching kind)
  can be built before it is saved (011). 011 builds on 010's tag. wuphf
  adopts both in its plan 044 (pin `v0.9.0`); the assistant bot adopts 010 in
  010's Step 5 and needs nothing from 011. Both plans follow the build limits
  (niced, 2 jobs, one workspace gate, no local `nix flake check`).

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

- **001 done 2026-10-25.** The crates are under `rust/` with their
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
  - **Step 7** ran after the owner pushed `main` and `v0.1.0` (public). In the
    assistant bot, branch `botconf-git-dependency` (commit `a7a83e0`, for the
    owner to merge) depends on the git tag and drops `crates/`. Its suite
    passes with 310 tests, the 355 from before minus the 45 that moved here,
    and `nix flake check` passes. `cargo update -p botconf` could not match
    the former path packages, so `cargo check` added the two git sources
    instead. `Cargo.lock` only gained their `source` lines, and
    botconf-telegram lost three dev-dependency edges, since a git dependency
    does not lock its dev-dependencies. No version moved. taplo wrapped the
    long `botconf` dependency line. The `botconf-split` branch is deleted.
- **002 done 2026-10-25** (tag `v0.1.1`). `botconf` declares Rust 1.95 and
  `botconf-telegram` 1.96. On rustc 1.95 with the lock file, `botconf` passes
  9+10+1 (default), 12+10+1 (`cli`), 10+10+1 (`sea-orm`) and 13+10+1
  (`--all-features`). `botconf-telegram` passes its 21 tests on 1.95 with
  `--ignore-rust-version`, which shows its 1.96 comes only from takecell.
  `nix flake check` passes on the pinned 1.98.1. No local 1.96 toolchain
  exists (nixpkgs has 1.95 and 1.97.1, no rustup), so the 1.96 job is first
  proven by CI. The highest `rust-version` anything in the lock declares is
  takecell's 1.96. Two scope additions came from the owner after wuphf plan
  035 failed against `v0.1.0`
  (`botconf@0.1.0 requires rustc 1.98`, plus lock churn):
  - **Dependency floors, not pins.** The workspace dependencies were recent
    patch pins (`serde 1.0.229`, `tokio 1.53.1` with `full`, ...), which
    forced every host to bump its lock. They are now the oldest releases that
    resolve and pass with `cargo update -Z direct-minimal-versions` (checked
    in a scratch copy on rustc 1.95; every suite passed). Some floors come
    from teloxide and sea-orm, not botconf's own code.

    | Dependency | Before | After |
    |---|---|---|
    | anyhow | 1.0.104 | 1.0.80 |
    | arc-swap | 1.9.2 | 1.6 |
    | clap | 4.6.7, `derive` + `env` | 4.5, `derive` |
    | figment | 0.10.19, `env` + `toml` | 0.10.19 (dev: `test`, `toml`, `env`) |
    | futures | 0.3.34 | 0.3.30 |
    | serde | 1.0.229, `derive` | 1.0.219 (dev: `derive`) |
    | serde_json | 1.0.151, `preserve_order` | 1.0.142 |
    | thiserror | 2.0.21 | 2.0.18 |
    | tokio | 1.53.1, `full` | 1.39, `sync` (dev: `macros`, `rt`, `rt-multi-thread`, `test-util`) |
    | tracing | 0.1.44 | 0.1.40 |

    wuphf's locked versions (serde 1.0.228, serde_json 1.0.150, thiserror
    2.0.18, tokio 1.52.3, clap 4.6.1, anyhow 1.0.102, tracing 0.1.44,
    futures-util 0.3.32) all satisfy these, so depending on botconf adds
    packages to its lock without moving any. chrono, sea-orm and teloxide are
    unchanged.
  - **No `preserve_order`.** Feature unification would have turned it on for
    every host, and wuphf's migration tests compare stored JSON byte for
    byte. Nothing in either crate relies on object key order: every suite
    passes without it. Map entries (`Kind::Map` lists, `Choices::KeysOf`)
    are listed in sorted order, which is also steadier than the iteration
    order of a `HashMap` config field. The only visible difference is the key
    order of JSON values printed by `/config list` and `get`. The assistant
    bot still enables the feature in its own `serde_json` dependency
    (`Cargo.toml`, line 35), so its behaviour is unchanged.
  - **Lock file:** not unchanged as the plan expected, but no version moved.
    It only lost two edges: `indexmap` under serde_json (`preserve_order`)
    and `parking_lot` under tokio (`full`).
  - **CI fix:** the planned `msrv` job ran plain `cargo test`.
    `rust-toolchain.toml` (1.98.1) would win over the toolchain the action
    installs, so the job runs `cargo +<toolchain> test`.
- **003 done 2026-10-25** (tag `v0.2.0`). `Kind::Ref { domain, many }`
  replaces `Chat`, `Users` and `Chats`; actors are `Option<String>`, the
  SeaORM `updated_by` column is `text`, and `load` casts it to text. Tests,
  in the dev shell (1.98.1): `botconf` 15 unit (13 + the `Ref` empty-value
  test + `old_bigint_tables_still_load`) + 10 integration + 1 doc with
  `--all-features`; `botconf-telegram` 23 (21 + the two new handler tests).
  On rustc 1.95: `botconf --all-features` passes, and `botconf-telegram`
  passes with `--ignore-rust-version`. Clippy (warnings denied) and
  `cargo fmt --check` are clean. Proofs:
  - `old_bigint_tables_still_load` fails without the cast (sqlx: "Rust type
    `Option<String>` (as SQL type `TEXT`) is not compatible with SQL type
    `INTEGER`"), so the cast is needed, as the plan expected. The existing
    `stores_and_replaces_overrides` failed the same way.
  - `usernames_resolve_through_the_user_domain` fails when `resolve` is not
    asked; `references_of_unknown_domains_are_typed_and_listed` fails when the
    ❌ button carries the plain text instead of the JSON value.

  Deviations:
  - **Remove buttons keep the JSON value** (`item.to_string()`), not the
    plain text (`plain(item)`) the plan named. The handler reads the button
    with `parse_value`, so plain text would turn the string `"42"` into the
    number `42` and remove the wrong item. The new test covers both a quoted
    `"42"` and the number `42`.
  - **The toy's `things` is `Vec<Value>`**, not `Vec<String>`: the plan's own
    test stores `["a@b", 42]`, which `Vec<String>` rejects.
  - **A single `telegram.user` reference** (`Ref { domain: USER, many:
    false }`) also gets Telegram's user picker (limited to one user), and a
    picked user is read as its value. The old kinds had no single user, so
    nothing used this before.
  - **`nix flake check` was not run**: concurrent heavy builds crashed the
    owner's laptop during the first attempt, so builds were limited to
    targeted `cargo` runs. CI runs the flake checks and the minimum-version
    jobs on the pushed commits.
  - **Assistant bot (Step 5):** on branch `botconf-ref-kinds`, see its commit
    message; only targeted tests ran there, not `nix flake check`.
- **009 done 2026-10-25** (tag `v0.3.0`). Descriptors hold `Shared` data
  (a `'static` reference or an `Arc`), the catalog is append-only behind an
  `ArcSwap`, and sections can be registered at runtime below the schema's
  dynamic namespaces, with a pluggable `Validator`, their own base, and
  stored overrides that wait for their section (`Snapshot::parked`). Tests on
  rustc 1.95 (the shell's toolchain; `botconf-telegram` with
  `--ignore-rust-version`): `botconf` 19 unit (15 + `Shared`,
  `const_declarations_still_work`, `positions_never_move`, the parked
  listing render) + 10 store + 10 dynamic + 1 doc with `--all-features`;
  `botconf-telegram` 23, unchanged. Clippy (warnings denied) and the nightly
  rustfmt check are clean. Removing the parking at load, letting dynamic keys
  into the host's figment, always moving re-registered settings, or keeping
  waiting overrides on `unset` each makes a dynamic test fail.
  Deviations:
  - **Builders read fields instead of destructuring.** The planned builder
    (`let Self { .. } = self; mem::forget(old)`) does not compile on 1.95:
    "destructor of `RuntimeSetting` cannot be evaluated at compile-time",
    because a `const fn` may not destructure a value with drop glue either.
    `titled`, `kind`, `entry_names` and `Field::group` instead
    `ptr::read` each field they keep and `mem::forget(self)`; the declarations
    stay `const`, as the plan requires. The replaced field is never dropped:
    it holds no `Arc` in a `const`, and leaks if a builder is called at
    runtime on owned data, so runtime code sets the fields directly
    (`RuntimeSetting::owned`).
  - **Nested references get their own `const`.** A nested
    `&Kind::Form(&FORM)` or `&[Field::new(..)]` is no longer promoted to
    `'static` (the types have drop glue), so a form, its fields, or a kind
    used by a map value is declared in its own `const` (`const PRESET: &Kind
    = &Kind::form(PRESET_FORM);`). Only declaration syntax changed; no test
    expectation did.
  - **`Setting` borrows its kind.** `botconf-telegram`'s `Setting` holds
    `&Kind` and `Option<&Field>` borrowed from the catalog (it stays `Copy`),
    and a press on a removed setting shows "This setting no longer exists"
    above the home screen.
  - **`ValueEntry::description`** is `Option<Shared<str>>`, and
    `Listing` gained `parked`. The existing CLI render test keeps its
    expectation (its listing has nothing parked); a new test covers the
    "Waiting for their section" block.
  - **Crate versions** were still `0.1.0`; they are `0.3.0` now, matching
    the tag.
  - **Wire consistency (Step 4):** every descriptor field is plain data or
    `Shared`, which reads the same for both variants; the only exception is
    `Choices::Dynamic`, a function pointer, which 005's `Describe` must
    resolve with `Kind::choices(view)` and never send as is.
  - **Skipped:** `nix flake check` and the 1.96 job (CI runs both on the
    pushed commits), and taplo (not on the shell's path; the only TOML change
    is the version line).
- **004 done 2026-10-29** (tag `v0.4.0`). `botconf-panel` holds the screens,
  the presses, the answers and the text command, with no platform
  dependency (`cargo tree -p botconf-panel -e normal | grep -c teloxide` is
  0). `botconf-telegram` draws its screens: `render.rs` turns spans into
  Telegram HTML and rows into inline keyboards, `input.rs` turns questions
  into messages with Telegram's pickers and answers into `Input`, and
  `handlers.rs` keeps the questions in `Prompts`. Its public API is
  unchanged, plus `token_store`, `token_ttl`, `engine()` and the `sea-orm`
  feature. Tests (dev shell, 1.98.1, `--all-features`): `botconf` 19 + 10
  store + 10 dynamic + 1 doc, `botconf-panel` 37, `botconf-telegram` 10;
  every test name from the baseline still exists in one of the two crates.
  `botconf-panel` checks on rustc 1.95. Clippy (warnings denied) and the
  nightly rustfmt are clean. Removing the remove-item value check or binding
  dynamic picks by position each makes a test fail
  (`removing_an_item_removes_that_value`, `dynamic_picks_bind_the_value`).
  Step 5: the assistant bot's `cargo test --lib` (311 passed) ran against
  this tree through a temporary `[patch]`, with no source change there.
  Deviations:
  - **Total encoding.** Entry names and section ids are escaped in tokens
    (`%25`, `%3A`, `%23`, `%40`), so every action has a stateless encoding
    and a handle can always store it; names with `:` or `#`, refused before,
    now get buttons. An unknown `%` sequence reads back literally, so tokens
    drawn before still parse. A screen's part number follows the page after
    `@` (`o:m:lamp@1`).
  - **`Add(Target, Value)`** is a new action (`A:<target>:<json>`), for the
    suggested references of `Names::candidates`.
  - **`Domain` describes how to ask**, not only how to parse and count: what
    to send for one or several ids, an example id, what the front end's
    picker picks, and why an unknown word is not an id. With these the
    Telegram questions read exactly as before, and an unregistered domain
    gets the generic wording the panel used for other domains. `Viewer`
    gained `pickers` (whether the front end's pickers work in this
    conversation), and `Ask` gained `title` (Telegram's input placeholder)
    and `rows` (the suggestions). The engine leaves "how to stop" to the
    front end (Telegram adds "Tap Cancel" or "Send /cancel").
  - **`Panel::stored_label`** names the source of a value changed from the
    panel (default "changed at runtime ✏️"); `botconf-telegram` keeps
    "changed from Telegram ✏️".
  - **`Panel::clock`** sets the time source, so expiry is tested without
    sleeping.
  - **`SettingsStore::dynamic_sections()`** (new, in `botconf`) lists the
    sections registered at runtime, so the home screen can show them after
    the schema's, sorted by title, with their own title and description.
  - **Picked ids must match the setting's domain.** A chat picked for a list
    of users is refused; before, it was added.
  - **Suggestions in Telegram** go on the question message as an inline
    keyboard (when there is no picker keyboard); pressing one answers the
    question and updates the panel that asked.
  - **Tests moved and added.** The 9 scenario tests of `handlers.rs`, the 2
    reading tests of `input.rs` and the 3 of `text.rs` run in the engine
    against tokens taken from rendered screens (`reads_answers_by_kind` uses
    `Input::Picked` for Telegram's shared users and chats; the conversion
    of those messages is `picker_messages_become_picked_ids` in
    `botconf-telegram`). `data_that_does_not_fit_is_refused` tests issuing:
    no handle when even 11 bytes do not fit, a handle otherwise. The handle
    tests (`redrawing_reuses_handles`, `colliding_handles_take_the_next_key`,
    `expired_handles_are_purged_hourly`) drive the token issuer directly.
  - **Skipped:** `nix flake check` and the minimum-version jobs (CI runs
    them on the pushed commits), and taplo.
  - **Step 7 (assistant bot)** is on its branch `botconf-v0.4` (commit
    `e356c41`), for the owner to merge: pinned to `v0.4.0` with
    `botconf-telegram`'s `sea-orm` feature, a `panel_tokens` migration and
    `SeaOrmTokenStore` on the panel. The migration writes the table out
    (the same columns as `create_tokens_table`) instead of calling it, since
    the assistant's migration crate does not depend on botconf; the new
    test `panel_tokens_keep_button_handles` stores and reads a handle through
    `SeaOrmTokenStore`, and fails without the migration. `cargo test --lib`
    passes (312), clippy and rustfmt are clean, and `migrate` on a copy of
    the bot's SQLite file created the table. The test of the earlier actor
    migration found it by position from the end; it now finds it by name.
- **005 done 2026-10-29** (tag `v0.5.0`). `proto/botconf/v1/settings.proto`
  defines the service, and `botconf-grpc` serves it with tonic 0.12 and
  prost 0.13 (one tonic in the tree, as wuphf's). `SettingsStore::subscribe`
  hands out a `tokio::sync::watch` receiver of published snapshots, which
  `Watch` streams from. Tests: `botconf-grpc` 28 (the 15 planned service
  tests, a hook test, 11 mapping tests and a prefix test), passing on rustc
  1.95 (the wuphf shell's toolchain) and in the dev shell (1.98.1, with
  the whole workspace: `botconf` 19 + 10 dynamic + 11 store + 1 doc,
  `botconf-panel` 37, `botconf-telegram` 10); clippy (warnings denied),
  the nightly rustfmt and taplo are clean, and the proto lints with protoc.
  The lock file only gained packages. Skipping the authorizer for `Set`
  (its operation reported as `Panel`), dropping the change hook in `Act`,
  or not reporting removed keys in `Watch` each makes a test fail; so does
  not publishing to subscribers. Deviations:
  - **The proto follows 004 as it landed.** `Ask` carries `title` and the
    suggestion `rows`; `RenderRequest` and `ActRequest` carry `pickers`;
    `ChangeResponse` carries the counts of a reload (`overrides`, `ignored`).
    `Page` is `home` or `section` only: a page by token would need the
    engine to redeem handles outside a press, and a client opens a token's
    page by pressing it.
  - **Limits default to Telegram's.** Fields a client leaves at zero (or no
    `Limits` at all) mean 64-byte tokens, 4096 characters and 8 buttons a
    row; `max_rows` 0 means no limit.
  - **The host names references.** `SettingsService::names` takes the
    host's `Names` (e.g. wuphf's chat titles), since names come from the
    host, not the client; the `Panel` holds the domains and the token store.
  - **`on_change` hears every change made through the service**: setters,
    `Reload`, panel presses and answers, and text commands.
  - **A text command with no arguments** answers with the panel in `post`
    (a new message), like the engine's `CommandReply::Panel`.
  - **Unset of nothing** answers `changed: false` with the key's current
    value, so a client can show it.
  - **`Watch` with no prefixes** watches every key, and a key whose section
    was removed comes back with no value (and `catalog_changed`).
  - **`#![allow(clippy::result_large_err)]`** in `botconf-grpc`: handlers and
    the authorizer answer with `tonic::Status`, as wuphf's do.
  - **Nix and CI:** the dev shell and the checks get protoc; the checks'
    source keeps `.proto` files (`cleanCargoSource` dropped them); the
    `msrv` job installs Ubuntu's protoc for `botconf-grpc` on 1.95.
  - **Crate versions** are `0.5.0`, matching the tag.
  - **Skipped:** `nix flake check` (CI runs it on the pushed commits) and
    the 1.96 job for `botconf-telegram`.
- **006 done 2026-10-29** (tags `v0.6.0` and `go/v0.6.0`). `go/` is the
  module `github.com/propheci/botconf/go`, with the bindings in
  `go/settingspb` (committed, generated by `scripts/gen-go.sh` with protoc
  34.1, protoc-gen-go 1.36.11 and protoc-gen-go-grpc 1.6.2 from the wuphf dev
  shell) and the client in `go/settings`. It requires grpc 1.76.0 and
  protobuf 1.36.11, like wuphf's `proto/go`, and its indirect requirements
  match that module's. Tests: the 8 planned unit tests over `bufconn`
  (race detector on), and the cross-language test (build tag
  `integration`) against `botconf-grpc`'s new `toy_server` example. Leaving
  out the bearer token, Watch's reconnect, or the InvalidArgument mapping
  each makes a test fail. Deviations:
  - **No page by token.** 005's proto has only `home` and `section` pages,
    so the client has `Home()` and `Section(id)` but no `PageToken`; a
    client opens a token's page by pressing it.
  - **A `View` instead of a bare `Limits`.** `Render`, `Press`, `Answer` and
    `Command` take `View{Limits, Pickers}`, because 005's requests carry
    `pickers` as well as the limits.
  - **Answers are built with `TextInput`, `PickedInput` and `CancelInput`**,
    not a single `Input` constructor per kind.
  - **`WithWatchErrors(func(error))`**, an option the plan did not list,
    hands each error that ends a Watch stream to the caller (e.g. to log it)
    before the client reconnects.
  - **Errors keep their gRPC status.** `*settings.Error` matches the `Err`
    values with `errors.Is` and still answers `status.FromError`; an
    `UNAUTHENTICATED` status counts as `ErrPermissionDenied`.
  - **`Decode`** lays the value at the prefix itself (if any) under the
    values below it, and skips keys without a value.
  - **Tool versions.** botconf's own dev shell has protoc 36.1 and
    protoc-gen-go 1.36.12 (nixpkgs), so the committed files come from the
    wuphf shell's pinned versions. CI installs the pinned plugins and
    Ubuntu's protoc, and its diff ignores only the protoc version line in
    the headers (`git diff -I '^// (.|- )protoc +v'`), so a plugin or code
    difference still fails it.
  - **The integration step in CI** sets `RUSTUP_TOOLCHAIN=stable`, which
    wins over `rust-toolchain.toml`'s dev toolchain.
  - **Crate versions** are `0.6.0`, matching the tag.
  - **Skipped locally:** `nix flake check` (CI runs it on the pushed
    commits). Every build ran niced with two jobs.
- **007 done 2026-10-29** (tags `v0.7.0` and `go/v0.7.0`). `go/render/telegram`
  (gotd v0.144.0, as wuphf's Telegram connector) and `go/render/discord`
  (discordgo v0.29.0, the latest release). Adding gotd raised the Go
  module's `golang.org/x/*` requirements to what wuphf's Telegram connector
  already uses; grpc and protobuf stay at 1.76.0 and 1.36.11. The Telegram
  package does not import discordgo. Tests use a recording gotd invoker, a
  fake Discord API and a scripted settings server; none talk to Telegram or
  Discord. Dropping the answer-once guard, letting a stranger's inline query
  get the panel, editing an inline message on the main connection instead of
  its DC, skipping markdown escaping, or never deferring a slow press each
  makes a test fail. Deviations:
  - **`go/settings/settingstest`** is a new scriptable server (`RenderFunc`,
    `ActFunc`, recorded requests, `Start`) rather than 006's test fake
    exported as is: the renderers need to script responses, which that fake
    cannot. 006's own tests keep their fake.
  - **`BotPanel.Command` takes the command's message id**, so text replies
    answer the command as the plan asks.
  - **`BotPanel.HandleService`** (not in the plan) takes the users or chats
    picked with Telegram's picker, which arrive as a service message, not a
    `*tg.Message`. Picked chats are sent as Bot API ids (`-100…` for
    channels and supergroups), the `telegram.chat` domain's form. Options
    `WithPickers` and `WithPeerResolver` (default: zero access hash, which
    a bot may use for peers it has met) were added.
  - **`InlinePanel` takes a `HelperBot` interface** (`API()` and `DC()`),
    which `*telegram.Client` satisfies, instead of `*telegram.Client`
    itself, so tests can fake the data centers. A typed nil client counts
    as no helper bot.
  - **`HandleInlineQuery` returns `(bool, error)`**: a query that is not a
    panel token is left to the host (wuphf's sticker plan uses the same
    helper bot). A stranger's query for a live token gets an empty answer and
    does not use the token up.
  - **Where an inline panel's questions are answered.** The bot cannot tell
    which chat an inline message is in, so a question is answered by the
    account's next message in the chat the panel was last opened in. One
    inline panel per account at a time works best.
  - **A `post` on an inline panel replaces it** instead of posting a new
    message, for the same reason. Notices after typed answers show above
    the redrawn panel; refusals show on the panel and keep the question.
  - **Discord presses are answered directly when they are quick.** Discord
    lets only the first response to a press open a modal, so the renderer
    presses first and updates the message (or opens the modal) when the
    press takes under two seconds; a slower press is acknowledged with a
    deferred update and the message edited afterwards, as the plan says.
    A question that comes after a deferral (or after a modal) shows its text
    with an "✏️ Answer" button that opens the modal. `TestPressDefersThenEdits`
    covers the slow path and `TestPressUpdatesTheMessage` the quick one.
  - **The Discord fake has four calls**, not three: `FollowupMessageCreate`
    carries the ephemeral notices.
  - **Custom ids `cfg:!q` and `cfg:!p`** belong to the renderer (the modal
    and its button, the picker select menu); panel tokens never start with
    `!`.
  - **The Discord panel has no `Command`**: Discord programs take commands as
    slash commands, which the host routes.
  - **Crate versions** are `0.7.0`, matching the tag.
  - **Not tested here, for the owner:** the inline route against real
    Telegram (the plan's STOP condition asks for one manual run). With a
    real helper bot (inline mode on) and a user account: open the panel in a
    chat, check that the message shows the bot's buttons, press one, and
    check that the message is edited.
  - **Skipped locally:** `nix flake check` (CI runs it). Every Go command ran
    niced with `GOMAXPROCS=2` and `-p=2`.
