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
| 6 | **Hybrid tokens** (owner, 2026-10-25): a button's token is the stateless encoding of its action when that fits the front end's limit, and otherwise a short handle stored in a `TokenStore` that the host provides. Actions never point into a list that can change by position. See "Tokens" below. |

Decided while planning, confirmed by the owner on 2026-10-25:

- **Tokens are hybrid.** See decision 6 and the "Tokens" section. The
  planning draft used only stateless tokens, with a position plus a check for
  removing list items. The owner chose the hybrid instead, because wuphf's
  ids (rule ids, `<connector>:<address>` chat references) are often too long
  for a stateless token, and a choice picked by position can pick the wrong
  value when the list changed between drawing the panel and the press.
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
- **Prompts stay with the front end.** The engine keeps no per-conversation
  state: asking for a value returns an `Ask` holding a `Question` (which has
  a compact string encoding), and the front end keeps it until the answer
  arrives, with its own prompt mechanism. `botconf-telegram`'s `Prompts`
  (used by the assistant bot's other modules too) stays in
  `botconf-telegram`. Over gRPC (plan 005) the server keeps no
  per-conversation state either: the encoded `Question` goes to the client,
  which sends it back with the answer (the Go SDK keeps it). The only state
  is the token store, which the host owns and persists.

## Tokens

Every button carries a token, which the front end sends back unread. The
rule: **an action names the exact values it acts on, never a position in a
list that can change. Its token is the action's stateless encoding when that
fits `limits.token_bytes`, and otherwise a stored handle.**

**Stateless form.** Today's `callback.rs` encoding without the prefix (the
front end adds its own), e.g. `o:3.lights#2`, `p:5:1`. It needs no storage,
works across restarts and across processes, and is what almost every button
of the assistant bot uses.

**What an action may point at by position.** Only `Choices::Fixed` lists,
which are compiled into the program and cannot change while it runs:
`Pick(Target, usize)` and `Toggle(Target, usize)` keep today's encoding for
them. Choices from `KeysOf` or `Dynamic` lists (modules, lights, scenes,
wuphf rules and chats) are bound by value: new actions `PickValue(Target,
Value)` and `ToggleValue(Target, Value)`, encoded `P:<target>:<json>` and
`T:<target>:<json>` (the value as compact JSON; uppercase codes are unused
today apart from `R`). Removing a list item, `Remove(Target, Value)`, binds
the item as today (`r:<target>:<item>`, now JSON; a non-JSON argument is
read as a string, so `r` tokens from before this plan still work). Targets
and map entry names are already exact.

**Stored form.** When the encoded action is longer than the limit, the engine
stores it and puts a handle in the button:

- The handle is `~` followed by 10 base62 characters (11 bytes, which fits
  every front end). `~` never starts a stateless token, since action codes
  are letters.
- The 10 characters come from a 64-bit hash of the encoded action, using a
  fixed algorithm implemented in the crate (FNV-1a), not `DefaultHasher`,
  whose output may change between Rust releases. The same action gets the
  same handle, so redrawing a screen renews the existing row instead of
  adding one.
- On a collision (`put` reports that the handle holds a different, unexpired
  action), the engine derives the next handle from the action and an attempt
  number, up to 3 attempts. After that it leaves the button out and logs a
  warning, as it leaves out buttons that do not fit today.
- Each stored action expires after a TTL, 30 days by default
  (`Panel::token_ttl(Duration)`). Every render that issues the handle again
  renews it.

**Stale presses.** An unknown or expired handle returns `PanelError::Stale
{ notice: "This panel is out of date", screen }` with the home screen,
because the page it belonged to is unknown. A value-bound action whose value
is gone (a toggled module or a removed list item no longer listed, or an
index token from before this plan pointing into a list that is no longer
`Fixed`) returns `Stale { notice: "This list changed", screen }` with the
target's page redrawn. Front ends show the notice and redraw the message.

**Purging.** Opportunistic: the `Panel` remembers when it last purged (in
memory). A `put` more than an hour after the last purge first calls
`purge_expired(now)`. Hosts then get clean-up without having to run a timer
task, and the cost is at most one `DELETE` an hour; deduplicated handles keep
the table small anyway. `Panel::purge_tokens()` is public for hosts that
prefer their own timer.

