# Plan 010: `reload()` re-reads the config file

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Build limits (the owner's laptop)**: prefix every cargo command with
> `nice -n 19` and set `CARGO_BUILD_JOBS=2`. Check and test only the crates you
> touch (`-p`), and run the workspace gate once, at the end. Do not run
> `nix flake check`; CI runs it on push. Only one agent builds at a time.
>
> **Drift check (run first)**: `git diff --stat v0.7.0..HEAD -- rust/botconf`
> Expected: empty (007 tagged `v0.7.0`; 006 and 007 touched only `go/`, the
> README and `botconf-grpc`'s example). What must still hold:
> `rust/botconf/src/store.rs` has `pub struct SettingsStore<S>` with a
> `base: Figment` field (line 69), `pub async fn load(schema: S, base: Figment,
> storage: impl Storage)` (line 124), `pub async fn reload(&self)` building a
> snapshot from `self.layers(..)` (lines 192–205), and `fn layers` returning
> `Layers { base: &self.base, .. }` (lines 590–597). If the store already
> takes something other than a `Figment`, STOP.

## Status

- **Priority**: P1 (the owner's decision; both hosts claim a reload re-reads
  the file and neither does)
- **Effort**: S
- **Risk**: LOW-MED — changes what `reload` sees, and the base can now change
  while the store runs; the existing store tests are the safety net
- **Depends on**: 009 (dynamic sections keep their own base and must not be
  re-read)
- **Category**: bug / API
- **Planned at**: `4cf325a` (tag `v0.7.0`), 2026-10-29

## Why this matters

`SettingsStore` keeps the `Figment` it was loaded with and layers every later
change on a clone of it. Figment reads its providers when they are *merged*,
not when the figment is extracted, so the stored `Figment` is a frozen copy of
the file and the environment as they were at startup. `reload()` therefore
re-reads only the stored overrides:

- The assistant bot's README says `/config reload` "re-read[s] the config file
  and the database" and that a running bot "picks up … edits to the config
  file after `/config reload`" (assistant_bot_rs `README.md:130` and `:143`).
  Neither is true.
- botconf's own README describes the gRPC `Reload` as "Re-read the config
  file, the environment and the stored overrides" (`README.md:254`), and the
  panel's help says "reload — re-read the config file and the stored settings"
  (`rust/botconf-panel/src/command.rs:70`).
- wuphf found it while executing its plan 039 (see wuphf's `plans/README.md`,
  039's notes: its test `owner_ids_follow_reload` could not be written) and
  had to word its CLI around it (042: "file edits still need a restart").

The owner decided on 2026-10-29 that reload **must** re-read the file.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | `reload()` rebuilds the base from its sources (the config file, the environment) before re-reading the stored overrides. |
| 2 | A reload whose new base is invalid, or whose sources fail (e.g. the file was deleted), is **rejected**: the current snapshot and the current base stay, and the error says why. |
| 3 | Sections registered at runtime (009) keep their own base; a reload never re-reads it. |

### Decisions this plan makes (flag to the owner; reviewers may push back)

- **A `BaseSource` trait, with impls for closures and for `Figment`.**
  ```rust
  pub type BaseError = Box<dyn std::error::Error + Send + Sync>;

  /// Where the base configuration comes from. Asked at load and at every
  /// reload.
  pub trait BaseSource: Send + Sync + 'static {
      fn figment(&self) -> Result<Figment, BaseError>;
  }

  /// A fixed base: a reload sees the same values (figment read the sources
  /// when they were merged).
  impl BaseSource for Figment { /* Ok(self.clone()) */ }

  /// A base rebuilt on each call, e.g. `move || AppConfig::figment(&path)`.
  impl<F, E> BaseSource for F
  where
      F: Fn() -> Result<Figment, E> + Send + Sync + 'static,
      E: Into<BaseError>,
  { /* self().map_err(Into::into) */ }
  ```
  A trait, not a bare `impl Fn`, so that `SettingsStore::load(schema, figment,
  storage)` keeps compiling unchanged for hosts that don't need reloads (the
  store's own tests, the doc example, botconf-grpc's toy server, the
  testing helpers). The two impls do not overlap: `Fn` is a fundamental
  trait, so the compiler knows `Figment` does not implement it. If the
  compiler disagrees (E0119), see STOP conditions.
- **`load` takes `base: impl BaseSource`** instead of `base: Figment`. Every
  existing call site passes a `Figment` and keeps working. No `load_with`
  twin.
- **The live base is an `ArcSwap<Figment>`**, swapped only while the write
  lock is held, after a successful reload. Every change (`set`, `unset`,
  `register_section`, …) layers on the live base, so after a reload the
  edited file is what `unset` falls back to.
- **Errors**: a failing source becomes
  `SettingsError::InvalidBase(BuildError::Source(reason))` (new variant). An
  invalid new base keeps using the existing `InvalidBase(BuildError::…)`. At
  `load`, both stay fatal, as an invalid base is today.
- **Keys read only at startup** (a bot token, a database URL, a listen
  address) change in the snapshot on reload, but the host only reads them at
  startup. The store cannot know which keys those are, so this is the host's
  documentation, not a check. Each host's step below says so in its README.

## Current state

- `rust/botconf/src/store.rs:65-80`, the store keeps a fixed base:
  ```rust
  pub struct SettingsStore<S: Schema> {
      schema: Arc<S>,
      sections: Arc<[Section]>,
      namespaces: Vec<&'static str>,
      base: Figment,
      storage: Box<dyn Storage>,
      catalog: ArcSwap<Catalog>,
      current: ArcSwap<Snapshot<S>>,
      published: watch::Sender<Arc<Snapshot<S>>>,
      write_lock: Mutex<Registry>,
  }
  ```
- `store.rs:124-160`, `load(schema: S, base: Figment, storage: impl Storage)`
  builds `Layers { base: &base, .. }`, calls `load_snapshot`, and stores
  `base`.
- `store.rs:192-205`, reload never touches the base:
  ```rust
  pub async fn reload(&self) -> Result<Change<S>, SettingsError> {
      let registry = self.write_lock.lock().await;
      let dynamic = registry.sections();
      let snapshot = load_snapshot(
          &self.layers(&dynamic),
          self.storage.as_ref(),
          &self.catalog.load(),
          &self.namespaces,
      )
      .await?;
      Ok(self.publish(snapshot))
  }
  ```
- `store.rs:590-597`, `fn layers` hands out `base: &self.base`.
- `store.rs:657-662`, `struct Layers<'a, S> { schema, sections, dynamic, base:
  &'a Figment }`; `store.rs:668` `load_snapshot` builds from
  `layers.base` and maps a bad base to `SettingsError::InvalidBase`.
- `store.rs:30-50`: `SettingsError::InvalidBase(#[source] BuildError)` and
  `BuildError { Config(Box<figment::Error>), Section { path, reason },
  Invalid(String) }`.
- `rust/botconf/tests/store.rs:139-158`, `sources_tell_where_values_come_from`
  runs a store inside `figment::Jail` with `Toml::file("config.toml")`; copy
  its pattern (a tokio runtime inside the jail) for the new tests.
  `store.rs:338-345` `an_invalid_base_is_refused`.
- Callers that pass a `Figment` today: `rust/botconf/src/lib.rs` (doc
  example), `rust/botconf/tests/*.rs`, `rust/botconf-panel/src/testing.rs`,
  `rust/botconf-telegram/src/testing.rs`,
  `rust/botconf-telegram/examples/panel_bot.rs`, `rust/botconf-grpc/src/tests.rs`,
  `rust/botconf-grpc/examples/toy_server.rs`. They must compile unchanged.
- Wording that claims a reload re-reads the file: `README.md:254`,
  `rust/botconf-panel/src/command.rs:70`. Wording that only counts stored
  settings: `rust/botconf/src/cli.rs:108` ("Reloaded: {overrides} stored
  setting(s) applied, {ignored} ignored"), `rust/botconf-panel/src/edit.rs:53`.
- assistant_bot_rs (`~/work/side_projects/telegram/assistants/assistant_bot_rs/main`,
  `main` at `e356c41`, on botconf `v0.4.0`):
  - `src/settings/mod.rs:100-111`: `pub async fn load(base: Figment, db,
    modules, log_filter)` calls `SettingsStore::load(schema, base,
    SeaOrmStorage::new(db))`.
  - `src/app.rs:20-21`: `let base = AssistantConfig::figment(&cli.config)?;`
    then `AssistantConfig::from_figment(&base)`; `settings::load(base, …)` at
    lines 53, 66 and 100.
  - `src/config/loader.rs`: `AssistantConfig::figment(path) ->
    Result<Figment, ConfigError>` (checks the file exists, merges
    `Toml::file_exact` and `ASSISTANT_*` env).
  - `README.md:130` and `:143` (quoted above).

## Commands you will need

```sh
export CARGO_BUILD_JOBS=2
nice -n 19 cargo check -p botconf --all-targets --all-features
nice -n 19 cargo test -p botconf --all-features
nice -n 19 cargo test --workspace --all-features   # once, at the end
nice -n 19 cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
```

In assistant_bot_rs, run every cargo command through its own shell:
`direnv exec . sh -c '…'` (its toolchain is 1.98.1; the wuphf shell's 1.95
rustfmt reformats unrelated files there).

## Scope

In: `rust/botconf/src/{store.rs,lib.rs,cli.rs}`, `rust/botconf/tests/store.rs`,
`rust/botconf-panel/src/edit.rs`, `README.md`, `plans/README.md`, crate
versions; the assistant bot's `src/settings/mod.rs`, `src/app.rs`, `README.md`
and `Cargo.toml`/`Cargo.lock` (Step 5).

Out: the proto and the Go module (the `Reload` RPC already says it re-reads the
file; nothing on the wire changes), dynamic sections' bases, wuphf (its plan
044 adopts this).

## Git workflow

Commit on `main`. Tag `v0.8.0` and `go/v0.8.0` on the final commit (the Rust
crates and the Go module are released together; the Go module is unchanged).
Push `main` and both tags yourself (the owner allows botconf pushes); never
force-push, never move a tag. The assistant bot's step goes on a new branch
there, committed but not pushed or merged.

## Steps

### Step 0: Baseline

`nice -n 19 cargo test -p botconf --all-features` → record the counts (the
004–007 notes: 19 unit + 10 dynamic + 11 store + 1 doc).

### Step 1: `BaseSource`

In `rust/botconf/src/store.rs` (or a new `rust/botconf/src/base.rs`, re-exported
from `lib.rs`), add `BaseError`, the `BaseSource` trait and its two impls as
in the design decisions, with doc comments that say a `Figment` is a fixed
snapshot of its sources and a closure re-reads them.

Add `BuildError::Source(String)` (`#[error("the configuration sources failed:
{0}")]`).

**Verify**: `nice -n 19 cargo check -p botconf` → exit 0.

### Step 2: The store uses the source

- Replace `base: Figment` with `source: Box<dyn BaseSource>` and `base:
  ArcSwap<Figment>`.
- `load(schema, base: impl BaseSource, storage)`: call `base.figment()`;
  a failure is `SettingsError::InvalidBase(BuildError::Source(error.to_string()))`.
  Store the source and the figment.
- `fn layers` takes the base explicitly (`fn layers<'a>(&'a self, base: &'a
  Figment, dynamic: &'a [..])`), and every caller loads it once per operation
  (`let base = self.base.load();`) while holding the write lock, so one change
  uses one base.
- `reload()`, under the write lock: rebuild the base with
  `self.source.figment()` (failure → the `Source` error, nothing swapped);
  build the snapshot with `load_snapshot` over the **new** base (an invalid
  base → its `InvalidBase` error, nothing swapped); only then store the new
  base and publish.
- Update the doc comments of `SettingsStore`, `load` and `reload` ("Rebuilds
  the base from its sources — the config file and the environment, for a
  closure source — then re-reads the stored overrides").

**Verify**: `nice -n 19 cargo test -p botconf --all-features` → the Step 0
counts, all green (no test changed yet); `nice -n 19 cargo check -p
botconf-panel -p botconf-telegram -p botconf-grpc --all-targets` → exit 0 with
no call site edited.

### Step 3: Tests

In `rust/botconf/tests/store.rs`, using `figment::Jail` like
`sources_tell_where_values_come_from`:

1. `reload_reads_an_edited_file` — load with
   `|| Ok::<_, figment::Error>(Figment::from(Toml::file("config.toml")))`,
   rewrite `config.toml` with a new `greeting`, `reload()` → the new value,
   `source("greeting") == Source::File`.
2. `reload_keeps_the_snapshot_when_the_file_becomes_invalid` — rewrite the
   file with `greeting = ""` (the Toy schema rejects an empty greeting) →
   `reload()` is `Err(SettingsError::InvalidBase(_))`; `current()` still has the
   old value; a following `set` still succeeds (the old base is still live).
3. `reload_keeps_the_snapshot_when_the_source_fails` — a closure that reads
   with `Toml::file_exact` after the file was deleted, or a closure returning
   `Err("gone")` on the second call → `InvalidBase(BuildError::Source(_))`,
   snapshot kept.
4. `overrides_still_win_over_an_edited_file` — `set("greeting", "stored")`,
   edit the file, reload → still `"stored"`; `unset("greeting")` → the
   **edited** file's value (proves later changes layer on the new base).
5. `a_fixed_base_does_not_reread` — load with a `Figment` built from the
   file, edit the file, reload → the old value (documents the `Figment` impl).
6. `dynamic_sections_keep_their_base_across_reload` (in `rust/botconf/tests/dynamic.rs`, beside 009's tests) — register a dynamic
   section, edit the file, reload → the section's values are unchanged.

**Verify**: `nice -n 19 cargo test -p botconf --all-features` → Step 0 counts
+ 6, all green. Mutation checks, each must fail a test: (a) `reload` ignores
the new base (keeps the old one) → 1 and 4 fail; (b) `reload` swaps the base
before validating → 2 fails; (c) `unset` layers on the base from `load` → 4
fails.

### Step 4: Wording and docs

- `rust/botconf/src/cli.rs:108` and `rust/botconf-panel/src/edit.rs:53`:
  "Reloaded the configuration: {n} stored setting(s) applied, {m} ignored".
  Update any test asserting the old text.
- README: in "Using it", a short subsection "Reloading" showing
  `SettingsStore::load(schema, move || AppConfig::figment(&path), storage)`,
  saying a `Figment` is a fixed base, a closure is re-read on every reload, an
  invalid file is refused with the reason, and keys the host reads only at
  startup still need a restart. `README.md:254` stays true.
- Crate versions → `0.8.0` (root `Cargo.toml` workspace version and lock).
- `plans/README.md`: 010 row DONE, with notes.

**Verify** (the gate, once): `nice -n 19 cargo test --workspace
--all-features` → all green; `nice -n 19 cargo clippy --workspace --all-targets
--all-features -- -D warnings` → clean; `cargo fmt --check` → clean. Then
commit, tag `v0.8.0` and `go/v0.8.0`, push `main` and both tags, and check
`git ls-remote --tags https://github.com/propheci/botconf` lists them.

### Step 5: The assistant bot re-reads its file

In assistant_bot_rs, on a new branch `botconf-v0.8` from `main`:

- `Cargo.toml`: both botconf dependencies to `tag = "v0.8.0"`; `cargo update
  -p botconf -p botconf-telegram` (or a plain `cargo check` if the update
  refuses, as in 001's notes). Only the botconf sources should move in the
  lock.
- `src/settings/mod.rs:100`: `pub async fn load(base: impl botconf::BaseSource,
  …)`.
- `src/app.rs`: keep the startup `base` for `from_figment` (the file must be
  valid on its own), but pass the stores a source:
  `let source = { let path = cli.config.clone(); move ||
  AssistantConfig::figment(&path) };` and `settings::load(source.clone(), …)`
  at lines 53, 66 and 100 (the closure is `Clone` because it only holds a
  `PathBuf`).
- `README.md:130` stays; `:143` becomes: "A running bot picks up changes made
  from the command line, and edits to the config file, after `/config
  reload` (or on its next start). The bot token, the API URL, the owner and
  the database settings are read only at startup, so changing them needs a
  restart."
- Test in `src/settings/mod.rs` tests, beside `without_overrides_the_file_wins`
  (it already writes a `config.toml` into a `tempfile::tempdir()`):
  `reload_picks_up_file_edits` — load with a closure over the temp path,
  rewrite the file with `sudo_users_id = [9]`, `reload()` →
  `access().is_sudo(UserId(9))`.

**Verify** (inside `direnv exec . sh -c '…'`, niced, 2 jobs):
`cargo test --lib` → previous count (312) + 1; `cargo clippy --all-targets --
-D warnings` and `cargo fmt --check` → clean. Mutation: pass the startup
`Figment` instead of the closure → the new test fails. Commit on the branch
(`feat(settings): reload re-reads the config file`); do not push or merge.

## Test plan

| Test | Crate | Proves |
|---|---|---|
| `reload_reads_an_edited_file` | botconf | the file is re-read |
| `reload_keeps_the_snapshot_when_the_file_becomes_invalid` | botconf | an invalid edit is refused, nothing swapped |
| `reload_keeps_the_snapshot_when_the_source_fails` | botconf | a failing source is refused |
| `overrides_still_win_over_an_edited_file` | botconf | overrides win; later changes use the new base |
| `a_fixed_base_does_not_reread` | botconf | a `Figment` stays a fixed base |
| `dynamic_sections_keep_their_base_across_reload` | botconf | 009's bases are untouched |
| `reload_picks_up_file_edits` | assistant bot | the host passes a re-reading source |

## Done criteria

- [ ] Every existing call site passing a `Figment` compiles unchanged.
- [ ] The 6 new store tests pass, and the three mutations each fail one.
- [ ] Workspace tests, clippy and fmt clean (niced, 2 jobs).
- [ ] `v0.8.0` and `go/v0.8.0` pushed and listed by `git ls-remote`.
- [ ] The assistant bot's branch `botconf-v0.8` committed with its test.
- [ ] `plans/README.md` row 010 DONE with notes.

## STOP conditions

- The drift check shows `store.rs` changed since `v0.7.0` in a way that moves
  the base handling.
- E0119 (conflicting implementations) on the two `BaseSource` impls. Report
  it; the fallback (a `BaseSource::fixed(figment)` wrapper and `impl
  BaseSource for Fn` only) breaks every call site, which is the owner's call.
- Any existing test needs a change other than the reload wording.
- In the assistant bot, the lock moves anything besides the two botconf
  sources.

## Maintenance notes

- The base can now change while the store runs. Anything that caches a value
  derived from the base outside the snapshot (a host reading a key once at
  startup) will not follow a reload; hosts should document those keys.
- wuphf adopts this in its plan 044 (pin `v0.9.0`, after 011).
