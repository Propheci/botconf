# Plan 008 (SPIKE, roadmap): A Go port of the settings store

> **Executor instructions**: This is a design spike, not an implementation
> plan. Its output is a design document (`docs/go-store.md`) and a
> recommendation, reviewed by the owner before any code is written. Do not
> write the store. Update the status row in `plans/README.md` when the
> document is done.
>
> **Drift check (run first)**: plans 001–007 and 009 should have landed
> (`git tag` lists `v0.7.0`). The spike can start earlier, but its answers
> about the wire contract and the panel engine assume their final shape.

## Status

- **Priority**: P3 (roadmap; no program needs it yet)
- **Effort**: M (spike)
- **Risk**: LOW (documents only)
- **Depends on**: 005, 006 (the contract the port must speak), 009
- **Category**: direction
- **Planned at**: 2026-10-25, before 001 landed

## Why this matters

The owner chose a remote design for wuphf: the store lives in the Rust host
and the Go connectors are clients. They also asked for the opposite option
on the roadmap: a Go program that **owns** its settings, with the same
layering (file < environment < stored overrides), validation, runtime keys,
kinds and panel, without a Rust process next to it. A standalone Go bot is
the obvious user.

The risk is the one the remote design avoided: two implementations of the
same semantics drifting apart. The spike must say how to prevent that, or
recommend against the port.

## Questions the spike must answer

1. **Who needs it, concretely?** Name at least one planned Go program that
   would host its own settings. If none, recommend parking the plan.
2. **Scope parity.** Which parts must match the Rust core exactly, and which
   may differ?
   - layering and the "object overrides replace maps" rule;
   - key resolution (`Catalog::resolve`, per-entry keys, `is_entry_name`);
   - `set`/`add`/`extend`/`remove`/`delete_entry`/`unset`/`reload`
     semantics, including how invalid stored overrides are ignored at load
     and parked under dynamic namespaces (009);
   - `Source` reporting;
   - the `Schema` hooks (`derive`, `lint`, `on_change`) in Go form.
3. **Typed config in Go.** How a Go program declares its config and runtime
   keys: generics (`Store[C any]`), struct tags, or explicit descriptors
   (`botconf.RuntimeSetting{Key: …, Kind: botconf.OneOf{…}}`). How
   validation runs: `Validate() error` on the config type, JSON Schema
   generated from the struct, or both.
4. **Config loading.** Which library plays figment's role (koanf, viper, a
   small in-house loader), and whether its env-variable mapping
   (`PREFIX_A__B`) and error messages can match.
5. **Storage.** The same `settings(key, value, updated_by, updated_at)` table
   through `database/sql` (SQLite and Postgres drivers), so a database can
   move between a Rust and a Go host.
6. **The panel.** The engine is Rust (004). Options to evaluate:
   - (a) port the engine to Go (doubles the UI logic: the drift the remote
     design avoided);
   - (b) the Go store serves the `botconf.v1.Settings` RPCs it can
     (`Describe` … `Watch`) and the panel stays Rust, run as a sidecar
     process that is a client of the Go store;
   - (c) no panel for Go hosts at first, only the text commands, which are
     small enough to port.
7. **Preventing drift.** A conformance suite both implementations run:
   language-neutral test vectors (`testdata/conformance/*.json`: a schema
   description, a base config, stored overrides, a sequence of operations,
   and the expected snapshots, sources, ignored and parked keys and errors).
   Estimate how much of `rust/botconf/tests/store.rs` and the dynamic-section
   tests can be expressed this way.
8. **Wire compatibility.** Whether the Go store should serve
   `botconf.v1.Settings` itself, so existing Go clients and renderers work
   against it unchanged.

## Deliverable

`docs/go-store.md` with:
- the answers above, each with a recommendation;
- a sketch of the Go API (types and function signatures only);
- a conformance-suite format with three worked vectors;
- an effort estimate per part, and a go/no-go recommendation.

## Done criteria

- [ ] `docs/go-store.md` exists and answers all eight questions
- [ ] The owner reviewed it (record their decision in `plans/README.md`)

## STOP conditions

- The answer to question 1 is "nobody": write that down, recommend parking,
  and stop.

## Maintenance notes

- If the port goes ahead, its implementation plans should start with the
  conformance suite running against the Rust core, before any Go store code.
