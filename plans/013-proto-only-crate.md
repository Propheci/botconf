# Plan 013: A proto-only `botconf-proto` crate

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Build limits (the owner's laptop)**: prefix every cargo command with
> `nice -n 19` and set `CARGO_BUILD_JOBS=2`. Check and test only the crates
> you touch (`-p`), and run the workspace gate once, at the end. Do not run
> `nix flake check`; CI runs it on push. Only one build at a time.
>
> **Drift check (run first)**: `git diff --stat v0.10.0..HEAD -- rust/ proto/`
> Expected: empty. What must still hold:
> - `rust/botconf-grpc/build.rs` compiles `../../proto/botconf/v1/settings.proto`
>   with `build_server(true)` and `build_client(true)`.
> - `rust/botconf-grpc/src/lib.rs:51-53` is `pub mod proto { tonic::include_proto!("botconf.v1"); }`.
> - The workspace version is `0.10.0` (`Cargo.toml:6`).
>
> If any of these is false, STOP.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW. The generated code moves crates and `botconf_grpc::proto`
  keeps its path through a re-export.
- **Depends on**: 012 (`v0.10.0`)
- **Category**: packaging
- **Planned at**: `c6e72c6` (tag `v0.10.0`), 2026-10-31

## Why this matters

wuphf's Rust connector SDK (wuphf plan 054) needs to call a settings service:
Declare, Get and Watch. Today the generated client lives only in
`botconf-grpc`, which also depends on `botconf` and `botconf-panel`. A
connector that only *calls* the service would pull in the whole settings
engine, the panel engine and figment. The owner decided on 2026-10-31 to
publish the generated messages and stubs in their own crate. Go already works
this way: `go/settings` imports only the generated package.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | New crate `rust/botconf-proto` (package `botconf-proto`). It holds only the code generated from `proto/botconf/v1/settings.proto`, with both client and server stubs. Its dependencies are `prost` and `tonic` (plus `tonic-build` at build time), and nothing from botconf. |
| 2 | `botconf-grpc` depends on it and keeps `pub use botconf_proto as proto;`. Every existing `botconf_grpc::proto::…` path still compiles, so hosts need no change. |
| 3 | The release is `v0.11.0` (minor, additive). There is no Go change, but tag `go/v0.11.0` as well, to keep the tags paired as before. |

## Steps

### Step 0: Baseline

`nice -n 19 env CARGO_BUILD_JOBS=2 cargo test -p botconf-grpc`. Record the count.

### Step 1: The crate

- `rust/botconf-proto/Cargo.toml`:
  - workspace `version`, `edition`, `rust-version`, `license` and `repository`;
  - description "Generated messages and gRPC stubs for botconf.v1.Settings";
  - deps `prost` and `tonic` (workspace); build-dep `tonic-build` (workspace).
- `rust/botconf-proto/build.rs`: move `rust/botconf-grpc/build.rs` here as is. The relative proto path is the same depth.
- `rust/botconf-proto/src/lib.rs`:
  ```rust
  //! The messages and gRPC stubs generated from `botconf.v1.Settings`, for
  //! hosts that serve the settings service and clients that call it, without
  //! the settings engine or the panel.
  tonic::include_proto!("botconf.v1");
  ```
- Root `Cargo.toml`: add the member, and add `botconf-proto = { path = "rust/botconf-proto", version = "0.11.0" }` to `[workspace.dependencies]`, next to the other botconf crates.
- `rust/botconf-grpc`:
  - delete `build.rs`;
  - drop `tonic-build` from its build-deps;
  - add `botconf-proto = { workspace = true }`;
  - replace `pub mod proto { … }` with `pub use botconf_proto as proto;`.
- Bump the workspace version to `0.11.0`, and every internal `version = "0.10.0"` pin with it.

**Verify**:
- `cargo test -p botconf-proto -p botconf-grpc` passes with the same count as the baseline.
- `cargo tree -p botconf-proto -e normal` shows no `botconf`, `botconf-panel` or `figment`.
- Mutation: make `botconf-proto` depend on `botconf`. The tree check then shows it. Revert.

Commit: `feat: publish the generated settings protocol as botconf-proto`.

### Step 2: Docs, gate and release

- README: in the crate list, say that `botconf-proto` is for clients that only call the service.
- CHANGELOG, or the release notes the repo keeps: 0.11.0.
- Gate: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
- Tag `v0.11.0` and `go/v0.11.0` and push main with the tags. The owner allows Claude to push botconf main and its tags.
- Set row 013 in `plans/README.md` to DONE.

Commit: `docs: botconf-proto; version 0.11.0`.

## STOP conditions

- The drift check fails.
- Moving the generated code changes any public path other than adding `botconf_proto`. For example, `botconf_grpc::proto::settings_client` stops resolving.
- The test count drops.

## Maintenance notes

- `botconf-proto` regenerates from the same `.proto` as Go, so a protocol change touches only that crate and the Go package.
