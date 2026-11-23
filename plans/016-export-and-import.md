# Plan 016: Export and import: back up the effective settings (file + stored overrides) as one commented TOML

> **Executor instructions**: as plan 015. Build limits: `nice -n 19`,
> `CARGO_BUILD_JOBS=2`, `-p` the crates touched until the gate; the gate runs
> once, in the dev shell. Go: `GOMAXPROCS=2 GOFLAGS=-p=2 nice -n 19`. One
> build at a time.

## Status

- **Priority**: P1 (wuphf plan 061 needs it)
- **Effort**: M–L
- **Risk**: MED — additive; an import rewrites a config file and the stored
  overrides, so it validates everything before writing anything
- **Depends on**: 015 (the description, `render`, `botconf-tui`)
- **Category**: feature
- **Planned at**: 015 in progress, 2026-11-23

## Why this matters

A host's settings live in two places: the config file and the overrides in
its database (changed from the panel). The owner wants one backup that
holds both, readable, and a restore that brings them back (2026-11-23).

## Design decisions (settled with the owner, 2026-11-23)

| # | Decision |
|---|---|
| 1 | **An export is one TOML file, written from the description**: the effective configuration (the base — the file and the environment — with every stored override applied) rendered like the example, so it reads as a config file; then the overrides kept under the host's dynamic namespaces (e.g. wuphf's `connector_settings.<id>`, which the host's own file must not hold) as their own tables, by their dotted keys. |
| 2 | **Secrets only with a flag**: by default the export names the environment variables (`ValueKind::EnvVar`) but holds no values; with secrets it adds a reserved `[env_file]` table of each named variable's value (from the env file, else the process environment). A `Secret` value in the file itself is exported as is only with the flag; without it, it is commented out with `<redacted>`. |
| 3 | **Import restores exactly**: it reads the backup, splits off the dynamic overrides and `[env_file]`, checks the rest with the host's validator (its own loader), then writes the config file (rendered, so it reads like the example), removes the stored overrides of the host's own keys (now in the file), stores the dynamic overrides (replacing those of the same namespaces), and sets the env file's variables (keeping its other lines). Nothing is written if any check fails. The written files are backed up first (`<file>.bak`). |
| 4 | **Offline and online**: offline, the host gives the description, its base `Figment`, its `Storage` and its dynamic namespaces (`botconf::export::{export, import}`). Online, `botconf.v1.Settings` gains `Export` (the backup, never with secrets: they would cross the wire) and `Import` (only what can change at runtime: runtime keys and dynamic overrides as overrides, validated like a `Set`; the other differences are listed as needing an offline import). The Rust CLI (`cli` feature) and the Go client get `export` and `import`. |
| 5 | `botconf-tui` gets Export and Import actions over the same functions. |
| 6 | Release with 015, or as `v0.14.0` / `go/v0.14.0` if 015 is already out. |

## Steps

1. **`botconf::export`**: the effective values (base + overrides, the store's
   layering, without a running store); `export(description, base, storage,
   namespaces, secrets) -> String`; `import(…)` with the validator, the
   `.bak` files and the all-or-nothing order. Tests: round trip (export,
   import into fresh files and a memory storage, same effective values and
   overrides); unknown keys kept as comments; a failing validator writes
   nothing; secrets absent without the flag.
2. **gRPC**: `Export` and `Import` in `proto/botconf/v1`, `botconf-grpc`
   handlers (owner-only, like `Set`), the CLI's `export` / `import`; the Go
   client's. Tests on the service.
3. **`botconf-tui`**: Export / Import actions (a path prompt; a summary of
   what an import changes before it writes).
4. **Docs, gate, release**, row 016 DONE.

## STOP conditions

- The store's layering cannot be reproduced offline without duplicating it
  (factor it out of `SettingsStore` instead; report if that changes its API).
