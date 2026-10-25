# Plan 009: Owned descriptors and sections registered at runtime

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Order**: this plan runs **after 003 and before 004**, despite its number
> (see `plans/README.md`). It was added after 004–008 were written, and
> other repositories' plans cite 004 and 005 by number, so it was appended
> rather than renumbering them.
>
> **Drift check (run first)**: `git diff --stat v0.2.0..HEAD -- rust/`
> Expected: empty (plan 003 tagged `v0.2.0`). What must still hold:
> `rust/botconf/src/kind.rs` has `#[derive(Clone, Copy, …)] pub enum Kind`
> with `Kind::Ref { domain: &'static str, many: bool }`, `Form(&'static Form)`,
> `Map { names: Option<Choices>, value: &'static Kind }`;
> `rust/botconf/src/keys.rs` has `RuntimeSetting` with `key: &'static str` and
> `const fn` builders; `Catalog::new(&[RuntimeSetting], &[Section])`;
> `SettingsStore::catalog(&self) -> &Catalog`; the panel engine
> (`rust/botconf-panel`) does **not** exist yet. If 004 already landed, STOP:
> its engine must then be adapted as part of this plan, which needs
> re-planning.

## Status

- **Priority**: P1 (wuphf's connector settings need it; 004 and 005 are
  written to sit on it)
- **Effort**: L
- **Risk**: MED-HIGH — changes the core's descriptor types (every kind
  declaration) and makes the catalog mutable; the store's existing tests are
  the safety net
- **Depends on**: 003
- **Category**: API / architecture
- **Planned at**: assistant_bot_rs `d772d66` sources (the crates as moved by
  001), 2026-10-25

## Why this matters

Today every runtime setting is known at compile time: `RuntimeSetting`,
`Kind`, `Form` and `FixedChoice` hold `&'static` data, sections are typed
Rust structs (`SectionSettings::of::<T>`) declared once by the `Schema`, and
the `Catalog` is built once in `SettingsStore::load`.

wuphf needs settings that only exist at runtime. Each Go connector (Telegram,
WhatsApp, Discord, …) declares its own settings when it registers with the
core: a section per connector, described by the connector, validated with
the JSON Schema the connector sends (the core cannot deserialize Go types),
whose base values come from the connector's own TOML file (sent at
registration), not from the host's config file. The core owns storage and
validation, and pushes changes back to the connector.

So botconf must:

1. describe settings with **owned** data, so descriptors built at runtime can
   be stored (while keeping `const` declarations for compile-time settings);
2. let a host **add, replace and remove sections after `load`**;
3. validate such a section with a **pluggable validator** instead of a Rust
   type;
4. take the section's **base values from a per-section provider** rather than
   the host's figment;
5. keep stored overrides for a section that has not registered yet (the
   connector is offline at startup) instead of discarding them as unknown.

