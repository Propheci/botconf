# Plan 004: Extract a platform-neutral panel engine (`botconf-panel`)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat v0.3.0..HEAD -- rust/`
> Expected: empty (plan 009 tagged `v0.3.0`; 009 runs before this plan).
> What must still hold (the results of plans 003 and 009):
> `rust/botconf/src/kind.rs` has `Kind::Ref { domain, many }` and no
> `Chat`/`Users`/`Chats`, and `Kind` is `Clone` (not `Copy`) with `Shared<…>`
> data; `SettingsStore::catalog()` returns an `Arc<Catalog>` whose
> `entries()` may hold tombstones (`None`); dynamic sections exist
> (`SettingsStore::register_section`); `rust/botconf-telegram/src/names.rs`
> has the domain-keyed `Names<R>` (`name(bot, domain, id)`,
> `resolve(domain, text)`); actors are `Option<String>`;
> `rust/botconf-telegram/src/` still holds `panel.rs`, `input.rs`,
> `callback.rs`, `edit.rs`, `handlers.rs`, `text.rs`, `prompts.rs`,
> `names.rs`, `testing.rs`. If 003 or 009 landed a different API (other
> names, `Ref` with more fields, `i64` actors, a catalog whose positions can
> move), STOP and re-plan the excerpts below against it.

## Status

- **Priority**: P1 (plans 005 and 007 render this engine's screens remotely)
- **Effort**: L
- **Risk**: MED — a large move of UI logic; the existing panel tests are the
  safety net
- **Depends on**: 003, 009 (009 makes descriptors owned and the catalog
  append-only; this engine is written against that)
- **Category**: architecture
- **Planned at**: assistant_bot_rs `d772d66` sources, before 001–003 and 009
  landed, 2026-10-25. Line numbers below are from that commit; 003 and 009
  shift some of them, and 009 turns `Kind` from `Copy` into `Clone`
  (`setting.kind` is borrowed or cloned, never copied).

## Why this matters

About 1,500 lines of `botconf-telegram` decide what the settings panel shows
and does: the home and section screens, an editor per kind, forms, maps,
lists, what each button changes, how a typed answer is read, and the text
subcommands. Only a small part is Telegram: HTML, inline keyboards, reply
keyboards with Telegram's pickers, and sending or editing messages. wuphf's
connectors (Telegram via gotd, as a bot or a user account, and later
Discord) must show the same panel, written in Go. Porting the screens to Go
would duplicate them and let them drift. Instead the screens are computed in
Rust, once, as platform-neutral `Screen`s; every front end only draws them
and reports what was pressed or typed. `botconf-telegram` becomes the first
such front end, and plan 005 serves the same engine over gRPC.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | One screen engine in Rust, a new crate `botconf-panel`, with no platform dependency (no teloxide). |
| 2 | A `Screen` is text plus a grid of buttons whose actions are short opaque tokens, plus a prompt when the user must type a value. Front ends never interpret tokens. |
| 3 | `botconf-telegram` is rebuilt as a thin teloxide renderer over the engine. Its existing tests keep passing. |
| 4 | Telegram's 64-byte callback data limit drives the token design. |
| 5 | Front ends register pickers and name lookups per reference domain (from plan 003). |

Decided while planning (flag to the owner; reviewers may push back):

- **Tokens are deterministic, not server-side handles.** The owner settled
  "short tokens the front end never interprets". This plan keeps today's
  stateless encoding of an action (`callback.rs`: setting index, entry name,
  field index, argument) as the token. It needs no table, works across
  restarts and across processes (the gRPC server in plan 005 and a reloaded
  bot see the same tokens), and already fits 64 bytes. A server-side handle
  table would add expiry and memory for no gain. The engine guarantees each
  token fits the front end's limit and leaves out buttons that do not fit,
  as today. One change: **removing a list item uses its position plus a
  4-hex-digit check of its value** instead of the value itself, because wuphf
  reference ids (`<connector>:<address>`) can be long. A stale check refuses
  the press ("This list changed") and redraws.
- **Text is a small span model, not HTML.** `Text` is a list of spans
  (plain, bold, italic, code, code block). Each renderer turns it into its
  own markup (Telegram HTML or gotd entities, Discord markdown). Lines are
  `\n` inside plain spans.
- **Limits come from the renderer.** `Limits { token_bytes, text_chars,
  max_rows, max_per_row }`. Telegram: 64 minus its prefix, 4096, none, 8.
  Discord (plan 007): 100, 2000, 5, 5. When `max_rows` is set and a screen has
  more rows, the engine pages them: the screen's content rows are cut into
  pages, and a last row holds ◀ / ▶ and the screen's own navigation (Back,
  Close). The page number travels in the `Open` token.
