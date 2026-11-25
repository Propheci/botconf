# Plan 019: Reference questions search and page through every candidate

> **Executor instructions**: as plan 015. Build limits: `nice -n 19`,
> `CARGO_BUILD_JOBS=2`, and `-p` only the crates touched until the gate. Never
> build on battery (`/sys/class/power_supply/AC/online` = 0 → stop).

## Status

- **Priority**: P1. wuphf's chat settings can only reach the first 8 chats.
- **Effort**: S–M
- **Risk**: LOW. Additive to `botconf-panel`; the renderers already show a
  question's rows and a question asked again after an answer.
- **Depends on**: `v0.17.0`
- **Category**: feature
- **Planned at**: `v0.17.0`, 2026-11-25

## Why this matters

The owner could not find a chat in wuphf's `/wuphf` chat pickers (2026-11-25):

- A list question offers the host's first 8 candidates and nothing else.
- A typed name is taken only when exactly one candidate matches it. Otherwise
  it is an error, or (for an unregistered domain) it is stored as typed.
- A single-reference question (e.g. a rule's source chat) offers no
  candidates at all.

## Design

1. **Pages of candidates.** A reference question (one value or a list)
   offers its candidates a page at a time, at most 8 or the front end's
   rows less 2, with ◀ / ▶ on their own row. Already chosen ones show
   ✅ (a list: pressing removes it) or 🔘 (one value). The others show ➕
   (adds) or ⚪ (sets the value, new `Action::PickRef`).
2. **Search.** If a typed answer names no reference, it is used as a search,
   whether it is one word or several. The question is asked again, showing
   the matching candidates as the same paged buttons. Send another word to
   search again; ‹ Back or /cancel gives up. A typed answer is read in
   this order:
   - the whole text as one reference: an id, a name the host resolves, or
     the only candidate that matches it;
   - then, for lists, each word the same way;
   - else a search. With no match, the domain's "unknown" error, or the text
     taken as typed for an unregistered domain, as before.
3. **`Action::Find(target, ask, Browse { query, offset })`**, encoded
   `F:<target>:<ask>:<offset>:<escaped query>`. It asks the question again
   with that page. On a draft it is wrapped in `InDraft`, like `Ask`.
4. **`Names::candidates_page(domain, query, offset, limit)`**, a default
   method built on `candidates`, so hosts need no change. Hosts with many
   ids override it.

## Steps

1. `token.rs`: `Browse`, `Action::Find`, `Action::PickRef`, encodings, and
   round-trip tests.
2. `lib.rs`: `Names::candidates_page`.
3. `input.rs`: paged candidate rows for both ask kinds, the search text,
   the reading order above (`Unread::{Rejected, Search}`).
4. `press.rs`: `Find` and `PickRef`, both directly and in drafts; a search
   answer asks again.
5. Tests: paging 20 → 8/8/4, search re-ask, a single ref picked, ✅ remove,
   in a draft, the reading order.
6. Gate: `cargo test -p botconf-panel`, clippy, fmt. Tag `v0.18.0`.