Plan 005's `Describe` RPC serializes descriptors; it works the same for
static and dynamic sections once descriptors are owned and choices are
resolved (see "Wire consistency").

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | The store lives in the host (wuphf's core) and stays authoritative over storage and validation. |
| 2 | Connector settings are declared by the connector at registration, with a JSON Schema; the host validates with a JSON Schema validator. |
| 3 | Values are pushed down to the connector (plan 005's `Watch`); the connector's TOML file stays its base and bootstrap. |
| 4 | Only keys that apply live are runtime keys (restart-only keys stay file-only); a `restart` flag may come later. |

Decided while planning. Items marked "confirmed" were confirmed by the owner
on 2026-10-25; the others are open to review:

- **`Shared<T>` for descriptor data** (confirmed by the owner 2026-10-25). A small enum
  `Shared<T: ?Sized + 'static> { Static(&'static T), Owned(Arc<T>) }`
  (`Clone`, `Deref<Target = T>`, `PartialEq` by value). `Kind`, `Form`,
  `Field`, `FixedChoice` and `RuntimeSetting` hold `Shared<str>`,
  `Shared<[…]>`, `Shared<Kind>` and `Shared<Form>` instead of `&'static`.
  `Static` keeps `const` declarations possible; `Owned` serves runtime
  descriptors. `Cow<'static, Kind>` cannot be used: its owned variant would
  hold a `Kind` inside a `Kind` (an infinitely sized type); `Arc` gives the
  indirection. `Kind` becomes `Clone` but no longer `Copy`.
- **`const` builders survive with `mem::forget`.** A `const fn` may not drop
  a value whose type has drop glue (`Shared` does, through `Arc`). Builders
  such as `titled(mut self, …)` would drop the old field, so they destructure
  `self`, `core::mem::forget` the replaced field (a `const fn` since Rust
  1.46), and rebuild. Every declaration in the assistant bot stays a `const`.
  If this does not compile on 1.95, STOP (see conditions).
- **The catalog is append-only.** Registering a section appends its entries;
  replacing a section whose descriptors are unchanged keeps its positions;
  replacing it with different descriptors tombstones the old positions and
  appends new ones; removing tombstones them. Positions never move, so the
  panel engine's tokens (004, which encode positions) stay valid for every
  setting that still exists, and a stale token simply finds a tombstone.
- **Dynamic sections never enter the host's figment.** `S::Config` usually
  has `deny_unknown_fields`; merging a connector's values into the main
  figment would break it. Each dynamic section has its own small figment:
  its base (from the provider) with its overrides merged on top, using the
  same `Override` provider and object-clearing rule as the main build.
- **Dynamic namespaces.** `Schema::dynamic_namespaces()` (default empty)
  lists key prefixes, e.g. `connectors`, under which sections may register
  later. A stored override under such a prefix with no registered section is
  **parked** (kept, listed as "waiting for its section"), not ignored. When
  the section registers, parked overrides are applied one by one; invalid
  ones move to `ignored` with the reason, as at load.
- **The validator returns the normalized value.** `Validator::validate(&Value)
  -> Result<Value, String>`: the section's effective value (with whatever
  defaults the validator fills) or the reason it is invalid. botconf ships
  no JSON Schema implementation; the host plugs one in (wuphf uses a JSON
  Schema crate). A `jsonschema` feature can be added later.

## Current state

(Line numbers from assistant_bot_rs `d772d66`; plan 003 changed `kind.rs`.)

- `rust/botconf/src/kind.rs` — `#[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Kind` (line 16) with `Text { optional }`, `Number { min, max, unit:
  &'static str, suggestions: &'static [i64], optional }`, `OneOf { choices,
  custom, optional }`, `SetOf { choices, inverted }`, `Ref { domain:
  &'static str, many }` (plan 003), `Form(&'static Form)`, `Map { names:
  Option<Choices>, value: &'static Kind }`. `Form { fields: &'static [Field],
  initial: &'static str }` (`Copy`), `Field { key, title: &'static str, kind:
  Kind, group: Option<&'static str> }` with `const fn new` and `const fn
  group`. `Choices { Fixed(&'static [FixedChoice]), KeysOf(&'static str),
  Dynamic(DynamicChoices) }`; `DynamicChoices(pub fn(&dyn View) ->
  Vec<Choice>)`; `FixedChoice { value, label: &'static str }` with
  `const fn new`; `Choice { value, label: String }` (owned already).
- `rust/botconf/src/keys.rs` — `#[derive(Clone, Copy, …)] pub struct
  RuntimeSetting { key: &'static str, title: Option<&'static str>,
  description: &'static str, per_entry: bool, kind: Kind }` with `const fn
  new`, `per_entry(key, description, value: &'static Kind)`, `titled`,
  `kind`, `entry_names`. `CatalogEntry { key: String, title: String,
  description: &'static str, per_entry, kind, section: Option<&'static str> }`;
  `Catalog { entries: Vec<CatalogEntry> }` with `new(settings, sections)`,
  `entries()` ("positions never change while the program runs") and
  `resolve(key)`.
- `rust/botconf/src/section.rs` — `Section { id: &'static str, path: String,
  title, description: &'static str, settings: SectionSettings }`;
  `SectionSettings { runtime: &'static [RuntimeSetting], parse: fn(&Value) ->
  Result<ParsedSection, String> }` with `of::<T>()`, `parse`, `check`;
  `ParsedSection { typed: Box<dyn Any>, json: Value }`.
- `rust/botconf/src/store.rs` — `SettingsStore { schema, sections:
  Arc<[Section]>, base: Figment, storage, catalog: Catalog, current:
  ArcSwap<Snapshot<S>>, write_lock }`; `load` builds the catalog once; `build`
  merges every override into the base figment, extracts `S::Config`, parses
  each section from `figment.find_value(path)`, runs `derive`;
  `load_snapshot` applies stored overrides one by one and ignores those whose
  key the catalog does not resolve ("cannot be changed at runtime"); `unset`
  scans the storage, so ignored keys can be cleaned up.
- `rust/botconf/src/snapshot.rs` — `Snapshot { config, derived, schema,
  sections: BTreeMap<&'static str, (String, ParsedSection)>, overrides,
  ignored, figment }`; `value(key)` reads sections from their parsed JSON,
  else the config; `source(key)` from the figment's metadata.
- `rust/botconf/src/command.rs` — `Listing { settings, entries, ignored }`.
- Assistant bot declarations: `src/settings/mod.rs` (`CORE_SETTINGS`,
  `LOG_FILTERS`, `AI_MODELS` as `const`), `src/modules/lights/settings.rs`
  (`RUNTIME_SETTINGS`, `PRESET_FORM` with `Field::new(..).group(LOOK)`),
  `src/modules/trips/settings.rs` (`RUNTIME_SETTINGS`, `YES_OR_NO`),
  `src/modules/general.rs`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Core tests | `cargo test -p botconf --all-features` | exit 0 |
| All | `cargo test --workspace --all-features` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features -- --deny warnings` | exit 0 |
| Format | `cargo fmt --check` | exit 0 |
| Minimum Rust | `cargo +1.95.0 test -p botconf --all-features --locked` | exit 0 |

## Scope

**In scope**: `rust/botconf/src/**`, `rust/botconf/tests/**`,
`rust/botconf-telegram/src/**` (adapting to `Clone` kinds and the
`Arc<Catalog>`), README, plans index; the assistant bot's declarations
(Step 6, its own branch).

**Out of scope**: the panel engine (004 is written against this plan's
result); the wire format (005); any JSON Schema implementation; wuphf.

## Git workflow

On `main`:
1. `feat(botconf)!: owned descriptors (Shared) for kinds and settings`
2. `feat(botconf)!: an append-only catalog behind the store`
3. `feat(botconf): sections registered at runtime, with their own validator and base`
4. `docs: describe dynamic sections`
Tag `v0.3.0` locally. (004 then tags `v0.4.0`, 005 `v0.5.0`.)

## Steps

### Step 0: Baseline

`cargo test --workspace --all-features 2>&1 | grep "^test result"` — record.

### Step 1: `Shared<T>` and owned descriptors

Add `rust/botconf/src/shared.rs`:

```rust
//! Descriptor data that is either compiled in (`Static`) or built at runtime
//! (`Owned`), e.g. the settings a connector declares when it connects.

use std::{fmt, ops::Deref, sync::Arc};

pub enum Shared<T: ?Sized + 'static> {
    Static(&'static T),
    Owned(Arc<T>),
}

impl<T: ?Sized> Clone for Shared<T> { /* Static copies, Owned clones the Arc */ }
impl<T: ?Sized> Deref for Shared<T> { type Target = T; /* … */ }
impl<T: ?Sized + PartialEq> PartialEq for Shared<T> { /* by value */ }
impl<T: ?Sized + Eq> Eq for Shared<T> {}
impl<T: ?Sized + fmt::Debug> fmt::Debug for Shared<T> { /* the value */ }
impl From<String> for Shared<str> { … }
impl<T> From<Vec<T>> for Shared<[T]> { … }
impl<T> Shared<T> { pub fn owned(value: T) -> Self { Self::Owned(Arc::new(value)) } }
```

Then change the descriptor types:

- `Kind`: derive `Clone, Debug, PartialEq, Eq` (no `Copy`). `Number { unit:
  Shared<str>, suggestions: Shared<[i64]>, … }`, `Ref { domain: Shared<str>,
  many }`, `Form(Shared<Form>)`, `Map { names: Option<Choices>, value:
  Shared<Kind> }`.
- `Form { fields: Shared<[Field]>, initial: Shared<str> }`; `Field { key,
  title: Shared<str>, kind: Kind, group: Option<Shared<str>> }`.
- `Choices { Fixed(Shared<[FixedChoice]>), KeysOf(Shared<str>),
  Dynamic(DynamicChoices) }` — `DynamicChoices` stays a function pointer
  (it is `'static` and only compiled-in settings use it).