- **Prompts stay with the front end.** The engine is stateless: asking for a
  value returns an `Ask` holding a `Question` (which has a compact string
  encoding), and the front end keeps it until the answer arrives, with its
  own prompt mechanism. `botconf-telegram`'s `Prompts` (used by the
  assistant bot's other modules too) stays in `botconf-telegram`. Over gRPC
  (plan 005) the server stays stateless too: the encoded `Question` goes to
  the client, which sends it back with the answer (the Go SDK keeps it).

## Current state

`rust/botconf-telegram/src` (assistant_bot_rs `d772d66` line numbers):

- `lib.rs` (175 lines) — `SettingsPanel<S, R>` (fields `store`, `prompts`,
  `names: Option<Arc<dyn Names<R>>>`, `on_change`, `section_note`, `command`,
  `prefix`, `owner`), builder methods (`names`, `on_change`, `section_note`,
  `command`, `callback_prefix`, `prompt_owner`), `open(bot, chat, page)`,
  `button(label, page)`, `post_button(prefix, label, page)`, `truncate`,
  `actor`. Constants `DEFAULT_PREFIX = "cfg:"`. Traits `PanelBot`
  (`Requester<Err = RequestError> + Clone + Send + Sync`).
- `callback.rs` (312) — `Target { setting: usize, entry: Option<String>, field:
  Option<usize> }` with `encode`/`parse` (`3.lights#2`), `Page { Home,
  Section(String), Setting(Target) }`, `Ask { Value, Items, Entry }`, `Button
  { Open, Post, Toggle(Target, usize), Pick(Target, usize), Remove(Target,
  String), Ask(Target, Ask), Clear, Reset, Delete, Reload, Close }` with
  `encode(prefix) -> Option<String>` (refuses data over 64 bytes) and
  `parse(prefix, data)`. 4 tests (`buttons_round_trip`, `parents`,
  `data_that_does_not_fit_is_refused`, `rejects_malformed_data`).
- `panel.rs` (729) — `Screen { text: String (HTML), keyboard:
  InlineKeyboardMarkup }`; `Setting<'a>` (resolves a `Target` against the
  `Catalog`: `key()`, `title()`, `value()`, `set()`, `change_list()`,
  `form_kind()`); `render(panel, bot, page, notice) -> Screen` (168);
  `home` (189), `section` (231), `setting_rows` (273), `setting` (299) with an
  editor per kind (`Json` 343, `Text` 354, `Ref` single (was `Chat`, 359),
  `Number | OneOf` 370, `SetOf` 423, `Ref` list (was `Users | Chats`, 440),
  `Form` 474, `Map` 494); `summary` (594); `button(prefix, label, Button) ->
  Option<InlineKeyboardButton>` (723).
- `input.rs` (494) — `Question { target, ask, panel: MessageId }`;
  `question(setting, ask, current, pickers) -> (String, ReplyMarkup, bool)`
  (45); Telegram pickers (123-152); `read(panel, snapshot, setting, question,
  message) -> Result<(Edit, Page), ReadError>` (166) with `value`, `typed`,
  `items`, the user and chat id readers, entry names checked by
  `is_entry_name` and `MAX_ENTRY_NAME_LEN = 32`. 3 tests.
- `edit.rs` (72) — `Edit { Set, Extend, Remove, Reset, DeleteEntry, Reload }`
  and `apply(store, edit, by) -> Applied { notice, change }`.
- `handlers.rs` (660) — `SettingsPanel::handler()` (answers to prompts, button
  presses filtered by prefix), `run_command(bot, msg, args)`, `handle_button`
  (121), `edit_for(panel, button) -> (Edit, Page)` (228), `handle_answer`
  (311), `post`/`send`/`show` (373-417). 8 tests driving the toy panel
  (`the_home_screen_lists_the_settings_and_sections`,
  `toggles_turn_features_on_and_off`, `choices_are_picked`,
  `map_entries_are_added_from_their_names_and_deleted`,
  `known_users_and_chats_are_shown_by_name`,
  `forms_are_edited_field_by_field`, `buttons_can_post_pages_from_elsewhere`,
  `clearing_an_optional_text`), plus the two added by plan 003.
