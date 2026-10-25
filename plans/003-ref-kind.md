# Plan 003: Replace the Telegram id kinds with `Kind::Ref` domains

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat v0.1.1..HEAD -- rust/`
> (use `v0.1.0` if plan 002 has not been tagged). Expected: empty. What must
> still hold: `rust/botconf/src/kind.rs` has `Kind::Chat`, `Kind::Users` and
> `Kind::Chats` as unit variants; `rust/botconf-telegram/src/names.rs` has
> `trait Names<R>` with `user(bot, UserId)` and `chat(bot, ChatId)`;
> `StoredOverride.by` is `Option<i64>`. If plan 004 (the panel engine) has
> already landed, STOP: this plan is written against the pre-engine
> `botconf-telegram` and must be re-planned against the engine.

## Status

- **Priority**: P1 (wuphf's settings name chats by connector and address,
  which the Telegram kinds cannot express)
- **Effort**: M
- **Risk**: MED — a breaking API change for both crates and a database
  migration in the assistant bot
- **Depends on**: 001, 002
- **Category**: API
- **Planned at**: assistant_bot_rs `d772d66` (the crates moved verbatim in
  001), 2026-10-25

## Why this matters

Three kinds hard-code Telegram: `Kind::Chat` (a Telegram chat id),
`Kind::Users` (Telegram user ids), `Kind::Chats` (Telegram chat ids). Front
ends decode their values with `as_i64()` and look names up through
`Names::user(UserId)` / `Names::chat(ChatId)`. wuphf names things by
platform-qualified references (a connector id plus an address such as a
WhatsApp LID, a phone JID or an Instagram thread id), and its Telegram,
WhatsApp and Discord connectors each need their own pickers. The core crate
should not know any platform: a reference names its **domain**, and each
front end registers how to pick, parse and name ids of the domains it knows.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | `Kind::Ref { domain: &'static str, many: bool }` **replaces** `Kind::Chat`, `Kind::Users` and `Kind::Chats` (no parallel variants). |
| 2 | Ids are opaque to botconf: the domain decides what they look like. Telegram is just two domains, `telegram.user` and `telegram.chat`. |
| 3 | Front ends register a picker and a name lookup **per domain**; `Names` becomes a lookup keyed by domain. |
| 4 | The assistant bot migrates once: its stored values (JSON numbers) do not change, only the kind hints and its `Names` implementation. |

Decided while planning (flag to the owner in the report; reviewers may push
back):

- **Actors become strings too** (wuphf's settings plans require it).
  `StoredOverride.by`, and every `by: Option<i64>` parameter on
  `SettingsStore` (`set`, `add`, `extend`, `remove`, `delete_entry`) and
  `command::execute`, become `Option<String>`, and the SeaORM `updated_by`
  column becomes `text`. Same reason as for references: a wuphf actor is a
  platform user id such as `123456789@lid`, which does not fit an `i64`.
  botconf records actors and never interprets them. The assistant bot gets a
  migration for its `settings.updated_by` column (Step 5 explains why it is
  optional for reading on SQLite but still done).
- **How a domain stores its ids.** botconf stores whatever JSON the host's
  config type deserializes from. A domain handler converts typed text or a
  picked entity into that JSON (`telegram.*` produce numbers). For a domain
  the front end does not know, typed input is read with `parse_value`
  (digits become numbers, anything else a string), which matches today's
  behaviour for `Kind::Json`.
- **Nouns.** The panel shows "3 users" / "3 chats" today. A domain handler
  gives its noun (`user`/`users`); unknown domains say `item`/`items`.

## Current state

- `rust/botconf/src/kind.rs:16-66` — `pub enum Kind`. The id kinds:

  ```rust
  /// A chat id.
  Chat,            // line 51
  /// A list of user ids.
  Users,           // line 53
  /// A list of chat ids.
  Chats,           // line 55
  ```

  `Kind::empty_value` (line 70) returns `[]` for `SetOf | Users | Chats`.
- `rust/botconf/src/store.rs` — `set`, `add`, `extend`, `remove`,
  `delete_entry` and `modify` take `by: Option<i64>` (lines 148, 158, 169, 188,
  203, 269); `StoredOverride { key, value, by: Option<i64> }` in
  `rust/botconf/src/storage/mod.rs`; `SeaOrmStorage` reads and writes
  `updated_by` as `big_integer` (`rust/botconf/src/storage/sea_orm.rs`,
  `create_table`). `command::execute(store, command, by: Option<i64>)`
  (`rust/botconf/src/command.rs:119-123`).
- `rust/botconf/tests/store.rs:65` — `RuntimeSetting::new("admins", "Who's in charge").kind(Kind::Users)`.
- `rust/botconf-telegram/src/names.rs:20-41` — the trait:

  ```rust
  pub trait Names<R>: Send + Sync + 'static {
      fn user<'a>(&'a self, bot: &'a R, id: UserId) -> BoxFuture<'a, Option<Name>>;
      fn chat<'a>(&'a self, bot: &'a R, id: ChatId) -> BoxFuture<'a, Option<Name>>;
      fn user_by_username<'a>(&'a self, _username: &'a str) -> BoxFuture<'a, Option<UserId>> { … None }
      fn remember_users<'a>(&'a self, _users: &'a [SharedUser]) -> BoxFuture<'a, ()> { … }
      fn remember_chat<'a>(&'a self, _chat: &'a ChatShared) -> BoxFuture<'a, ()> { … }
  }
  ```

- `rust/botconf-telegram/src/panel.rs`:
  - `359` `Kind::Chat =>` shows the chat's name via `chat_name`;
  - `440-472` `Kind::Users | Kind::Chats =>` lists items, decoding each with
    `item.as_i64()`, naming it with `user_name`/`chat_name`, with ❌ remove
    buttons and ➕ Add (`Button::Ask(target, Ask::Items)`);
  - `614-615` summaries `count(n, "user", "users")` / `"chat", "chats"`;
  - `642` `Kind::Text { .. } | Kind::Chat | Kind::Json => plain(&value)`;
  - `648-671` `item_line(name, id: i64)`, `user_name`, `chat_name`.
- `rust/botconf-telegram/src/input.rs`:
  - `45-121` `question()`: `(Ask::Items, Kind::Users)` offers the user picker,
    `(Ask::Items, Kind::Chats)` and `(Ask::Value, Kind::Chat)` the chat
    pickers (`user_pickers()`, `chat_pickers()` at 123-152);
  - `value()` reads a picked chat (`message.shared_chat()`) or a typed chat id
    for `Kind::Chat`;
  - `items()` reads picked users/chats, else splits typed words and maps them
    with `user_id(panel, word)` (numbers, or `@username` through
    `names.user_by_username`) for `Kind::Users` and `chat_id(word)` for
    `Kind::Chats`.
- `rust/botconf-telegram/src/lib.rs:173` — `fn actor(user: UserId) -> Option<i64>`;
  `edit.rs:28-32` `apply(store, edit, by: Option<i64>)`.
- `rust/botconf-telegram/src/testing.rs:144-154` — the toy schema uses
  `Kind::Users` (`admins`), `per_entry("access", …, &Kind::Users)` and
  `Kind::Chat` (`alerts_chat`); `FakeNames` implements `Names<Bot>`.
- Assistant bot (`~/work/side_projects/telegram/assistants/assistant_bot_rs/main`):
  `src/settings/mod.rs:168, 172, 178, 186, 213` use `Kind::Users`,
  `&Kind::Users`, `&Kind::Chats`, `Kind::Chat`, `Kind::Users`;
  `src/settings/mod.rs` `pub fn actor(user: Option<UserId>) -> Option<i64>`;
  `src/directory.rs:242-271` implements `Names<AssistantBot>` for
  `DirectoryNames` (`user`, `chat`, `user_by_username`, `remember_users`,
  `remember_chat`); `migration/src/m20260926_000001_create_settings.rs`
  creates `settings.updated_by` as `big_integer().null()`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Tests | `cargo test --workspace --all-features` | exit 0; baseline + new tests |
| Lint | `cargo clippy --workspace --all-targets --all-features -- --deny warnings` | exit 0 |
| Format | `cargo fmt --check` | exit 0 |
| Minimum Rust | the two `msrv` commands from plan 002 | exit 0 |
| Assistant tests | `cargo test --workspace` in the assistant bot | exit 0 |

## Scope

**In scope**: `rust/botconf/src/{kind.rs,store.rs,command.rs,storage/*.rs,cli.rs}`,
`rust/botconf/tests/store.rs`, `rust/botconf-telegram/src/**`,
`rust/botconf-telegram/examples/panel_bot.rs`, README, plans index; in the
assistant bot (separate branch): `src/settings/mod.rs`, `src/directory.rs`,
`src/modules/settings.rs` if it touches actors, a new migration, `Cargo.toml`
(the tag).

**Out of scope**: the panel engine split (plan 004); the wire format (005);
any other kind.

## Git workflow

- BOTCONF, on `main`:
  1. `feat(botconf)!: replace the Telegram id kinds with Kind::Ref domains`
  2. `feat(botconf)!: record actors as strings`
  3. `feat(botconf-telegram)!: look names up by domain`
  4. `docs: describe reference kinds and domains`
  Tag `v0.2.0` locally (breaking changes).
- Assistant bot: branch `botconf-ref-kinds`, commit
  `build: move to botconf 0.2 (reference kinds, string actors)`. It needs the
  owner to push `v0.2.0` first (STOP until `git ls-remote --tags
  https://github.com/propheci/botconf v0.2.0` prints a line).

## Steps

### Step 0: Baseline

`cargo test --workspace --all-features 2>&1 | grep "^test result"` — record.

### Step 1: `Kind::Ref` in the core

In `rust/botconf/src/kind.rs`, replace the three variants with:

```rust
/// References to things outside the configuration: users, chats, accounts.
/// `domain` says what they refer to (e.g. `telegram.user`, `wuphf.chat`), so
/// front ends know how to pick them and how to name them; botconf itself
/// never interprets the ids.
Ref {
    domain: &'static str,
    /// A list of references, rather than one.
    many: bool,
},
```

`empty_value`: `Self::SetOf { .. } | Self::Ref { many: true, .. } => Some([])`.
Update the module docs. Fix the compile errors in `rust/botconf` (only
`kind.rs` and the tests should mention the old variants).

In `tests/store.rs:65`, use `Kind::Ref { domain: "test.user", many: true }`.
Add to `kind.rs` tests:

- `empty_value` of `Ref { many: true }` is `[]`; of `Ref { many: false }` is
  `None`.

**Verify**: `cargo test -p botconf --all-features` → exit 0.

### Step 2: Domain handling in botconf-telegram

1. Add `rust/botconf-telegram/src/domains.rs`:

   ```rust
   //! The reference domains the Telegram panel knows: Telegram's own users
   //! and chats, picked with Telegram's pickers.
   pub const USER: &str = "telegram.user";
   pub const CHAT: &str = "telegram.chat";
   ```

   Re-export as `botconf_telegram::domains`.
2. Replace `Names<R>` with a domain-keyed trait (`names.rs`):

   ```rust
   pub trait Names<R>: Send + Sync + 'static {
       /// The name of `id` in `domain`, e.g. a Telegram user. `None` shows the id.
       fn name<'a>(&'a self, bot: &'a R, domain: &'a str, id: &'a Value)
           -> BoxFuture<'a, Option<Name>>;
       /// The id `text` refers to in `domain`, besides a plain id, e.g. an
       /// `@username` for `telegram.user`.
       fn resolve<'a>(&'a self, _domain: &'a str, _text: &'a str) -> BoxFuture<'a, Option<Value>> {
           Box::pin(async { None })
       }
       fn remember_users<'a>(&'a self, _users: &'a [SharedUser]) -> BoxFuture<'a, ()> { … }
       fn remember_chat<'a>(&'a self, _chat: &'a ChatShared) -> BoxFuture<'a, ()> { … }
   }
   ```

   `remember_*` stay: they are what Telegram's pickers hand over.
3. Pickers and parsing (`input.rs`): `question()` offers the user picker for
   `Ref { domain: USER, .. }` and the chat pickers for `Ref { domain: CHAT, .. }`;
   any other domain asks for typed ids. `value()` and `items()` read a pick for
   the Telegram domains as today; typed words become: a number for the
   Telegram domains (`user_id`, `chat_id` as today, with `@username` through
   `names.resolve(USER, word)`), otherwise `names.resolve(domain, word)` and,
   failing that, `parse_value(word)`.
4. Display (`panel.rs`): the `Ref { many: true, .. }` branch replaces
   `Users | Chats`; the `Ref { many: false, .. }` branch replaces `Chat`. Items
   are any JSON scalar: name them with `names.name(bot, domain, item)`, show
   the id with `plain(item)` (not `as_i64`), and use `plain(item)` in the
   `Button::Remove` data. Summaries: `count(n, noun, nouns)` with
   `("user", "users")` for `USER`, `("chat", "chats")` for `CHAT`, else
   `("item", "items")`.
5. Toy schema (`testing.rs`): `Ref { domain: USER, many: true }` for `admins`
   and `access`, `Ref { domain: CHAT, many: false }` for `alerts_chat`;
   `FakeNames` implements `name` by matching the domain. Add one setting
   `Ref { domain: "toy.thing", many: true }` (`things: Vec<String>`) to cover an
   unknown domain.

**Verify**: `cargo test -p botconf-telegram` → the 21 existing tests pass
unchanged in substance (only the toy's kinds changed), plus the new ones
below.

New tests (in `handlers.rs` tests, next to
`known_users_and_chats_are_shown_by_name`):

- `references_of_unknown_domains_are_typed_and_listed`: add `"a@b"` and `"42"`
  to `things` through `Ask::Items` with typed text; the screen lists both,
  `things` is `["a@b", 42]` (`parse_value`), and ❌ removes `"a@b"`.
- `usernames_resolve_through_the_user_domain`: `FakeNames::resolve(USER, "@ann")`
  returns `7`; typing `@ann` adds `7` to `admins`.

Commit 3 after Step 3 (the crates must compile together).

### Step 3: Actors are strings

- `StoredOverride.by: Option<String>`; every `by: Option<i64>` in
  `store.rs`, `command.rs`, `edit.rs` becomes `Option<String>`.
- `SeaOrmStorage`: `create_table` makes `updated_by` a `text().null()` column;
  `load` selects it as `CAST(updated_by AS TEXT)` (sea-query
  `Expr::col(…).cast_as(Alias::new("TEXT"))`, aliased back to `updated_by`)
  and reads `Option<String>`. The cast matters on SQLite even after a
  migration: a column with INTEGER affinity (declared `bigint`, as older
  tables are) stores a numeric-looking string such as `"42"` as an INTEGER
  (checked with sqlite 3.51: `insert … values('42')` into a `bigint` column
  gives `typeof = integer`, while `'123@lid'` stays `text`), and sqlx refuses
  to decode an INTEGER value as `String`. With the cast, an old SQLite table
  loads without a migration. Postgres still needs one: it rejects a text
  parameter for a `bigint` column on write. Document both in the storage
  module.
- `botconf-telegram/src/lib.rs` `actor(user)` returns `Some(user.0.to_string())`.
- `cli.rs`: unchanged (`by` is `None`).

Tests (`storage/sea_orm.rs`):
- the existing `stores_and_replaces_overrides` uses string actors
  (`Some("7".into())`), plus a non-numeric actor (`Some("123@lid".into())`)
  that round-trips;
- `old_bigint_tables_still_load`: create the table the pre-003 way
  (`updated_by` as `big_integer`), insert a row with `updated_by = 42` and
  one with `'123@lid'` through raw SQL, then `load()` returns
  `Some("42")` and `Some("123@lid")`. Run it once **without** the cast to
  confirm it fails (proves the cast is needed); if it passes without the
  cast, keep the cast anyway and note it in the report.

**Verify**: `cargo test --workspace --all-features` → exit 0. Commits 1–2.

### Step 4: Docs and tag

README: a "Reference kinds" section: what a domain is, the two Telegram
domains, how a front end handles an unknown domain, and the `updated_by`
note for existing tables. Update `examples/panel_bot.rs` if it uses the old
kinds (it should not). Commit 4. Tag `v0.2.0`.

### Step 5: Move the assistant bot (after the owner pushes `v0.2.0`)

On the assistant bot's branch `botconf-ref-kinds`:

1. `Cargo.toml`: both botconf dependencies to `tag = "v0.2.0"`;
   `cargo update -p botconf -p botconf-telegram`.
2. `src/settings/mod.rs`: `Kind::Users` → `Kind::Ref { domain: domains::USER, many: true }`
   (lines 168, 172 as `&Kind::Ref { … }`, 213); `&Kind::Chats` →
   `&Kind::Ref { domain: domains::CHAT, many: true }` (178); `Kind::Chat` →
   `Kind::Ref { domain: domains::CHAT, many: false }` (186). `actor()` returns
   `Option<String>`.
3. `src/directory.rs`: `DirectoryNames` implements `name(bot, domain, id)`:
   `USER` → `id.as_u64()` then `directory.user`, `CHAT` → `id.as_i64()` then
   `directory.chat`, otherwise `None`; `resolve(USER, text)` → the old
   `user_by_username`, returning `Value::from(id)`.
4. A new migration (next to the existing ones in `migration/src/`, registered
   in `migration/src/lib.rs`) changing `settings.updated_by` to text.
   - **Is it needed on SQLite?** Not for reading: botconf's `load` casts the
     column to text (Step 3), so the existing `bigint` table loads as is.
     It is still worth doing, because with INTEGER affinity every
     numeric-looking actor (all Telegram user ids) keeps being stored as an
     INTEGER, which only the cast hides. With a TEXT column the stored type
     is what was written. The bot's real database is SQLite
     (`config.example.toml`), so the migration is cheap.
   - **On Postgres it is required**: inserting a text actor into a `bigint`
     column fails.
   - SQLite cannot alter a column type: create `settings_new` with
     `updated_by TEXT NULL`, `INSERT INTO settings_new SELECT key, value,
     CAST(updated_by AS TEXT), updated_at FROM settings`, drop `settings`,
     rename. Postgres: `ALTER TABLE settings ALTER COLUMN updated_by TYPE text
     USING updated_by::text`. Pick with `manager.get_database_backend()`.
   - Test with the bot's in-memory database helper: a row stored with a
     numeric `updated_by` before the migration reads back as `Some("42")`
     through `SeaOrmStorage::load` after it, and
     `SELECT typeof(updated_by)` is `text`.

**Verify**: `cargo test --workspace` in the assistant bot → exit 0; its
settings tests (`src/settings/mod.rs` tests) pass unchanged; `nix flake
check` → exit 0. Commit; leave the branch for the owner.

## Test plan

| Test | Where | Proves |
|---|---|---|
| `empty_value` of `Ref` | `botconf/src/kind.rs` | lists of references start empty; single ones have no default |
| `references_of_unknown_domains_are_typed_and_listed` | `botconf-telegram` handlers tests | non-Telegram ids are typed, stored as given, named by id, removable |
| `usernames_resolve_through_the_user_domain` | same | `resolve` replaces `user_by_username` |
| existing 21 `botconf-telegram` tests | same | Telegram pickers and names behave as before |
| string actor round-trip | `botconf/src/storage/sea_orm.rs` | non-numeric actors are stored |
| assistant migration test | assistant `migration` | old numeric `updated_by` values survive |

## Done criteria

- [ ] `grep -rn "Kind::Users\|Kind::Chats\|Kind::Chat\b" rust/` → no output
- [ ] `grep -rn "by: Option<i64>" rust/` → no output
- [ ] All tests, clippy, fmt and the minimum-version jobs pass
- [ ] `v0.2.0` tagged locally; README section written
- [ ] Assistant bot branch passes its suite on `v0.2.0` (after the push)
- [ ] `plans/README.md` row updated

## STOP conditions

- Plan 004 landed before this plan (see the drift check).
- A test needs an id to be numeric outside the Telegram domains.
- The assistant bot's settings tests need more than the kind and actor
  changes to pass.

## Maintenance notes

- Plan 009 runs next (owned descriptors, dynamic sections), then 004.
- Plan 004 moves domain handling into the panel engine (`Domains` registry
  with nouns, typed-id parsing and name lookup); this plan's
  `botconf-telegram` code is the reference behaviour it must keep.
- Plan 005 sends `domain` over the wire; Go renderers register handlers for
  the same domain strings.
- wuphf will declare domains such as `wuphf.chat` (a connector id plus an
  address) and `wuphf.connector`; their id format is wuphf's business.
