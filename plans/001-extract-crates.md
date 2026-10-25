# Plan 001: Move botconf and botconf-telegram into this repository

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**, in the source repository
> `~/work/side_projects/telegram/assistants/assistant_bot_rs/main` (called
> **ASSISTANT** below):
> `git -C ASSISTANT log --oneline d772d66..HEAD -- crates Cargo.toml Cargo.lock`
> Expected: empty. New commits under `crates/` are fine as long as `crates/`
> still holds exactly `botconf/` and `botconf-telegram/` (`ls ASSISTANT/crates`)
> and both still use `version.workspace = true` style manifests; the subtree
> split in Step 2 carries them over. If `crates/` holds anything else, STOP.
> In this repository (**BOTCONF**, `~/work/side_projects/botconf`):
> `git log --oneline` → only the initial "docs: add README, license and
> implementation plans" commit, and no `rust/` directory.

## Status

- **Priority**: P1 (everything else builds on it)
- **Effort**: M
- **Risk**: LOW for the code (moved verbatim); MED for the switch-over in
  ASSISTANT, which depends on the owner having pushed this repository
- **Depends on**: none. Step 7 (the switch in ASSISTANT) also needs the
  remote `github.com/propheci/botconf` to exist and hold the `v0.1.0` tag.
- **Category**: migration
- **Planned at**: ASSISTANT commit `d772d66`, 2026-10-25

## Why this matters

The settings framework was built inside the assistant bot and extracted into
two generic crates there (`crates/botconf`, `crates/botconf-telegram`). A
second program, the wuphf messaging bridge, is going to host runtime settings
too, and its Go connectors need a client and panel renderers. One repository
for the framework (Rust crates, the wire contract and the Go module) keeps
the two languages from drifting, gives both bots the same versioned
dependency, and stops the assistant bot from being the only place the code
can change.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | One repository, `github.com/propheci/botconf`, laid out as `rust/`, `proto/` and `go/`. |
| 2 | Both crates move with their git history. |
| 3 | The assistant bot then depends on this repository through git, pinned to a tag, and deletes its copies. No two copies of the code. |
| 4 | Git dependencies only (no crates.io, no Go proxy release) until the API settles. Rust tags are `v0.x.y`; Go tags are `go/v0.x.y` on the same commit. |
| 5 | MIT license. |
| 6 | The executor never adds remotes and never pushes. The owner creates `github.com/propheci/botconf` and pushes. |

Decided while planning (the owner did not settle these; reviewers may push
back):

- **The Cargo workspace is at the repository root**, with
  `members = ["rust/*"]`, not a workspace inside `rust/`. Reasons: one
  `Cargo.lock` and one `target/` at the root; `cargo test` and the Nix flake
  work from the root like in every other Rust repo the owner has; crane's
  `cleanCargoSource ./.` picks up the whole workspace; a cargo git dependency
  finds a package by name anywhere in the repository, so consumers are not
  affected either way; the Go module in `go/` has its own `go.mod` and does
  not see Cargo files. `rust/` still separates the languages, as decided.
- **History moves with `git subtree split`**, which ships with git (checked:
  `git subtree -h` works in the dev environment; `git filter-repo` is not
  installed). The split carries the four commits that touched `crates/`
  (`f97c0e4`, `cb8b499`, `31e5962`, `93d2a3d`). Older history, from before the
  code was extracted out of `src/settings`, stays in the assistant bot; the
  first commit's message already says where the code came from.

## Current state

ASSISTANT at `d772d66`:

- `crates/` holds exactly `botconf/` and `botconf-telegram/`. History:

  ```
  93d2a3d docs: an example bot for the settings panel, and the crates in the README
  31e5962 feat(botconf-telegram): move the settings panel into a generic crate
  cb8b499 feat(botconf): a clap settings subcommand behind the `cli` feature
  f97c0e4 feat(botconf): extract the settings core into a generic crate
  ```

- Root `Cargo.toml` is a workspace (`members = [".", "crates/*", "migration"]`,
  `resolver = "3"`) with:

  ```toml
  [workspace.package]
  version = "0.1.0"
  edition = "2024"
  rust-version = "1.98"
  license = "MIT"

  [workspace.dependencies]
  sea-orm = { version = "2.0.3", default-features = false, features = [
    "macros", "with-chrono", "runtime-tokio-rustls", "sqlx-postgres", "sqlx-sqlite",
  ] }
  sea-orm-migration = { ... }          # not used by the crates
  tokio = { version = "1.53.1", features = ["full"] }
  anyhow = "1.0.104"
  arc-swap = "1.9.2"
  chrono = { version = "0.4.45", default-features = false, features = ["clock", "serde"] }
  clap = { version = "4.6.7", features = ["derive", "env"] }
  figment = { version = "0.10.19", features = ["env", "toml"] }
  futures = "0.3.34"
  serde = { version = "1.0.229", features = ["derive"] }
  serde_json = { version = "1.0.151", features = ["preserve_order"] }
  teloxide = { version = "0.17.0", default-features = false, features = [
    "cache-me", "ctrlc_handler", "macros", "rustls", "throttle", "tracing",
  ] }
  thiserror = "2.0.21"
  tracing = "0.1.44"
  ```

  The comment above `serde_json` explains `preserve_order` (the assistant's
  AI schemas); keep the feature, it changes the JSON the crates render.