- `text.rs` (248) — `Action { Panel, Help, Run(SettingsCommand) }`,
  `usage(command)`, `parse_action(args)`, `render(outcome, command) -> String`
  (HTML). 3 tests.
- `prompts.rs` (305), `names.rs` (41), `testing.rs` (226, the `Toy` schema
  with every kind, `FakeNames`, `panel()`), `examples/panel_bot.rs`.
- The assistant bot uses: `botconf_telegram::{SettingsPanel, Page,
  post_button, DEFAULT_PREFIX, Names, Name, prompts}` (`src/modules/settings.rs`,
  `src/directory.rs`, `src/prompts.rs`, the lights and trips modules for
  `prompts`).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Engine tests | `cargo test -p botconf-panel` | exit 0 |
| Telegram tests | `cargo test -p botconf-telegram` | exit 0, every pre-existing test name still present (here or in the engine) |
| All | `cargo test --workspace --all-features` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features -- --deny warnings` | exit 0 |
| Format | `cargo fmt --check` | exit 0 |
| No platform deps | `cargo tree -p botconf-panel -e normal \| grep -c teloxide` | `0` |
| Minimum Rust | `cargo +1.95.0 test -p botconf-panel --locked` | exit 0 |

Record the full list of test names first:
`cargo test -p botconf-telegram -- --list 2>/dev/null | grep ': test$' | sort > /tmp/botconf-tests-before.txt`.

## Scope

**In scope**: `rust/botconf-panel/**` (create), `rust/botconf-telegram/**`,
root `Cargo.toml` (member, workspace dependency), `.github/workflows/ci.yml`
(add `botconf-panel` to the `msrv` matrix at 1.95), README, plans index;
the assistant bot only if its use of the public API breaks (it should not).

**Out of scope**: the wire format and gRPC (plan 005); Go (006, 007); new
editors or kinds; changing what any screen says (text and buttons must stay
the same, apart from markup being produced through spans).

## Git workflow

On `main`:
1. `feat(botconf-panel): add the platform-neutral screen engine`
2. `refactor(botconf-telegram): render the engine's screens`
3. `docs: describe the panel engine and how to write a front end`
Tag `v0.4.0` locally.

## The engine's API

Write the crate to this shape. Names may be adjusted for clarity; the split
of responsibilities may not.

```rust
//! rust/botconf-panel/src/lib.rs

/// A message of the panel: what to show and the buttons under it.
pub struct Screen {
    pub text: Text,
    pub rows: Vec<Vec<ButtonView>>,
}

pub struct ButtonView {
    pub label: String,
    /// Opaque. The front end sends it back to `Panel::press` as is.
    pub token: String,
}

/// Formatted text. Renderers map it to their markup.
pub struct Text(pub Vec<Span>);
pub struct Span { pub style: Style, pub text: String }
pub enum Style { Plain, Bold, Italic, Code, CodeBlock }

/// What the front end can display.
pub struct Limits {
    pub token_bytes: usize,
    pub text_chars: usize,
    pub max_rows: Option<usize>,
    pub max_per_row: usize,
}

/// Who is looking, and how their screen is limited.
pub struct Viewer<'a> {
    /// Recorded as the author of changes.
    pub actor: Option<String>,
    pub limits: &'a Limits,
    /// Names and id resolution for reference domains, if the front end has any.
    pub names: Option<&'a dyn Names>,
}

/// Names for references; the front end's `Names` adapts to this.
pub trait Names: Send + Sync {
    fn name<'a>(&'a self, domain: &'a str, id: &'a Value) -> BoxFuture<'a, Option<Name>>;
    fn resolve<'a>(&'a self, domain: &'a str, text: &'a str) -> BoxFuture<'a, Option<Value>>;
}
pub struct Name { pub full: String, pub short: String }

/// What a front end knows about a reference domain.
pub struct Domain {
    pub name: &'static str,
    pub noun: (&'static str, &'static str),   // ("user", "users")
    /// Reads a typed id, e.g. digits for Telegram; `None` = not an id.
    pub parse: fn(&str) -> Option<Value>,
}

pub enum Page { Home, Section(String), Setting(Target) }
pub struct Target { /* as in callback.rs today */ }

/// The engine over a settings store.
pub struct Panel<S: Schema> { /* store, domains, command, section_note, on_change */ }

