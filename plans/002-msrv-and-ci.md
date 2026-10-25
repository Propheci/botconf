# Plan 002: Declare the real minimum Rust versions and test them in CI

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git log --oneline v0.1.0..HEAD -- Cargo.toml rust/*/Cargo.toml Cargo.lock .github nix flake.nix`
> Expected: empty, or only plan bookkeeping. What must still hold: the root
> `Cargo.toml` has `[workspace.package] rust-version = "1.98"` and both crate
> manifests say `rust-version.workspace = true`; `.github/workflows/ci.yml`
> has the single `nix flake check` job; `Cargo.lock` has `takecell 0.1.2`
> (`grep -A1 'name = "takecell"' Cargo.lock`). Otherwise STOP.

## Status

- **Priority**: P1 (wuphf's toolchain is rustc 1.95; it cannot build a
  dependency that declares 1.98)
- **Effort**: S
- **Risk**: LOW — manifest metadata and CI only
- **Depends on**: 001
- **Category**: build
- **Planned at**: before 001 landed, from a scratch build of the crates at
  assistant_bot_rs `d772d66`, 2026-10-25

## Why this matters

The crates inherited `rust-version = "1.98"` from the assistant bot's
workspace, which pins its own toolchain at 1.98.1. That number is not what
the code needs. wuphf builds with rustc 1.95 (the nixpkgs toolchain in its dev
shell), and cargo refuses to build a dependency whose `rust-version` is
newer than the compiler. Declaring the real minimum and testing it in CI
keeps botconf usable from both bots and catches accidental use of newer
features.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | `botconf`: `rust-version = "1.95"`, the lowest version that builds and passes. |
| 2 | `botconf-telegram`: `rust-version = "1.96"`, because `takecell 0.1.2` (pulled in by `teloxide-core 0.13.0` from `teloxide 0.17.0`) declares 1.96. wuphf does not use teloxide, so this costs nothing there. |
| 3 | CI tests on the minimum versions **and** on the pinned stable toolchain. |

## Current state

Measured on a scratch copy of both crates with `rust-version` lowered, the
assistant bot's `Cargo.lock`, and the nix rustc 1.95.0:

| Build | Result |
|---|---|
| `botconf`, default features | ok: 9 unit, 10 integration, 1 doc test |
| `botconf --features cli` | ok: 12, 10, 1 (1 ignored) |
| `botconf --features sea-orm` | ok: 10, 10, 1 |
| `botconf --all-features` | ok: 13, 10, 1 (1 ignored) |
| `botconf-telegram` | refused before compiling: `error: rustc 1.95.0 is not supported by the following package: takecell@0.1.2 requires rustc 1.96` |
| `botconf-telegram --ignore-rust-version` | ok, 21 tests |
| `botconf-telegram` with `cargo update takecell --precise 0.1.1` | ok, 21 tests, no flags |

No source file needs a language or library feature newer than 1.95; the only
obstacle is takecell's declared version, not a real code need.

- Root `Cargo.toml` (after 001): `[workspace.package] rust-version = "1.98"`,
  `resolver = "3"`.
- `rust-toolchain.toml`: `channel = "1.98.1"` (the dev toolchain; keep it).
- `.github/workflows/ci.yml`: one job, `nix flake check` (build, clippy with
  warnings denied, tests, rustfmt, taplo).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Stable tests | `cargo test --workspace --all-features` | exit 0 |
| 1.95 tests, botconf | `nix shell nixpkgs#rustc nixpkgs#cargo -c cargo test -p botconf --all-features --locked` (or `cargo +1.95.0 …` with rustup) | exit 0 |
| 1.96 tests, botconf-telegram | `cargo +1.96.0 test -p botconf-telegram --locked` (rustup) | exit 0 |
| Nix checks | `nix flake check --print-build-logs` | exit 0 |

Check the nixpkgs rustc version first: `nix shell nixpkgs#rustc -c rustc --version`.
If it is not 1.95.x, use fenix in the flake instead (Step 3 shows how).

## Scope

**In scope**: root `Cargo.toml`, `rust/botconf/Cargo.toml`,
`rust/botconf-telegram/Cargo.toml`, `.github/workflows/ci.yml`, `flake.nix`
(an MSRV check), `README.md`, `plans/README.md`.

**Out of scope**: source changes; changing `rust-toolchain.toml`; the Go job
(plan 006 adds it).

## Git workflow

Commit on `main`:
1. `build: declare the minimum Rust versions the crates need`
2. `ci: test the crates on their minimum Rust versions`

## Steps

### Step 1: Declare the versions

- Root `Cargo.toml`: `rust-version = "1.95"` in `[workspace.package]`, with a
  comment: `# The lowest version that builds and passes; CI tests it.`
- `rust/botconf/Cargo.toml`: keep `rust-version.workspace = true`.
- `rust/botconf-telegram/Cargo.toml`: replace with `rust-version = "1.96"`
  and a comment: `# takecell 0.1.2 (teloxide-core 0.13) declares 1.96; the
  crate itself needs no more than the workspace's 1.95.`

**Verify**: `cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | "\(.name) \(.rust_version)"'`
→ `botconf 1.95` and `botconf-telegram 1.96`. `cargo test --workspace
--all-features` → exit 0, same numbers as before.

Note on `resolver = "3"`: it is MSRV-aware and prefers dependency versions
compatible with the workspace's lowest `rust-version`, so a fresh
`cargo update` may pick `takecell 0.1.1`. Either version is fine; do not run
`cargo update` in this plan (the lock file must not change).

### Step 2: Prove the minimums locally

Run the two minimum-version commands above.

**Verify**: both exit 0 with the numbers from "Current state".

Commit 1.

### Step 3: CI jobs

Add two jobs to `.github/workflows/ci.yml`, next to the `nix flake check`
job, using `dtolnay/rust-toolchain` (no Nix needed for these):

```yaml
  msrv:
    name: minimum Rust (${{ matrix.package }} on ${{ matrix.toolchain }})
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        include:
          - package: botconf
            toolchain: "1.95.0"
            args: --all-features
          - package: botconf-telegram
            toolchain: "1.96.0"
            args: ""
    steps:
      - uses: actions/checkout@v7
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.toolchain }}
      - run: cargo test --locked -p ${{ matrix.package }} ${{ matrix.args }}
```

The stable side is already covered by `nix flake check` (the pinned 1.98.1
toolchain).

**Verify**: the YAML parses (`nix run nixpkgs#yq-go -- '.' .github/workflows/ci.yml >/dev/null`
→ exit 0). The jobs themselves run once the owner pushes; ask them to check
the first run.

### Step 4: Document

README "Using it": add "Minimum Rust: 1.95 for `botconf`, 1.96 for
`botconf-telegram`." Update this plan's row in `plans/README.md`.

Commit 2. Tag `v0.1.1` locally (`git tag -a v0.1.1 -m "botconf 0.1.1: minimum Rust 1.95"`),
since wuphf will pin a tag that has the lowered version.

## Test plan

No new tests. The existing suites run on three toolchains: 1.95 (botconf),
1.96 (botconf-telegram) and the pinned stable (everything, via Nix).

## Done criteria

- [ ] `cargo metadata` shows `botconf 1.95`, `botconf-telegram 1.96`
- [ ] Both minimum-version test commands pass locally
- [ ] `ci.yml` has the `msrv` matrix; `nix flake check` passes
- [ ] `Cargo.lock` unchanged (`git diff v0.1.0 -- Cargo.lock` empty)
- [ ] `v0.1.1` tag exists locally; `plans/README.md` row updated

## STOP conditions

- `botconf` fails to build or test on 1.95: report the shortest decisive
  error line and the feature involved. Do not raise the declared version
  without the owner.
- `botconf-telegram` fails on 1.96 for a reason other than a dependency's
  declared version.
- Any step needs a source change.

## Maintenance notes

- When `teloxide` updates, re-check `cargo tree -p botconf-telegram -e normal`
  for dependencies whose `rust-version` is above 1.96 and adjust the
  declared version, with a comment saying which dependency forced it.
- Plans 004 and 005 add crates (`botconf-panel`, `botconf-grpc`); each must
  add itself to the `msrv` matrix at 1.95 (wuphf builds them with 1.95).
- Plan 006 adds a Go job to this workflow.