- `crates/botconf/Cargo.toml` inherits `version`, `edition`, `rust-version`,
  `license` and every dependency version from the workspace (`arc-swap`,
  `figment`, `futures`, `serde`, `serde_json`, `thiserror`, `tokio`, `tracing`;
  optional `chrono`, `clap`, `sea-orm`; dev `figment` with `features =
  ["test"]`, `tokio` with `features = ["test-util"]`). Features: `sea-orm`,
  `cli`.
- `crates/botconf-telegram/Cargo.toml` does the same, with
  `botconf = { path = "../botconf" }`, `anyhow`, `futures`, `serde_json`,
  `teloxide`, `thiserror`, `tracing`; dev `figment`, `serde`, `tokio`
  (`test-util`). It has `examples/panel_bot.rs`.
- The root `[package]` (the bot) depends on
  `botconf = { path = "crates/botconf", features = ["cli", "sea-orm"] }` and
  `botconf-telegram = { path = "crates/botconf-telegram" }` (`Cargo.toml:56-57`).
  `migration/` does not depend on botconf: it creates the `settings` table
  itself (`migration/src/m20260926_000001_create_settings.rs`).
- Tooling to copy: `rust-toolchain.toml` (channel `1.98.1`, no rustfmt,
  nightly rustfmt from the flake because `rustfmt.toml` uses unstable
  options), `rustfmt.toml`, `taplo.toml`, `flake.nix` (fenix + crane +
  flake-utils), `nix/build.nix` (crane checks: clippy with `--deny warnings`,
  tests, rustfmt, taplo), `.github/workflows/ci.yml` (`nix flake check`).
- Test counts, measured on a scratch copy (rustc 1.95, locked versions):
  `botconf` 9 unit + 10 integration (`tests/store.rs`) + 1 doc with default
  features, 13 + 10 + 1 with `--all-features`; `botconf-telegram` 21.
- ASSISTANT's README has an "Architecture" tree listing `crates/botconf` and
  `crates/botconf-telegram` (around line 513).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Build | `cargo build --workspace --all-targets --all-features` | exit 0, no warnings |
| Tests | `cargo test --workspace --all-features` | exit 0; 13 + 10 + 1 (botconf) and 21 (botconf-telegram) |
| Lint | `cargo clippy --workspace --all-targets --all-features -- --deny warnings` | exit 0 |
| Format | `cargo fmt --check` (nightly rustfmt, from `nix develop`) | exit 0 |
| TOML format | `taplo fmt --check` | exit 0 |
| Nix checks | `nix flake check --print-build-logs` | exit 0 |

Run everything inside `nix develop` (Step 4 adds the flake).

## Scope

**In scope**:
- BOTCONF: `rust/botconf/**`, `rust/botconf-telegram/**` (moved), root
  `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`,
  `taplo.toml`, `flake.nix`, `flake.lock`, `nix/build.nix`, `.envrc`,
  `.github/workflows/ci.yml`, `README.md`, `plans/README.md`.
- ASSISTANT (Step 7 only): root `Cargo.toml`, `Cargo.lock`, deleting
  `crates/`, `README.md`, `nix/build.nix` only if crane needs a change for git
  dependencies.

**Out of scope** (do NOT touch):
- Any code change in the crates beyond what the move forces (manifest
  fields). No renames, no API changes, no MSRV change (plan 002).
- `proto/`, `go/` (plans 005–007).
- ASSISTANT's application code (`src/`, `migration/`).

## Git workflow

- BOTCONF: commit on `main` (the repository has no other branches).
  1. the subtree merge commit (made by `git subtree add`)
  2. `build: add the Cargo workspace and the Nix dev shell for the crates`
  3. `ci: run the Nix checks on GitHub Actions`
  4. `docs: describe the crates and how to depend on them`
  Then tag `v0.1.0` locally (annotated: `git tag -a v0.1.0 -m "botconf 0.1.0"`).
- ASSISTANT: a branch `botconf-git-dependency`, one commit
  `build: depend on botconf from its own repository`. The owner merges.
- Do NOT add remotes, push, or push tags.

## Steps

### Step 0: Record the baseline