**Who owns the store.** The host. `Panel::new` uses a `MemoryTokenStore`
(handles last until the process stops), so nothing changes for a host that
never needs handles. A host whose ids are long passes a persistent store:
the sea-orm one from this crate, or its own (wuphf writes a sqlx one).

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
in the assistant bot, only Step 7 (the persistent token store and its
migration) — its use of the existing public API must not break.

**Out of scope**: the wire format and gRPC (plan 005); Go (006, 007); new
editors or kinds; changing what any screen says (text and buttons must stay
the same, apart from markup being produced through spans).

## Git workflow

On `main`:
1. `feat(botconf-panel): add the platform-neutral screen engine`
2. `refactor(botconf-telegram): render the engine's screens`
3. `docs: describe the panel engine and how to write a front end`
Tag `v0.4.0` locally. Step 7 is one commit in `assistant_bot_rs`.

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
    /// Ids worth offering as buttons when adding to a reference setting, e.g.
    /// the chats a host has seen recently. `query` narrows them (empty for
    /// all); the engine shows at most `limit`. Default: none, so only the
    /// platform picker and typed ids are offered.
    fn candidates<'a>(
        &'a self,
        _domain: &'a str,
        _query: &'a str,
        _limit: usize,
    ) -> BoxFuture<'a, Vec<(Value, Name)>> {
        Box::pin(async { Vec::new() })
    }
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

/// Keeps the actions of buttons whose stateless token would be too long
/// (see "Tokens"). Same style as `botconf::Storage`.
pub trait TokenStore: Send + Sync + 'static {
    /// Stores `action` under `key` until `expires_at_ms`. If `key` holds the
    /// same action, or an action that expired before `now_ms`, it is replaced
    /// (renewing the expiry). Returns `false`, changing nothing, if `key`
    /// holds a different unexpired action (a collision).
    fn put<'a>(&'a self, key: &'a str, action: &'a str, now_ms: i64, expires_at_ms: i64)
        -> BoxFuture<'a, Result<bool, StorageError>>;
    /// The action stored under `key`, unless it is absent or expired at `now_ms`.
    fn get<'a>(&'a self, key: &'a str, now_ms: i64)
        -> BoxFuture<'a, Result<Option<String>, StorageError>>;
    /// Deletes the entries expired at `now_ms`; returns how many.
    fn purge_expired(&self, now_ms: i64) -> BoxFuture<'_, Result<u64, StorageError>>;
}

/// Handles in memory: they last until the process stops. The default.
pub struct MemoryTokenStore { /* Mutex<HashMap<String, (String, i64)>> */ }

/// Handles in a SQL table (feature `sea-orm`), created by the host's
/// migrations with `create_tokens_table`.
#[cfg(feature = "sea-orm")]
pub struct SeaOrmTokenStore { /* db, table */ }
#[cfg(feature = "sea-orm")]
pub const DEFAULT_TOKENS_TABLE: &str = "panel_tokens";
/// `key` (string, primary key), `action` (text), `expires_at` (big integer,
/// milliseconds since the Unix epoch).
#[cfg(feature = "sea-orm")]
pub fn create_tokens_table(table: &str) -> TableCreateStatement;

/// The engine over a settings store.
pub struct Panel<S: Schema> { /* store, tokens, ttl, last_purge, domains, command, section_note */ }

