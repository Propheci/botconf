# Plan 011: Drafts in the settings panel

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Build limits (the owner's laptop)**: prefix every cargo command with
> `nice -n 19` and set `CARGO_BUILD_JOBS=2`; prefix Go commands with
> `nice -n 19` and set `GOMAXPROCS=2` and `GOFLAGS=-p=2`. Check and test only
> the crates you touch (`-p`), and run the workspace gate once, at the end. Do
> not run `nix flake check`; CI runs it on push. Only one agent builds at a
> time.
>
> **Drift check (run first)**: `git diff --stat v0.8.0..HEAD -- rust/ proto/`
> Expected: empty (010 tagged `v0.8.0`). What must still hold: `Field` in
> `rust/botconf/src/kind.rs` has `key`, `title`, `kind`, `group` and `const fn
> new`/`group`; `Page` in `rust/botconf-panel/src/token.rs` is `Home |
> Section(String) | Setting(Target)`; `Question { target, ask }` in
> `rust/botconf-panel/src/lib.rs`; new map entries are created in
> `input.rs` (`AskKind::Entry`) and `press.rs` (`fn pick`) with
> `Edit::Set(key, empty)`; the proto's `FormField` has fields 1–4. If 010 did
> not land, STOP (this plan's tag and wuphf 044 assume it).

## Status

- **Priority**: P1 (wuphf rules cannot be created from the panel without it)
- **Effort**: M–L
- **Risk**: MED — a new kind of page and a new kind of action in the engine;
  the engine's 37 tests and the renderers' tests are the safety net
- **Depends on**: 004, 005, 009, 010
- **Category**: feature (UX)
- **Planned at**: `4cf325a` (tag `v0.7.0`) plus plan 010, 2026-10-29

## Why this matters

The panel saves every edit at once, and the store refuses an invalid value.
That works while each step leaves a valid value, and breaks when a value is
only valid once several fields are set:

- **A new map entry whose empty value is invalid.** Adding an entry stores
  `Kind::empty_value()` (for a form, `Form.initial`) immediately
  (`input.rs:209-219`, `press.rs:316-321`). A wuphf rule starts from
  `{"kind": "many-to-one"}`, which has no `source_connector` or `destination`,
  so the store refuses it and the panel cannot create a rule at all (wuphf
  039's notes).
- **A field change that is only valid together with others.** Switching a
  rule's `kind` from many-to-one to one-to-one leaves `source_connector` set
  and `source` unset: refused, and there is no order of single edits that gets
  there.

The owner decided on 2026-10-29 to add drafts: the engine holds an unsaved
value, walks the user through the fields that still need a value, and stores
it only when the user presses Save and the store accepts it.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | A draft is an unsaved value for one key (a form setting, or an entry of a map of forms). It is shown like the setting's own page, with **💾 Save** and **✖️ Cancel**. |
| 2 | Save stores the value through the normal store path (`set`), so it is validated exactly like any other change; a refused save keeps the draft and shows the reason. |
| 3 | Nothing is stored until Save. Cancel discards the draft. |

### Decisions this plan makes (flag to the owner; reviewers may push back)

- **Drafts travel inside the panel's own tokens; no new storage.** A draft
  page and every button on it carry the whole draft value. Those actions are
  long, so the hybrid token scheme (004) stores them as `~` handles in the
  host's `TokenStore` (30-day TTL, renewed on redraw), exactly like other long
  buttons. So:
  - the gRPC server (005) stays stateless, and a draft works across
    processes and survives a restart for as long as its handles live;
  - a typed answer inside a draft carries the draft in the `Question`, which
    the front end already keeps as opaque text (`Ask.question` in the proto,
    Go's `settings.Questions`, botconf-telegram's prompts). A front end that
    restarts loses only the pending question, not the draft's buttons.

  The alternative, a draft table keyed by user and setting, needs a new
  storage trait in every host (wuphf, the assistant bot) and expiry, for no
  gain over handles.
- **Required fields are declared, with an optional condition.** `Field` gains:
  ```rust
  /// The value is incomplete without this field (the panel's drafts ask for it).
  pub const fn required(self) -> Self
  /// This field applies only while the field `key` holds the string `value`;
  /// otherwise it is hidden, and dropped when that field changes.
  pub const fn when(self, key: &'static str, value: &'static str) -> Self
  ```
  Deriving "what is missing" from the validator's error text is not possible:
  the reason is free text from `Schema::derive` or a JSON Schema validator.
  The declaration only guides the draft; validity is still the store's
  decision (decision 2).
- **A dry run in the store.** `SettingsStore::check(&self, key: &str, value:
  Value) -> Result<(), SettingsError>` builds the configuration as `set` would
  and returns the same error, without storing or publishing anything. The
  draft page uses it to decide whether Save is offered, and shows the reason
  otherwise.
- **When drafts start:**
  1. Adding an entry (a typed name, or a picked name in `fn pick`) whose empty
     value fails `check` opens a draft at that entry instead of saving.
  2. Changing a field of a form (on a stored setting or entry) whose result
     fails `check` opens a draft holding the changed value, with the reason as
     the notice, instead of refusing the press.
  3. A typed whole value (`/config set`, or "name value" when adding an
     entry) is unchanged: it is saved or refused, never drafted.
  Only form values get drafts; other kinds keep today's refusal.
- **`when` also applies outside drafts.** A stored form's page hides fields
  whose condition does not hold. When a field that others depend on changes,
  the engine drops the dependent fields that no longer apply, before
  validating (as `group` already clears a group).
- **Names.** User-facing: "draft". In code, `Page::Draft(Target, Value)` and
  `Action::InDraft { value, then }` (a draft's value plus the action pressed
  on it) and `Action::SaveDraft(Target, Value)`. `screens.rs` already has a
  private `struct Draft` (a screen being built, line 35); rename it to
  `Building` so the two do not get confused.
- **Wire changes are additive.** `FormField` gains `bool required = 5;` and
  `FieldCondition when = 6;` (`message FieldCondition { string key = 1;
  string value = 2; }`). `Page` (Render) is unchanged: draft pages are only
  reached by pressing buttons. The Go bindings are regenerated with the
  pinned generators; the Go renderers (007) need **no change**, because the
  screen model (text plus rows of `{label, token}`) is unchanged.

## Current state

- `rust/botconf/src/kind.rs:165-178`, `Form { fields: Shared<[Field]>,
  initial: Shared<str> }`, `const fn new`. `kind.rs:182-215`, `Field { key,
  title, kind, group: Option<Shared<str>> }`, `const fn new`, `const fn group`
  (which reads the fields out with `ptr::read` and `mem::forget`, see
  `RuntimeSetting::titled`; new builders must follow that pattern).
  `kind.rs:129`, `Kind::empty_value()`.
- `rust/botconf/src/store.rs:348`, `pub async fn set`; `store.rs:477-…`,
  `async fn modify` resolves the key, computes the value, builds the snapshot
  with the overrides, then writes storage and publishes.
- `rust/botconf-panel/src/token.rs:101-127`, `Page { Home, Section(String),
  Setting(Target) }` with `encode`/`parse` (`h`, `m:<id>`, `s:<target>`).
  `token.rs:165-192`, `Action` (Open, Post, Toggle, Pick, ToggleValue,
  PickValue, Add, Remove, Ask, Clear, Reset, Delete, Reload, Close) with
  `encode`/`parse`.
- `rust/botconf-panel/src/lib.rs:198-218`, `Question { target, ask }`, encoded
  as `<ask code>:<target>`.
- `rust/botconf-panel/src/setting.rs:76-105`, `Setting::set` rewrites the form
  from `snapshot.value(&key)`, clearing the field's group, and returns
  `Edit::Set(key, form)`.
- `rust/botconf-panel/src/input.rs:194-221`, `AskKind::Entry`: a name, then
  `kind.empty_value()` for forms, `Edit::Set("<map>.<name>", value)`.
- `rust/botconf-panel/src/press.rs:304-328`, `fn pick`: a picked map-entry
  name stores `entry.empty_value()`.
- `rust/botconf-panel/src/press.rs:31`, `press`; `press.rs:101`, `answer`;
  `rust/botconf-panel/src/screens.rs:43`, `render`; `screens.rs:227`, `fn
  setting` (a setting's page; `Kind::Form` at `screens.rs:418` lists the
  fields); `screens.rs:35`, the private `struct Draft`.
- `rust/botconf-panel/src/edit.rs:28-71`, `apply(store, Edit, by)`.
- `proto/botconf/v1/settings.proto:191-201`, `message FormField { key = 1;
  title = 2; kind = 3; group = 4; }`; `:404-417`, `message Ask { string
  question = 1; … }` (opaque to clients).
- `rust/botconf-panel/src/testing.rs`, the engine's toy schema;
  `rust/botconf-panel/src/tests.rs`, 37 tests (`answer(&panel, target,
  AskKind::Entry, typed("cozy"))` at line 436 is the entry flow).

## Commands you will need

```sh
export CARGO_BUILD_JOBS=2
nice -n 19 cargo check -p botconf -p botconf-panel --all-targets --all-features
nice -n 19 cargo test -p botconf-panel
nice -n 19 cargo test --workspace --all-features   # once, at the end
nice -n 19 cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
# Go bindings (generators pinned in 006: protoc 34.1, protoc-gen-go 1.36.11,
# protoc-gen-go-grpc 1.6.2, from the wuphf dev shell)
scripts/gen-go.sh
(cd go && GOMAXPROCS=2 GOFLAGS=-p=2 nice -n 19 go test -race ./...)
```

## Scope

In: `rust/botconf/src/{kind.rs,store.rs}`, `rust/botconf-panel/src/**`,
`rust/botconf-grpc/src/convert.rs` (the new `FormField` fields both ways),
`proto/botconf/v1/settings.proto`, `go/settingspb/*` (regenerated),
`README.md`, `plans/README.md`, crate versions.

Out: the Go renderers' code, botconf-telegram's rendering (it draws the
engine's screens; its prompt store keeps `Question`, which only grows a
field), wuphf (its plan 044 marks its rule form).

## Git workflow

Commit on `main`, one commit per step. Tag `v0.9.0` and `go/v0.9.0` on the
final commit (the Go bindings change). Push `main` and both tags yourself;
never force-push, never move a tag.

## Steps

### Step 0: Baseline

`nice -n 19 cargo test -p botconf -p botconf-panel -p botconf-telegram -p
botconf-grpc` → record the counts.

### Step 1: `required`, `when` and the store's dry run

- `kind.rs`: `Field` gains `required: bool` and `when: Option<(Shared<str>,
  Shared<str>)>`, set by `const fn required(self)` and `const fn when(self,
  key, value)` in the `group` pattern. `Field::new` sets `false`/`None`.
  A helper `Form::applies(&self, field: &Field, value: &Map) -> bool` (the
  `when` test) and `Form::missing(&self, value: &Map) -> Vec<&Field>`
  (required and applying, but absent or `null`).
- `store.rs`: `pub async fn check(&self, key: &str, value: Value) ->
  Result<(), SettingsError>`, sharing `modify`'s build step: resolve the key,
  layer the value as `set` would, build, return the error. No storage write,
  no publish. Hold the write lock while building, like `modify`.
- `botconf-grpc/src/convert.rs` and the proto: `FormField.required = 5`,
  `FieldCondition when = 6`, both directions; regenerate the Go bindings.

Tests: `form_missing_lists_required_applying_fields`, `when_hides_fields`,
`check_validates_without_storing` (the snapshot and storage are unchanged,
and `subscribe` sees nothing), `form_fields_round_trip_required_and_when`
(grpc convert).

**Verify**: `nice -n 19 cargo test -p botconf -p botconf-grpc` → Step 0
counts + 4, green; `git diff --stat go/settingspb` shows only the two new
fields.

### Step 2: Draft pages and actions

- `token.rs`: `Page::Draft(Target, Value)` (`d:<target>:<json>`), `Action::InDraft
  { value: Value, then: Box<Action> }` and `Action::SaveDraft(Target, Value)`.
  Their encodings are long, so they end up as handles. Keep `parse` total
  (malformed JSON → `None`).
- `screens.rs`: rename the private `Draft` to `Building`. A draft page (`fn
  draft`): the setting's title with "📝 Draft — not saved", the fields like
  the form page (hiding fields whose `when` fails, marking missing required
  ones "⚠️ needed"), then either the store's refusal reason (from `check`) or
  nothing, then a row: **💾 Save** (only when `missing` is empty and `check`
  passes) and **✖️ Cancel** (`Open` of the parent page). Every field button is
  the normal field action wrapped in `InDraft { value, then }`.
- The stored-form page (`screens.rs:418`) also hides fields whose `when`
  fails.

**Verify**: `nice -n 19 cargo check -p botconf-panel --all-targets` → exit 0.

### Step 3: Presses and answers inside a draft

- `setting.rs`: `Setting::set_in(&self, form: Map, value: Option<Value>) ->
  Map` (the field change, with the group clearing and the `when` cleanup),
  and `Setting::set` uses it on the stored form.
- `press.rs`: `InDraft { value, then }` runs `then` against the draft value
  instead of the store: a field edit produces a new draft value via `set_in`
  and shows `Page::Draft(entry, new_value)`; an `Ask` produces a question
  carrying the draft. `SaveDraft(target, value)` applies `Edit::Set(key,
  value)`; a refused save is `PanelError::Rejected(reason)` with the draft
  still showing (the panel message is not changed).
- `lib.rs`: `Question` gains `draft: Option<Value>`; the encoding appends
  `|<json>` when present, and `parse` reads both forms (old encodings have no
  draft).
- `press.rs::answer`: with a draft question, the answer edits the draft (as
  above) instead of the store.
- Starting drafts (the design decisions' "when drafts start"): in
  `input.rs` `AskKind::Entry` and `press.rs` `fn pick`, when `check(entry key,
  empty)` fails for a form, show `Page::Draft(entry target, empty)` with the
  notice "Fill in the fields, then 💾 Save" instead of `Edit::Set`; in a field
  edit on a stored form, when `check` fails, show `Page::Draft(target,
  new_value)` with the reason as the notice instead of refusing.

**Verify**: `nice -n 19 cargo test -p botconf-panel` → Step 0 count, green
(no existing test changed: the toy's existing forms start valid).

### Step 4: Tests

Add to the engine's toy schema (`testing.rs`) a map of forms that needs two
fields together, e.g. `routes.<name>` with `kind` (`a`/`b`, required), `x`
(required `when kind = a`), `y` (required `when kind = b`), validated in the
toy's `derive`. Then, in `tests.rs`:

1. `a_new_entry_opens_a_draft` — add `routes.r1` → a draft page, nothing
   stored; set `x` through the draft's buttons → Save appears; Save → stored,
   the entry's page shows.
2. `switching_a_kind_opens_a_draft` — a stored `kind = a` route; pick
   `kind = b` → a draft (the reason as the notice), `x` dropped, `y` needed;
   set `y`; Save → stored as `b`.
3. `cancel_discards_the_draft` — Cancel → the parent page, storage
   unchanged.
4. `an_invalid_save_is_refused` — a Save token for an invalid value (built by
   hand, or from a draft whose value went stale because the config changed)
   → `Rejected` with the reason, nothing stored.
5. `a_typed_answer_edits_the_draft` — an `Ask` inside a draft, answered with
   text → the draft updated, nothing stored.
6. `drafts_survive_a_new_panel` — a draft button pressed on a second
   `Panel` sharing the token store (as after a restart) → same draft page.
7. `old_questions_still_parse` — a `Question` encoded without a draft still
   parses.

**Verify**: `nice -n 19 cargo test -p botconf-panel` → Step 0 count + 7,
green. Mutation checks, each must fail a test: (a) entries saved directly
again → 1 fails; (b) `when` cleanup removed → 2 fails; (c) Save skips the
store (no `set`) → 1 fails; (d) `InDraft` edits the store → 3 or 5 fails.

### Step 5: Docs and tag

- README: under "Declaring settings", `required()` and `when()` with a short
  example (a form with two kinds); under "Writing a front end", one paragraph
  on drafts (nothing for a front end to do: draft buttons are ordinary
  tokens, and questions stay opaque).
- Crate versions → `0.9.0`; `plans/README.md` row 011 DONE with notes.
- Gate, once: `nice -n 19 cargo test --workspace --all-features`, clippy,
  fmt, and the Go module's `go vet ./... && go test -race ./...` (niced).
- Commit, tag `v0.9.0` and `go/v0.9.0`, push `main` and both tags; check
  `git ls-remote --tags https://github.com/propheci/botconf`.

The assistant bot needs no change (its forms start valid). Moving it to
`v0.9.0` is optional and is not part of this plan.

## Test plan

| Test | Crate | Proves |
|---|---|---|
| `form_missing_lists_required_applying_fields`, `when_hides_fields` | botconf | the declarations |
| `check_validates_without_storing` | botconf | the dry run has no side effects |
| `form_fields_round_trip_required_and_when` | botconf-grpc | the wire carries them |
| `a_new_entry_opens_a_draft` | panel | new entries no longer fail |
| `switching_a_kind_opens_a_draft` | panel | multi-field changes work |
| `cancel_discards_the_draft` | panel | nothing stored until Save |
| `an_invalid_save_is_refused` | panel | validity stays the store's |
| `a_typed_answer_edits_the_draft` | panel | typed fields inside drafts |
| `drafts_survive_a_new_panel` | panel | drafts live in the token store |
| `old_questions_still_parse` | panel | front ends holding old questions keep working |

## Done criteria

- [ ] All existing tests pass unchanged (except renames of the private
      `Draft`).
- [ ] The new tests pass; the four mutations each fail one.
- [ ] The Go bindings regenerate with only the two `FormField` additions; the
      Go renderers' tests pass untouched.
- [ ] `v0.9.0` and `go/v0.9.0` pushed.
- [ ] `plans/README.md` row 011 DONE with notes.

## STOP conditions

- 010 did not land (no `v0.8.0`).
- A draft action does not fit a handle (the `TokenStore` refuses it): report
  the size; do not truncate drafts.
- The `const fn` builders for `required`/`when` do not compile on 1.95 with
  the `group` pattern.
- Any existing renderer test (Telegram, Go) fails: the screen model changed,
  which this plan rules out.

## Maintenance notes

- A draft's handles expire with the token TTL (30 days by default); an old
  draft button then redraws the home screen as stale, like any expired
  handle.
- wuphf's plan 044 marks its rule form's fields with `required()`/`when()`
  and moves to `v0.9.0`.