In ASSISTANT: `cargo test -p botconf --all-features 2>&1 | grep "^test result"`
and `cargo test -p botconf-telegram 2>&1 | grep "^test result"`. Record the
numbers (13/10/1 and 21 expected at `d772d66`; more if the drift check showed
new commits).

### Step 1: Split the crates' history in ASSISTANT

```sh
cd ASSISTANT
git subtree split --prefix=crates -b botconf-split
git log --oneline botconf-split
```

**Verify**: the log lists the commits that touched `crates/` (the four above,
plus any new ones), with rewritten hashes; `git ls-tree --name-only
botconf-split` → `botconf` and `botconf-telegram`. The branch is local and
temporary; it is deleted in Step 7.

### Step 2: Add the history under `rust/` in BOTCONF

```sh
cd BOTCONF
git subtree add --prefix=rust ASSISTANT botconf-split
```

(`ASSISTANT` is the local path; `git subtree add` accepts a repository path
and a ref.)

**Verify**: `ls rust` → `botconf  botconf-telegram`; `git log --oneline -- rust`
lists the carried commits under a merge commit "Add 'rust/' from commit …";
`git log --oneline --follow rust/botconf/src/store.rs | wc -l` ≥ 1.

### Step 3: Make the workspace

Create the root `Cargo.toml`:

```toml
[workspace]
members = ["rust/*"]
resolver = "3"

[workspace.package]
version = "0.1.0"
edition = "2024"
# Unchanged from the assistant bot for now; plan 002 sets the real minimum.
rust-version = "1.98"
license = "MIT"
repository = "https://github.com/propheci/botconf"

[workspace.dependencies]
botconf = { path = "rust/botconf" }

anyhow = "1.0.104"
arc-swap = "1.9.2"
chrono = { version = "0.4.45", default-features = false, features = ["clock", "serde"] }
clap = { version = "4.6.7", features = ["derive", "env"] }
figment = { version = "0.10.19", features = ["env", "toml"] }
futures = "0.3.34"
sea-orm = { version = "2.0.3", default-features = false, features = [
  "macros",
  "with-chrono",
  "runtime-tokio-rustls",
  "sqlx-postgres",
  "sqlx-sqlite",
] }
serde = { version = "1.0.229", features = ["derive"] }
# `preserve_order`: JSON objects keep the order they are written in. The
# assistant bot relies on it for its AI schemas, and values render in the
# order they were declared.
serde_json = { version = "1.0.151", features = ["preserve_order"] }
teloxide = { version = "0.17.0", default-features = false, features = [
  "cache-me",
  "ctrlc_handler",
  "macros",
  "rustls",
  "throttle",
  "tracing",
] }
thiserror = "2.0.21"
tokio = { version = "1.53.1", features = ["full"] }
tracing = "0.1.44"
```

Then:

- In `rust/botconf-telegram/Cargo.toml`, replace
  `botconf = { path = "../botconf" }` with `botconf = { workspace = true }`.
- Add `repository.workspace = true` to both crate manifests (after
  `license.workspace = true`).