impl<S: Schema> Panel<S> {
    /// Uses a `MemoryTokenStore` and a 30-day token TTL.
    pub fn new(store: Arc<SettingsStore<S>>) -> Self;
    pub fn token_store(self, tokens: impl TokenStore) -> Self;
    pub fn token_ttl(self, ttl: Duration) -> Self;
    /// Deletes expired handles now (also done at most hourly on its own).
    pub async fn purge_tokens(&self) -> Result<u64, PanelError>;
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
    /// A stateless token that posts `page` as a new message, for other
    /// panels' buttons; `None` if it does not fit (pages and section ids are
    /// short, so this is what `post_button` keeps using).
    pub fn post_token(&self, page: &Page, limits: &Limits) -> Option<String>;
    /// The same, falling back to a stored handle when it does not fit.
    pub async fn post_token_stored(&self, page: &Page, limits: &Limits) -> Result<String, PanelError>;
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
    /// The press or answer made no sense (a malformed token, a bad value);
    /// show it to the user and keep the panel.
    Rejected(String),
    /// The button was out of date (see "Tokens"): show `notice` and redraw
    /// the panel message with `screen`.
    Stale { notice: String, screen: Screen },
    /// The settings storage failed: an infrastructure error for the host.
    Storage(SettingsError),
    /// The token store failed: an infrastructure error for the host.
    Tokens(StorageError),
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
  actions (rename `Button` to `Action`), `encode() -> String` without the
  prefix (the front end adds its own) and `parse`. `Page` gains an optional
  page number for paged screens (`o:s:3#1@2` style; choose a separator that
  cannot occur in entry names — entry names already exclude `:` and `#`, so
  use `@`). Add the value-bound actions `PickValue(Target, Value)` (`P`) and
  `ToggleValue(Target, Value)` (`T`), and make `Remove` carry a `Value`
  (`r`, JSON, with a non-JSON argument read as a string), as described in
  "Tokens". Move its 4 tests, adjusted for the page encoding, keeping their
  names; `data_that_does_not_fit_is_refused` now tests the issuing step
  below, which falls back to a handle instead of refusing.
- `botconf-panel/src/tokens.rs` (new): `TokenStore`, `MemoryTokenStore`, the
  FNV-1a handle derivation, and `issue(action, limits, store, ttl, now) ->
  Result<Option<String>, StorageError>` (stateless if it fits, else a
  handle; `None` only after 3 collisions) and `redeem(token, store, now) ->
  Result<Option<Action>, StorageError>` (`None`: unknown or expired). The
  opportunistic hourly purge lives here, keyed on an `AtomicI64` held by the
  `Panel`.
- `botconf-panel/src/tokens/sea_orm.rs` behind a `sea-orm` feature:
  `SeaOrmTokenStore` and `create_tokens_table`, modelled on
  `botconf::storage::SeaOrmStorage` (same `Alias`-built queries). `put` is
  one `INSERT … ON CONFLICT(key) DO UPDATE SET action = excluded.action,
  expires_at = excluded.expires_at WHERE <table>.action = excluded.action OR
  <table>.expires_at <= :now`, and reports `rows_affected == 1`; SQLite and
  Postgres both accept `ON CONFLICT … DO UPDATE … WHERE`. If sea-query
  cannot express the `WHERE` (`OnConflict::action_and_where`), use a
  transaction with a `SELECT` then an insert or update instead.
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
- Buttons: `ButtonView { label, token }` from `tokens::issue(action, …)`:
  the stateless encoding when it fits `limits.token_bytes`, else a stored
  handle. A button is left out only when `issue` returns `None` (3
  collisions) or the limit is below a handle's 11 bytes.
- Choice buttons use `Pick`/`Toggle` with an index for `Choices::Fixed`
  lists and `PickValue`/`ToggleValue` for `KeysOf` and `Dynamic` ones; list
  items use `Remove(Target, Value)`.
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
- `tokens_fit_the_limit` — with `token_bytes = 20`, no token is longer than
  20 and every button of the `Toy` screens is present (long actions became
  handles).
- `long_entry_names_get_buttons_through_handles` — a map entry named with 60
  characters: its button's token starts with `~`, is 11 bytes, and pressing
  it opens that entry.
- `short_tokens_are_unchanged` — for the `Toy` home and section screens, every
  token whose action has no `KeysOf`/`Dynamic` choice equals today's
  `callback.rs` encoding without the prefix (copy the strings from the
  current tests), so the assistant bot's existing buttons keep working.
- `dynamic_picks_bind_the_value` — render a setting whose choices are
  `Dynamic`, reorder the list behind it (change what the dynamic function
  returns), press the button drawn for `b`: the stored value is `b`.
- `removing_an_item_removes_that_value` — render a list, remove another item
  through the store, press the remove button for the original item: exactly
  that item is gone; pressing it again → `PanelError::Stale` "This list
  changed" with the list redrawn.
- `expired_handles_are_stale` — a handle issued with a 1-minute TTL, pressed
  with the clock 2 minutes later (pass `now` through the tokens API; no
  sleeps) → `PanelError::Stale` "This panel is out of date" with the home
  screen. An unknown `~…` token behaves the same.
- `redrawing_reuses_handles` — rendering the same screen twice issues the
  same handles and leaves one row per handle in the `MemoryTokenStore`.
- `colliding_handles_take_the_next_key` — a store pre-filled with a
  different action under the first derived key: the button gets the second
  key and presses work.
- `expired_handles_are_purged_hourly` — two `put`s within an hour purge
  once; a `put` an hour later purges again (count `purge_expired` calls with
  a wrapping test store).
- `screens_are_paged_when_rows_are_limited` — `max_rows = 5` on a section
  with 9 settings: page 1 has 4 content rows plus a nav row with ▶; pressing
  ▶ shows the rest with ◀.

And in `tokens/sea_orm.rs` (feature `sea-orm`, `sqlite::memory:`, like
`botconf`'s `stores_and_replaces_overrides`):
`sea_orm_token_store_renews_refuses_collisions_and_purges` — put, put the
same action (renewed), put a different action under the same key (`false`),
put over an expired entry (`true`), `get` after expiry (`None`),
`purge_expired` count.

### Step 3: Presses, answers and the text command

- `press`: `handle_button` + `edit_for` logic (`handlers.rs:121-310`) minus
  Telegram: redeem the token (`tokens::redeem`: a `~` handle is looked up in
  the store, anything else is parsed; an unknown or expired handle →
  `PanelError::Stale` with the home screen; a value-bound action whose value
  is gone → `Stale` with the target's page); `Open` → `screen`; `Post` →
  `post`; `Ask` →
  `ask` (built by `input::question` logic, minus the reply keyboards: the
  picker is reported as `Picker { domain, many }`; for an `Items` ask on a
  `Kind::Ref` setting, the question screen also lists up to
  `Limits.max_rows`-bounded `Names::candidates(domain, "", n)` as buttons,
  each a value-bound add action — not already in the list, labelled with the
  short name; typed text that `resolve` cannot read is retried as a
  `candidates` query and, with one match, added); edits → apply via
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
  hooks. New builder methods `token_store(…)` and `token_ttl(…)` pass through
  to the engine; without them it uses the engine's `MemoryTokenStore`, so the
  assistant bot builds unchanged (Step 5). `botconf-telegram` re-exports
  `TokenStore` and `MemoryTokenStore`, and gains a `sea-orm` feature that
  enables `botconf-panel/sea-orm` and re-exports `SeaOrmTokenStore`,
  `DEFAULT_TOKENS_TABLE` and `create_tokens_table`.
- `PanelError::Stale { notice, screen }` → answer the callback with `notice`
  as a toast and edit the panel message to `screen`. `PanelError::Tokens` is
  returned as an error, like `Storage`.
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
section (render `Text` and rows, send tokens back unread, keep `Question`s,
register domains, pass `Limits`, handle `Stale`); a "Tokens" section (the
hybrid rule, when a host needs a persistent `TokenStore`, the table from
`create_tokens_table`, the TTL and the hourly purge). Add `botconf-panel` to
the `msrv` matrix at 1.95. Commit 3, tag `v0.4.0`.

### Step 7: The assistant bot keeps handles across restarts

In `assistant_bot_rs`, on its own branch or main per that repo's rules,
after bumping its botconf tag to `v0.4.0`:

1. Enable `botconf-telegram`'s `sea-orm` feature.
2. Add a migration (`migration/src/m20261025_000001_create_panel_tokens.rs`,
   or the next free date) that runs
   `botconf_telegram::create_tokens_table(botconf_telegram::DEFAULT_TOKENS_TABLE)`,
   with a `down` that drops the table, and register it in
   `migration/src/lib.rs`.
3. In `src/modules/settings.rs::panel`, add
   `.token_store(botconf_telegram::SeaOrmTokenStore::new(ctx.db.clone()))`
   (use however `AppContext` exposes the connection).
4. `cargo test --workspace` and `cargo run -- migrate` against a copy of the
   local SQLite file.

The assistant bot's ids are short, so nearly all its buttons stay stateless;
this only keeps the occasional long one (a long preset or schedule name)
working after a restart. Commit there:
`feat(settings): keep panel button handles in the database`.

**Verify**: its suite passes; `sqlite3 <copy> '.schema panel_tokens'` shows
`key`, `action`, `expires_at`.

## Test plan

| Test | Crate | Proves |
|---|---|---|
| the 4 moved `callback.rs` tests | panel | tokens round-trip, parents, long data falls back to a handle, malformed data |
| `tokens_fit_the_limit` | panel | limits are honoured without dropping buttons |
| `long_entry_names_get_buttons_through_handles` | panel | long ids still get buttons |
| `short_tokens_are_unchanged` | panel | the assistant bot's existing buttons keep working |
| `dynamic_picks_bind_the_value` | panel | a reordered list cannot misapply a press |
| `removing_an_item_removes_that_value` | panel | removal is by value; stale presses redraw |
| `expired_handles_are_stale` | panel | expiry and unknown handles |
| `redrawing_reuses_handles`, `colliding_handles_take_the_next_key`, `expired_handles_are_purged_hourly` | panel | the store stays small and correct |
| `sea_orm_token_store_renews_refuses_collisions_and_purges` | panel (`sea-orm`) | the SQL store's semantics |
| `ref_items_offer_candidates` | panel | adding to a `Kind::Ref` list shows `Names::candidates` (minus ids already listed) as buttons; pressing one adds that id; typed text matching one candidate adds it |
| `screens_are_paged_when_rows_are_limited` | panel | Discord-sized screens work |
| the 10 scenario tests from `handlers.rs` | panel | every editor behaves as before |
| input and text tests | panel | answers and the text command as before |
| HTML, callback data, pickers, posting | telegram | the renderer is faithful |
| assistant bot suite (Step 5) | assistant | public API unchanged |

## Done criteria

- [ ] `botconf-panel` has no teloxide dependency
- [ ] Every pre-existing test name passes in one of the two crates
- [ ] Assistant bot passes its suite against this tree without code changes
- [ ] Clippy, fmt, minimum Rust 1.95 for `botconf-panel` pass
- [ ] Hybrid tokens: no button points into a `KeysOf`/`Dynamic` list by
      position; long actions get handles; the sea-orm store passes its test
- [ ] `v0.4.0` tagged locally; README updated; `plans/README.md` row updated
- [ ] Assistant bot keeps handles in its database (Step 7)

## STOP conditions

- The drift check fails (plan 003 or 009 not landed, or landed differently).
- A screen's text or buttons must change to fit the span model (other than
  markup produced through spans): report which one.
- A test name from the baseline cannot be kept because its behaviour no
  longer exists.
- The assistant bot needs a code change (Step 5).
- `botconf-panel` ends up needing teloxide or any platform crate.
- The sea-orm token store can express neither the conditional upsert nor the
  transaction fallback against both SQLite and Postgres.

## Maintenance notes

- Plan 005 exposes `render`, `press`, `answer` and `command_text` over gRPC:
  the proto mirrors `Screen`, `Text`, `Ask`, `Input` and `Limits`.
- New kinds are added in `botconf` (the hint) and in `botconf-panel` (the
  editor). Front ends only change if the kind needs a platform picker.
- Hosts that need handles must give the panel a persistent `TokenStore`, or
  their long buttons stop working after a restart (they come back as
  `Stale`, never as a wrong action). wuphf writes a sqlx implementation.
- With handles available, the assistant bot's 20-character cap on light and
  preset names (`MAX_NAME_LEN`, set by the callback limit) could be raised.
  That is the assistant bot's call, not part of this plan.