impl<S: Schema> Panel<S> {
    pub fn new(store: Arc<SettingsStore<S>>) -> Self;
    pub fn domain(self, domain: Domain) -> Self;
    pub fn section_note(self, note: impl Fn(&Snapshot<S>, &str) -> Option<String> + …) -> Self;
    pub fn command(self, command: impl Into<String>) -> Self;   // for the texts
    pub fn store(&self) -> &Arc<SettingsStore<S>>;

    pub async fn render(&self, page: &Page, viewer: &Viewer<'_>) -> Screen;
    /// A button was pressed.
    pub async fn press(&self, token: &str, viewer: &Viewer<'_>) -> Result<Reaction<S>, PanelError>;
    /// The user answered a question.
    pub async fn answer(&self, question: &Question, input: Input, viewer: &Viewer<'_>)
        -> Result<Reaction<S>, PanelError>;
    /// The text form: `/config`, `/config set …` (text.rs today).
    pub async fn command_text(&self, args: &str, viewer: &Viewer<'_>) -> Result<CommandReply<S>, PanelError>;
    /// A token that posts `page` as a new message, for other panels' buttons.
    pub fn post_token(&self, page: &Page, limits: &Limits) -> Option<String>;
}

/// What a press or an answer leads to. Several fields may be set.
pub struct Reaction<S: Schema> {
    /// Redraw the panel message with this.
    pub screen: Option<Screen>,
    /// Post this as a new message (the `Post` buttons).
    pub post: Option<Screen>,
    /// A short notice: a toast on Telegram, an ephemeral reply on Discord.
    pub notice: Option<String>,
    /// Ask the user to type (or pick) something.
    pub ask: Option<Ask>,
    /// The panel was closed: redraw it with `screen` and no buttons.
    pub closed: bool,
    /// The settings change this made, for the host's hooks.
    pub change: Option<Change<S>>,
}

pub struct Ask {
    pub question: Question,
    pub text: Text,
    /// Offer the platform's picker for this domain, if the front end has one.
    pub picker: Option<Picker>,
    /// The current value, for "tap to copy".
    pub current: Option<String>,
}
pub struct Picker { pub domain: &'static str, pub many: bool }

/// What the panel asked; kept by the front end until answered.
pub struct Question { pub target: Target, pub ask: AskKind }
impl Question { pub fn encode(&self) -> String; pub fn parse(text: &str) -> Option<Self>; }
pub enum AskKind { Value, Items, Entry }

/// An answer.
pub enum Input {
    Text(String),
    /// Ids picked with a platform picker, already in the domain's JSON form.
    Picked { domain: String, ids: Vec<Value> },
    Cancel,
}

pub enum PanelError {
    /// The press or answer made no sense (stale button, bad value); show it
    /// to the user and keep the panel.
    Rejected(String),
    /// The storage failed: an infrastructure error for the host.
    Storage(SettingsError),
}
```

## Steps

### Step 0: Baseline

Record test names (see "Commands") and `cargo test --workspace
--all-features` totals.

### Step 1: Create the crate and move the model

Create `rust/botconf-panel` (`Cargo.toml` like `botconf-telegram`'s, with
`botconf`, `futures`, `serde_json`, `thiserror`, `tracing`; dev `figment`,
`serde`, `tokio` `test-util`). Add it to the workspace members (`rust/*`
already matches) and to `[workspace.dependencies]` as
`botconf-panel = { path = "rust/botconf-panel" }`.

Move, adapting only imports:

- `callback.rs` → `botconf-panel/src/token.rs`: `Target`, `Page`, the button
  actions (rename `Button` to `Action`), `encode(limit) -> Option<String>`
  without the prefix (the front end adds its own). `Page` gains an optional
  page number for paged screens (`o:s:3#1@2` style; choose a separator that
  cannot occur in entry names — entry names already exclude `:` and `#`, so
  use `@`). `Remove(Target, String)` becomes `Remove(Target, usize, u16)`
  (position and check). Move its 4 tests, adjusted for the new `Remove` and
  page encoding, keeping their names.
- `edit.rs` → `botconf-panel/src/edit.rs`, unchanged apart from `by:
  Option<String>`.
- `panel.rs`'s `Setting<'a>` → `botconf-panel/src/setting.rs`.

**Verify**: `cargo test -p botconf-panel` → the moved tests pass.

### Step 2: Screens

Move `render`, `home`, `section`, `setting_rows`, `setting`, `summary` and
helpers from `panel.rs` into `botconf-panel/src/screens.rs`, producing
`Screen` with `Text` spans instead of HTML strings:

- `bold(&escape(x))` → `Span::bold(x)`; `code_inline(x)` → `Span::code(x)`;
  `code_block(x)` → `Span::code_block(x)`; `italic(…)` → `Span::italic(…)`;
  `escape(x)` → `Span::plain(x)`. No escaping in the engine.
- Buttons: `ButtonView { label, token }` from `Action::encode(limits.token_bytes)`;
  leave out buttons whose token does not fit (as `button()` does today).
- Reference editors use the registered `Domain` for nouns and `viewer.names`
  for names (plan 003 behaviour).
- Read the catalog once per call (`store.catalog()`, an `Arc<Catalog>` since
  009) and resolve `Target`s with `catalog.entry(position)`; a tombstoned
  position renders "This setting no longer exists" with a Back button.
  Dynamic sections (009) are listed on the home screen like the schema's
  sections, after them, by title; `Page::Section(id)` works for both.
- Paging: if `limits.max_rows` is set and the screen has more rows, apply the
  rule from the design decisions. Add a helper `fn page(rows, nav, limits,
  page) -> Vec<Vec<ButtonView>>` and test it directly.
- `max_per_row`: rows built with more buttons are split (today the code uses
  2–4 per row; keep those groupings when they fit).

Add `testing.rs` to the engine (the `Toy` schema and a `FakeNames` without
the bot), moved from `botconf-telegram`; `botconf-telegram` keeps a thin
adapter for its own tests.

Engine tests (new, in `botconf-panel/src/tests.rs`):

- `home_lists_settings_and_sections` — the `Toy` home screen's text spans and
  button labels equal what `botconf-telegram` showed (copy the expectations
  from `the_home_screen_lists_the_settings_and_sections`).
- `tokens_fit_the_limit` — with `token_bytes = 20`, no token is longer than 20
  and buttons whose action would not fit are absent.
- `screens_are_paged_when_rows_are_limited` — `max_rows = 5` on a section
  with 9 settings: page 1 has 4 content rows plus a nav row with ▶; pressing
  ▶ shows the rest with ◀.
- `removing_a_changed_item_is_refused` — remove token for position 0 after
  the list changed under it → `PanelError::Rejected`, and the list is intact.

### Step 3: Presses, answers and the text command

- `press`: `handle_button` + `edit_for` logic (`handlers.rs:121-310`) minus
  Telegram: decode the token; `Open` → `screen`; `Post` → `post`; `Ask` →
  `ask` (built by `input::question` logic, minus the reply keyboards: the
  picker is reported as `Picker { domain, many }`); edits → apply via
  `edit::apply`, `screen` = the page to show, `notice` = `Applied.notice`,
  `change`; `Close` → `closed` with the "Settings closed; send /{command} to
  open them again." screen; a token that does not parse → `Rejected("This
  button no longer works; send /{command}")`.
- `answer`: `input::read` logic over `Input` (`Text` parsed as today; `Picked`
  for picker results; `Cancel` → `screen` of the question's page, notice
  "Cancelled"). Read errors → `Rejected(reason)`.
- `command_text`: `text.rs` (`parse_action`, `usage`, `render` into `Text`)
  plus `command::execute`. `Panel` opens the home screen (`CommandReply::Panel(Screen)`);
  others reply with `Text` and an optional `Change`.

Move `input.rs`'s 3 tests and `text.rs`'s 3 tests (adjusted to spans and
`Input`), keeping their names. Move the 10 `handlers.rs` scenario tests to
the engine as engine-level tests driving `press`/`answer` with tokens taken
from the rendered `Screen` (same names). Keep in `botconf-telegram` only what
needs Telegram (see Step 4).

**Verify**: `cargo test -p botconf-panel` → exit 0;
`cargo tree -p botconf-panel -e normal | grep -c teloxide` → `0`.

Commit 1.

### Step 4: Rebuild botconf-telegram on the engine

`botconf-telegram` keeps its public API (`SettingsPanel<S, R>` and builder
methods, `handler()`, `run_command`, `open`, `button`, `post_button`,
`DEFAULT_PREFIX`, `Page`, `Target` (re-exported from the engine), `Names<R>`,
`Name`, `prompts`), now implemented as:

- `SettingsPanel` wraps a `botconf_panel::Panel<S>` with the two Telegram
  domains registered (`telegram.user`, `telegram.chat`: digits parse to
  numbers; nouns user/users, chat/chats) plus its prefix, prompts, names and
  hooks.
- `render.rs` (new): `Text` → Telegram HTML (`teloxide::utils::html`
  escaping each span), truncated to 4096 characters; rows → inline keyboard
  with callback data `prefix + token`; `Limits { token_bytes: 64 -
  prefix.len(), text_chars: 4096, max_rows: None, max_per_row: 8 }`.
- `Ask` → the question message with the reply keyboard: Telegram's pickers for
  `Picker { domain: USER | CHAT }` when `prompts::pickers_work_in(chat)`,
  else a force-reply; the `Question` is stored with `Prompts::ask` as today
  (its data is now `(Question, MessageId)`).
- `handle_button`/`handle_answer`/`run_command` become: build a `Viewer`
  (actor from the user id, names adapted from `Names<R>` with the bot
  captured), call the engine, then send/edit/answer the callback per the
  `Reaction`, and call `on_change` with `Reaction.change`.
- Picker messages (`shared_users`, `shared_chat`) become
  `Input::Picked { domain, ids }` after `names.remember_*`.

Keep in `botconf-telegram`'s tests whatever exercises Telegram specifics:
HTML escaping of spans, the 64-byte callback data with the prefix, the
pickers offered for the two domains, and `buttons_can_post_pages_from_elsewhere`.

**Verify**:
- `cargo test -p botconf-telegram` and `cargo test -p botconf-panel` → exit 0;
- `cargo test -p botconf-telegram -p botconf-panel -- --list 2>/dev/null | grep ': test$' | sed 's/^.*:://' | sort -u`
  contains every name from `/tmp/botconf-tests-before.txt` (module paths
  may differ);
- `examples/panel_bot.rs` builds (`cargo build -p botconf-telegram --examples`).

Commit 2.

### Step 5: The assistant bot still builds

In a scratch checkout of the assistant bot, point both botconf dependencies
at this repository by path (`[patch."https://github.com/propheci/botconf"]`
in its `.cargo/config.toml` or a temporary `Cargo.toml` edit; do not commit
it there) and run `cargo test --workspace`.

**Verify**: exit 0 with no source change in the assistant bot. If its code
must change, STOP (the public API was meant to stay).

### Step 6: Docs and tag

README: the crate list gains `botconf-panel`; a "Writing a front end"
section (render `Text` and rows, send tokens back, keep `Question`s, register
domains, pass `Limits`). Add `botconf-panel` to the `msrv` matrix at 1.95.
Commit 3, tag `v0.4.0`.

## Test plan

| Test | Crate | Proves |
|---|---|---|
| the 4 moved `callback.rs` tests | panel | tokens round-trip, parents, refusal of long data, malformed data |
| `tokens_fit_the_limit` | panel | limits are honoured |
| `screens_are_paged_when_rows_are_limited` | panel | Discord-sized screens work |
| `removing_a_changed_item_is_refused` | panel | position + check is safe |
| the 10 scenario tests from `handlers.rs` | panel | every editor behaves as before |
| input and text tests | panel | answers and the text command as before |
| HTML, callback data, pickers, posting | telegram | the renderer is faithful |
| assistant bot suite (Step 5) | assistant | public API unchanged |

## Done criteria

- [ ] `botconf-panel` has no teloxide dependency
- [ ] Every pre-existing test name passes in one of the two crates
- [ ] Assistant bot passes its suite against this tree without code changes
- [ ] Clippy, fmt, minimum Rust 1.95 for `botconf-panel` pass
- [ ] `v0.4.0` tagged locally; README updated; `plans/README.md` row updated

## STOP conditions

- The drift check fails (plan 003 or 009 not landed, or landed differently).
- A screen's text or buttons must change to fit the span model (other than
  markup produced through spans): report which one.
- A test name from the baseline cannot be kept because its behaviour no
  longer exists.
- The assistant bot needs a code change (Step 5).
- `botconf-panel` ends up needing teloxide or any platform crate.

## Maintenance notes

- Plan 005 exposes `render`, `press`, `answer` and `command_text` over gRPC:
  the proto mirrors `Screen`, `Text`, `Ask`, `Input` and `Limits`.
- New kinds are added in `botconf` (the hint) and in `botconf-panel` (the
  editor). Front ends only change if the kind needs a platform picker.
- Server-side handle tokens remain possible later behind the same `token`
  field, if some platform's limit is ever below what the stateless encoding
  needs.