- Copy ASSISTANT's `Cargo.lock` to the root and run `cargo build --workspace
  --all-targets --all-features`. Cargo drops the entries the crates do not
  use and keeps every version the assistant bot tested with.
- Copy `rust-toolchain.toml`, `rustfmt.toml` and `taplo.toml` from ASSISTANT.
  In `taplo.toml`, set `include = ["*.toml", "rust/*/*.toml"]`.

**Verify**: `cargo build --workspace --all-targets --all-features` → exit 0, no
warnings; `cargo test --workspace --all-features` → the Step 0 numbers;
`git diff --stat` shows no change under `rust/*/src`.

### Step 4: Add the Nix flake

Copy ASSISTANT's `flake.nix` and `nix/build.nix` and trim them to a library
workspace:

- `flake.nix`: description "botconf: runtime-editable, typed settings"; keep
  `nixpkgs`, `flake-utils`, `fenix`, `crane`; keep `rustToolchain` (from
  `rust-toolchain.toml`, same `sha256`), `nightlyRustfmt` and the crane
  setup; drop `apps.tuya-local-key`, `nixosModules` and `packages.default`;
  `checks = build.checks`; `devShells.default` with `nightlyRustfmt` and
  `pkgs.taplo` (no `sea-orm-cli`, no `sqlite`).
- `nix/build.nix`: the crates are libraries, so there is no `package`. Use
  `pname = "botconf"; version = "0.1.0";` explicitly (there is no root
  `[package]` for `crateNameFromCargoToml`). Keep `cargoArtifacts =
  craneLib.buildDepsOnly commonArgs`, and the `clippy` (`--workspace
  --all-targets --all-features -- --deny warnings`), `tests` (`--workspace
  --all-features`), `rustfmt` and `taplo` checks.
- `.envrc` with `use flake`.

Run `nix flake lock` to create `flake.lock`.

**Verify**: `nix flake check --print-build-logs` → exit 0;
`nix develop -c cargo fmt --check` → exit 0.

Commit 2 (Steps 3–4).

### Step 5: CI

Copy `.github/workflows/ci.yml` from ASSISTANT unchanged (it runs
`nix flake check`). Plan 002 adds the minimum-version jobs.

**Verify**: `taplo fmt --check` still passes; the workflow file is valid YAML
(`nix run nixpkgs#yq-go -- '.' .github/workflows/ci.yml >/dev/null` → exit 0).

Commit 3.

### Step 6: Document and tag

- `README.md`: in "Status", say the crates are here (moved from the
  assistant bot with history); in "Using it", show the two crates with their
  features:

  ```toml
  botconf = { git = "https://github.com/propheci/botconf", tag = "v0.1.0", features = ["cli", "sea-orm"] }
  botconf-telegram = { git = "https://github.com/propheci/botconf", tag = "v0.1.0" }
  ```

- Update this plan's row in `plans/README.md`.

Commit 4, then `git tag -a v0.1.0 -m "botconf 0.1.0: the crates from assistant_bot_rs"`.

**Verify**: `git tag` → `v0.1.0`; `git status` clean.

### Step 7: Switch the assistant bot to the git dependency

**STOP before this step until the owner confirms that
`https://github.com/propheci/botconf` exists and has the `v0.1.0` tag**
(`git ls-remote --tags https://github.com/propheci/botconf v0.1.0` prints a
line). Ask the owner; do not push it yourself.

In ASSISTANT, on a new branch `botconf-git-dependency`:

1. Root `Cargo.toml`: `members = [".", "migration"]`; replace the two path
   dependencies with

   ```toml
   botconf = { git = "https://github.com/propheci/botconf", tag = "v0.1.0", features = ["cli", "sea-orm"] }
   botconf-telegram = { git = "https://github.com/propheci/botconf", tag = "v0.1.0" }
   ```

2. `git rm -r crates`.
3. `cargo update -p botconf -p botconf-telegram` (only these; the other
   locked versions must not move).
4. README "Architecture": drop the `crates/` lines and say the settings
   framework comes from `github.com/propheci/botconf`.
5. `git branch -D botconf-split` (the temporary branch from Step 1).

**Verify**: `cargo test --workspace` → the assistant's full suite passes with
the same total as before the switch minus the moved crates' tests;
`git diff Cargo.lock` only changes the two packages' `source` lines (and
removes nothing else); `nix flake check` → exit 0 (crane fetches git
dependencies from the lock file; if the repository is private this fails —
see STOP conditions).

Commit, and leave the branch for the owner to merge.

## Test plan

No new tests: the code moves unchanged. The moved suites are the regression
net: `botconf` (unit, `tests/store.rs`, doc test) and `botconf-telegram`
(21), plus the assistant bot's own suite after Step 7, which exercises both
crates through `src/settings` and the `/config` module.

## Done criteria

- [ ] `git log --oneline -- rust` in BOTCONF shows the carried commits
- [ ] `cargo test --workspace --all-features` in BOTCONF matches the Step 0
      numbers; clippy (deny warnings), `cargo fmt --check`, `taplo fmt
      --check` and `nix flake check` pass
- [ ] `git diff <subtree merge>..HEAD -- rust/*/src` is empty (code moved
      verbatim)
- [ ] `v0.1.0` tag exists locally
- [ ] ASSISTANT branch `botconf-git-dependency` builds and passes its tests
      against the git dependency, with `crates/` removed (after the owner
      pushed the repository)
- [ ] `plans/README.md` row updated

## STOP conditions

- The drift check shows `crates/` holding anything besides the two crates.
- Moving the crates needs a source change (anything under `rust/*/src`).
- `cargo build` in BOTCONF wants different dependency versions than
  ASSISTANT's `Cargo.lock` had (other than dropping unused entries).
- Step 7 is reached and the remote does not exist or lacks the tag.
- `nix flake check` in ASSISTANT cannot fetch the git dependency (e.g. the
  repository is private): report; the fix (making it public, or
  `outputHashes`/an access token for crane) is the owner's call.

## Maintenance notes

- `tokio` keeps the `full` feature inherited from the assistant bot. The
  crates only need `sync` (and `macros`/`rt` in tests); trimming it is a
  separate change, worth doing before publishing.
- Plan 002 lowers `rust-version` to the real minimum and adds the
  minimum-version CI jobs.
- Further history: the code's earlier life in the assistant bot's
  `src/settings/` is not carried. If ever needed, `git filter-repo` with path
  renames over ASSISTANT can rebuild it.