- `FixedChoice { value, label: Shared<str> }`.
- `RuntimeSetting { key, description: Shared<str>, title: Option<Shared<str>>,
  per_entry, kind }` — `Clone`, not `Copy`.

Keep declarations short with `const fn` constructors, so the assistant bot's
`const` arrays stay `const`:

```rust
impl Kind {
    pub const fn text(optional: bool) -> Self;
    pub const fn number(min: i64, max: i64, unit: &'static str, suggestions: &'static [i64], optional: bool) -> Self;
    pub const fn one_of(choices: Choices, custom: bool, optional: bool) -> Self;
    pub const fn set_of(choices: Choices, inverted: bool) -> Self;
    pub const fn reference(domain: &'static str, many: bool) -> Self;
    pub const fn form(form: &'static Form) -> Self;
    pub const fn map(names: Option<Choices>, value: &'static Kind) -> Self;
}
impl Choices { pub const fn fixed(choices: &'static [FixedChoice]) -> Self; pub const fn keys_of(key: &'static str) -> Self; }
impl Form { pub const fn new(fields: &'static [Field], initial: &'static str) -> Self; }
```

Builders that replace a field (`RuntimeSetting::titled`, `kind`,
`entry_names`, `Field::group`) destructure `self`, `core::mem::forget` the
replaced field and rebuild, e.g.:

```rust
pub const fn titled(self, title: &'static str) -> Self {
    let Self { key, title: old, description, per_entry, kind } = self;
    core::mem::forget(old);
    Self { key, title: Some(Shared::Static(title)), description, per_entry, kind }
}
```

Owned constructors for runtime descriptors: `RuntimeSetting::owned(key:
String, description: String, kind: Kind)`, `Kind::Ref { domain:
Shared::from(domain), many }`, etc.

Fix every use in `botconf` and `botconf-telegram` (`setting.kind` is now
borrowed or cloned; `*value` for `&'static Kind` becomes `(*value).clone()`
or a borrow). Update `tests/store.rs`, the `botconf-telegram` toy schema and
`examples/panel_bot.rs` to the constructors.

Tests (in `shared.rs` and `kind.rs`):
- `shared_static_and_owned_compare_by_value` — `Shared::Static("a") ==
  Shared::from("a".to_string())`.
- `const_declarations_still_work` — a `const` `RuntimeSetting` using
  `titled`, `kind(Kind::one_of(Choices::fixed(&X), true, false))` and a
  `const` `Form` with a grouped `Field` compile and equal an owned
  equivalent built at runtime.

**Verify**: `cargo test --workspace --all-features` → baseline + 2;
`cargo +1.95.0 test -p botconf --all-features --locked` → exit 0. Commit 1.

### Step 2: An append-only catalog behind the store

- `CatalogEntry { key: String, title: String, description: Shared<str>,
  per_entry, kind: Kind, section: Option<Shared<str>> }`.
