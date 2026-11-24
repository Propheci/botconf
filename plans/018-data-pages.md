# Plan 018: Data pages, host-defined list, detail, confirm and question screens in the panel, and where the panel was opened

> **Executor instructions**: as plan 015. Build limits: `nice -n 19`,
> `CARGO_BUILD_JOBS=2`, and `-p` only the crates touched until the gate. Never
> build on battery (`/sys/class/power_supply/AC/online` = 0 → stop).

## Status

- **Priority**: P1. wuphf's data panel (wuphf plans 074–077) needs it.
- **Effort**: M–L
- **Risk**: LOW–MED. Additive to `botconf-panel` and `botconf-grpc`; the
  renderers are unchanged, because a data page is an ordinary `Screen`.
- **Depends on**: `v0.16.1`
- **Category**: feature
- **Planned at**: `v0.16.1`, 2026-11-24

## Why this matters

The owner wants a database panel in wuphf's `/wuphf` (grilled 2026-11-24).
It needs browsing (lists, paging, search), detail views, actions with Yes/No
confirmation, and typed answers such as "older than N days". Today the panel
renders only settings. botconf gives the host a generic way to add pages.
The host supplies the data and the operations, so the assistant bot can
reuse it.

## Design decisions (settled with the owner, 2026-11-24)

| # | Decision |
|---|---|
| 1 | Data pages live in botconf, as generic screens fed by a host data source. They are not a second renderer in wuphf. |
| 2 | They are served through the same Settings service (Render, Act) and its authorizer, so every front end and the CLI get them. |
| 3 | Destructive actions always confirm (Yes/No); there is no undo. |

## Planner decisions

1. **`DataSource` trait** (in `botconf-panel`), registered with
   `Panel::data(entry_title, source)`. The settings root gains an entry
   button (wuphf: "🗄 Data"). It has three methods:
   - `page(&self, viewer, route) -> DataPage`;
   - `act(&self, viewer, route, action, input) -> ActOutcome`;
   - `open(&self, viewer, context) -> Option<Route>`, the in-context entry.

   `Route` is the host's opaque string, e.g. `conv/<id>/messages?q=…&p=2`.
   Buttons carry it through the existing token store, stored when long, as
   settings tokens are.
2. **`DataPage`** has a title, a body `Text`, `items: Vec<Item { label,
   route }>`, `actions: Vec<Action { id, label, kind }>`, `paging: Option<{
   page, more }>` and `back: Option<Route>`. `kind` is one of:
   - `Go(Route)`;
   - `Run`, done at once;
   - `Confirm { prompt }`, Yes/No first;
   - `Ask { prompt, input: Text | Number }`, a typed answer through the
     existing `Ask`/`Input` path, then `act`;
   - `Choose { prompt, options }`, a question with buttons, e.g. "records
     only / with mirror copies".

   The panel turns a `DataPage` into a `Screen`:
   - items as one button per row;
   - paging as ‹ Prev / Next ›;
   - actions as buttons;
   - Back last.

   The text is cut at `Limits.text_chars`.
3. **`ActOutcome`**: `Show(Route)` (go to a page, with an optional toast) or
   `Message(Text)` (a result screen with Back), and an error as a toast.
4. **Where the panel was opened:** `Viewer` gains `context: Option<Context {
   chat: String, thread: Option<String> }>`, the front end's native chat. A
   front end that knows it passes it; for Telegram, the chat or topic the
   command was sent in. `Render` with no page calls `DataSource::open` first;
   when it returns a route, the panel opens there instead of at the root.
5. **gRPC:** `RenderRequest` gains `context` (the same shape). `Act` and
   `Render` already carry the actor and the token. No new RPCs.
6. **Go:** the renderers (gotd bot, gotd user via inline, discordgo) are
   unchanged, since they draw `Screen`s. The Go client gains `context` on
   render.

## Steps

1. The `DataSource` trait, `DataPage`, routes and their tokens, and the
   `Screen` building, in `botconf-panel`, with tests. A fake source with 25
   items should produce a page of 10 with Next; a `Confirm` action should show
   Yes/No and run only on Yes; `Ask` should take a number; `Choose` should
   offer its options.
2. `Viewer.context` and the in-context `open`, with tests.
3. `botconf-grpc` and proto: `context` on render, and the Act/Render paths
   for data tokens. Regenerate the Go bindings; update the Go client.
4. The CLI front end (`botconf-tui`): data pages render as screens too.
5. Docs; tag `v0.17.0` / `go/v0.17.0`. It is additive, so a minor version
   bump is enough.

## STOP conditions

- The token store can't hold a route of the length real routes reach (a
  search query in it). Then keep queries server-side: the route names a
  stored query id.