- `Catalog { entries: Vec<Option<CatalogEntry>> }` — `None` is a tombstone.
  `entries()` returns `&[Option<CatalogEntry>]` (positions stable);
  `live()` iterates `(position, &CatalogEntry)`; `resolve(key)` skips
  tombstones; `entry(position) -> Option<&CatalogEntry>`.
  Private `append(entries) -> Range<usize>` and `tombstone(range)`.
- `SettingsStore.catalog: ArcSwap<Catalog>`; `catalog(&self) ->
  Arc<Catalog>`. Changes swap it under the write lock, before publishing the
  snapshot that uses it.
- Adapt `command.rs` (`listing`) and `botconf-telegram` (its `Target`
  resolves through `entry(position)`; a tombstone renders as "This setting
  no longer exists").

Test: `positions_never_move` — append A (3 entries), B (2); tombstone A;
append C: B keeps positions 3–4, C gets 5–…, positions 0–2 resolve to
nothing.

**Verify**: all tests pass. Commit 2.

### Step 3: Dynamic sections

New public API (in `rust/botconf/src/dynamic.rs`, re-exported):

```rust
/// A section described at runtime, e.g. by a connector when it connects.
pub struct DynamicSection {
    pub id: String,          // unique among sections, e.g. "connector:telegram-1"
    pub path: String,        // e.g. "connectors.telegram-1"; must be below a dynamic namespace
    pub title: String,
    pub description: String,
    /// Keys relative to `path`.
    pub settings: Vec<RuntimeSetting>,
    pub validator: Arc<dyn Validator>,
    /// The section's base values, below the stored overrides.
    pub base: Value,
    /// Where the base comes from, shown as its source, e.g. "connector telegram-1".
    pub base_source: String,
}

pub trait Validator: Send + Sync + 'static {
    /// The section's effective value, or why it is invalid.
    fn validate(&self, section: &Value) -> Result<Value, String>;
}

impl<S: Schema> SettingsStore<S> {
    /// Adds or replaces a section. Its stored (and parked) overrides are
    /// applied one at a time; invalid ones are ignored with the reason.
    pub async fn register_section(&self, section: DynamicSection) -> Result<Change<S>, SettingsError>;
    /// Removes a section; its stored overrides stay (parked) for next time.
    pub async fn remove_section(&self, id: &str) -> Result<Option<Change<S>>, SettingsError>;
}

// Schema gains:
fn dynamic_namespaces(&self) -> Vec<&'static str> { Vec::new() }
```

Behaviour:

- `register_section` (under the write lock): reject (`InvalidValue`) a path
  outside every dynamic namespace, a path overlapping a static section or
  another dynamic section, or a runtime key the validated base does not have
  (the dynamic twin of `SectionSettings::check`). Catalog: unchanged
  descriptors keep positions; otherwise tombstone and append.
- Build: `build()` partitions overrides. Keys at or below a dynamic
  section's path go to that section; keys below a dynamic namespace with no
  section are parked; the rest go to the main figment as today. A dynamic
  section's value: `Figment::from(Serialized::defaults(&base))` named
  `base_source`, merged with its overrides (same `Override` provider and
  object-clearing rule, relative keys), extracted as `Value`, then
  `validator.validate`. An `Err` fails the build like an invalid static
  section (`BuildError::Section { path, reason }`).
- `Snapshot` gains `dynamic: BTreeMap<String, DynamicParsed>` (id → path,
  validated value, its figment) and `parked: BTreeMap<String, Value>`.
  `Snapshot::value(key)` and `source(key)` consult dynamic sections first
  (`Source::Stored` for overrides, `Source::Other(base_source)` for the
  base). Add `Snapshot::dynamic(id) -> Option<&Value>`.
- `load_snapshot`: a stored key that the catalog does not resolve but that
  lies below a dynamic namespace is parked (not ignored).
- `set`/`unset`/… on a key of a registered dynamic section work like any
  catalog key. `unset` also clears parked keys (it already scans storage).
- `command::Listing` gains `parked: Vec<(String, Value)>`; `botconf::cli`
  and `botconf-telegram`'s text render list them under "Waiting for their
  section".
- `on_change` is called for registrations and removals like for any change.

Tests (`rust/botconf/tests/dynamic.rs`, with a toy validator that accepts
objects whose `limit` is a number from 1 to 10 and fills `limit: 5`):

1. `registered_sections_layer_overrides_on_their_base`
2. `overrides_wait_for_their_section` — store `connectors.a.limit = 7`
   before load; it is parked; registering `a` applies it (value 7, source
   `Stored`).
3. `invalid_parked_overrides_are_ignored_on_registration` — parked
   `limit = 99` moves to `ignored` with the validator's reason; the base wins.
4. `invalid_changes_are_rejected_by_the_validator` — `set(limit, 0)` →
   `InvalidValue`, nothing stored.
5. `dynamic_keys_never_reach_the_host_config` — the toy `S::Config` has
   `deny_unknown_fields` and still extracts with a section registered.
6. `re_registering_unchanged_keeps_positions` and
   `re_registering_changed_moves_positions` — catalog positions.
7. `paths_must_be_inside_a_dynamic_namespace` — and must not overlap.
8. `removed_sections_park_their_overrides`.
9. `unset_clears_parked_overrides`.

**Verify**: `cargo test -p botconf --all-features` → exit 0. Commit 3.

### Step 4: Wire consistency (no code; check)

Plan 005 serializes descriptors for `Describe`. Confirm each descriptor field
is either owned data (`Shared` reads the same for both variants) or
`Choices::Dynamic`, which is never sent as is: `Describe` resolves choices
against the snapshot (`Kind::choices(view)`), and that resolution is what
goes on the wire. Note the result in this plan's row.

### Step 5: Docs and tag

README: "Dynamic sections" (when to use them, the validator, base, parked
overrides, namespaces) and the new declaration constructors. Commit 4; tag
`v0.3.0`.

### Step 6: Move the assistant bot (after the owner pushes `v0.3.0`)

On the assistant bot's branch for botconf upgrades: switch the tag, then
rewrite the declarations with the `const fn` constructors (`Kind::reference`,
`Kind::one_of`, `Choices::fixed`, `Kind::map`, `Form::new`, …). No behaviour
change; its suite must pass unchanged.

## Test plan

See the tests in Steps 1–3; plus every existing `botconf` and
`botconf-telegram` test, unchanged in substance.

## Done criteria

- [ ] No `&'static` left in the public descriptor types except inside
      `Shared::Static` and `DynamicChoices`
      (`grep -n "&'static" rust/botconf/src/{kind,keys}.rs` shows only
      constructor parameters and `Shared`)
- [ ] The assistant bot's declarations stay `const`
- [ ] The nine dynamic tests and the catalog test pass; all suites, clippy,
      fmt and the 1.95 job pass
- [ ] `v0.3.0` tagged locally; README updated; `plans/README.md` row updated

## STOP conditions

- The `mem::forget` builder pattern does not compile on 1.95 (report the
  error). Do not fall back to non-`const` declarations without the owner.
- Plan 004 landed before this plan.
- Keeping dynamic sections out of the main figment turns out impossible for
  some key shape (report the case).
- An existing test must change its expectations (beyond constructor syntax).

## Maintenance notes

- 004's engine reads `store.catalog()` (an `Arc<Catalog>`) per render and
  skips tombstones; its tokens hold positions, which this plan keeps stable.
- 005's `Watch` must include dynamic sections and report registrations and
  removals; wuphf's connectors watch their own section's path.
- A `jsonschema` feature with a ready `Validator` could be added if a second
  host needs one.
- A `restart: bool` flag on `RuntimeSetting` (settings that apply after a
  restart) fits here later without breaking anything.
