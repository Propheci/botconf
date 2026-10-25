# Plan 007: Go panel renderers (Telegram bots, Telegram user accounts, Discord)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat go/v0.6.0..HEAD -- go/ proto/`
> Expected: empty (plan 006 tagged `go/v0.6.0`). What must still hold:
> `go/settings` has `Client` with `Render`, `Press`, `Answer`, `Command`,
> `Questions`, `Input`, `Page`, `Actor`; `settingspb` has `Screen`, `Row`,
> `Button`, `Text`, `Span`/`Style`, `Ask`, `Picker`, `Limits`, `ActResponse`
> (with `screen`, `post`, `notice`, `ask`, `closed`, `rejected`, `reply`, `stale`) as in
> plan 005's proto. If any differ, STOP and re-plan the mapping below.

## Status

- **Priority**: P2 (wuphf's Telegram panel plan needs the Telegram part;
  Discord can follow when a Discord connector exists)
- **Effort**: L
- **Risk**: MED — the Telegram user-account path relies on inline-bot
  mechanics across two sessions
- **Depends on**: 006
- **Category**: feature
- **Planned at**: before 001–006 landed, 2026-10-25; gotd APIs checked in
  `github.com/gotd/td v0.144.0` (the version wuphf's Telegram connector uses)

## Why this matters

The panel engine (004) decides every screen and every button's effect; the
service (005) serves it; the client (006) reaches it from Go. What is left
is drawing a `Screen` on each platform and turning presses and replies back
into `Press`/`Answer`/`Command` calls. These renderers are shared by every
Go program that shows a botconf panel, starting with wuphf's Telegram
connector, which can run as a **bot** or as a **user account**, and later a
Discord connector.

## Design decisions (settled with the owner — do not revisit)

| # | Decision |
|---|---|
| 1 | Renderers only map `Screen` ↔ platform. No settings logic, no authorization (the host's authorizer decides, using the actor the renderer reports). |
| 2 | Telegram bot sessions: inline keyboards with callback data `prefix + token`. |
| 3 | Telegram user sessions cannot attach callback buttons, so they use the **inline-bot trick**: a helper bot (a second gotd client with a bot token, run by the same program) answers an inline query with the rendered screen; the user session sends that result with `messages.sendInlineBotResult`; button presses then reach the bot as inline callback queries (with an `inline_message_id`); the bot edits the message with `messages.editInlineBotMessage`. The inline query string carries only a short token (Telegram allows 256 characters). |
| 4 | Discord: message components (buttons). |
| 5 | When the helper bot is unavailable, a user session falls back to text: numbered options and text commands. |

Decided while planning. Items marked "confirmed" were confirmed by the owner
on 2026-10-25; the others are open to review:

- **Package layout** (confirmed by the owner 2026-10-25): `go/render/telegram` (gotd, both session kinds) and
  `go/render/discord` (discordgo). Each depends only on `go/settings`,
  `settingspb` and its platform library, so a Telegram connector does not
  pull in discordgo.
- **The inline token is a random, single-use, short-lived handle** (confirmed by the owner 2026-10-25): 16
  random bytes, base64url, 22 characters, expiring after 2 minutes, that maps
  to a rendered `Screen` kept in memory by the renderer. The bot answers an
  inline query only when the token is known **and** the querying user is the
  user session's own account (the bot is told that id), so nobody else can
  pull a panel through the bot. The query is `Private` and `CacheTime: 0`.
- **Inline edits go to the message's data center.** `InputBotInlineMessageID`
  carries a `DCID`; Telegram requires `editInlineBotMessage` to be sent to
  that DC. gotd's `Client.DC(ctx, dc, max)` returns an invoker for it.
- **Text** is rendered with gotd's `telegram/message/entity` builder (UTF-16
  offsets for entities), not HTML, so the same code serves both the bot
  path and the inline result (whose `InputBotInlineMessageText` takes raw
  entities).
- **Discord prompts use modals** (confirmed by the owner 2026-10-25). An `Ask` opens a modal with one text input
  instead of waiting for the next message; the question is kept in
  `settings.Questions` and the modal's custom id carries only a short key.
  Pickers for domains `discord.user` and `discord.channel` use Discord's user
  and channel select menus. Discord interactions must be acknowledged within
  3 seconds: the renderer defers the update first, then edits.
- **Limits**: Telegram `{token_bytes: 64 - len(prefix), text_chars: 4096,
  max_rows: 0, max_per_row: 8}`; Discord `{token_bytes: 100 - len(prefix),
  text_chars: 2000, max_rows: 5, max_per_row: 5}` (button labels are cut to
  80 characters by the renderer).

## Current state

- `go/settings` (006): `Client.Render/Press/Answer/Command`,
  `Questions`, `Input`, typed errors.
- gotd `v0.144.0` (checked in the module cache):
  - `tg.UpdateDispatcher.OnBotCallbackQuery` (`tl_handlers_gen.go:497`),
    `OnBotInlineQuery` (`:467`), `OnInlineBotCallbackQuery` (`:517`);
  - `tg.InputBotInlineMessageID{DCID int, …}` (and the 64-bit variant), both
    with `GetDCID()`;
  - `tg.MessagesEditInlineBotMessageRequest{ID, Message, Entities,
    ReplyMarkup, …}`;
  - `telegram.Client.DC(ctx, dc int, max int64) (CloseInvoker, error)`
    (`telegram/pool.go:186`);
  - `telegram/message/entity` (text + entities builder);
  - requests used: `MessagesSendMessage`, `MessagesEditMessage`,
    `MessagesSetBotCallbackAnswer`, `MessagesGetInlineBotResults`
    (returns `*tg.MessagesBotResults{QueryID, Results}`),
    `MessagesSendInlineBotResult{Peer, RandomID, QueryID, ID}`,
    `MessagesSetInlineBotResults{QueryID, Results, CacheTime, Private}`
    with `tg.InputBotInlineResult{ID, Type: "article", Title, SendMessage:
    &tg.InputBotInlineMessageText{Message, Entities, ReplyMarkup}}`.
- wuphf's Telegram connector already answers a callback query from a bot
  session (`connectors/telegram/internal/telegram/connector.go:60-69`) and
  tests gotd requests with a recording invoker
  (`connectors/telegram/internal/telegram/sticker_label_test.go:15-36`);
  copy that test pattern.
- The helper bot must have inline mode enabled in @BotFather (`/setinline`).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Build | `cd go && go build ./... && go vet ./...` | exit 0 |
| Tests | `cd go && go test -race ./render/...` | exit 0 |
| Format | `cd go && test -z "$(gofmt -l .)"` | exit 0 |
| Import hygiene | `cd go && go list -deps ./render/telegram \| grep -c discordgo` | `0` |

## Scope

**In scope**: `go/render/telegram/**`, `go/render/discord/**`,
`go/go.mod`/`go.sum` (gotd `v0.144.0`, `github.com/bwmarrin/discordgo` latest
release), README, plans index.

**Out of scope**: any change to the proto, the engine or the client (STOP if
needed); command routing in a host (deciding which messages are panel
commands is the host's job; the renderer offers helpers); authorization.

## Git workflow

On `main`:
1. `feat(go): render settings panels for Telegram bot sessions`
2. `feat(go): render settings panels for Telegram user sessions through a helper bot`
3. `feat(go): render settings panels for Discord`
4. `docs: describe the Go renderers`
Tag `v0.7.0` and `go/v0.7.0`.

## Steps

### Step 1: Shared pieces in `go/render/telegram`

- `text.go`: `func Entities(t *settingspb.Text) (string, []tg.MessageEntityClass)`
  using `entity.Builder` (bold, italic, code, pre). Truncate to the limit.
- `keyboard.go`: `func Keyboard(prefix string, rows []*settingspb.Row) *tg.ReplyInlineMarkup`
  (callback buttons, data `prefix + token`); `nil` for no rows.
- `limits.go`: the Telegram `Limits` for a prefix.

Tests: `TestEntitiesUseUTF16Offsets` (an emoji before a bold span gives the
right offset), `TestKeyboardPrefixesTokens`, `TestEmptyRowsGiveNoMarkup`.

### Step 2: Bot sessions (`bot.go`)

```go
// BotPanel shows a settings panel through a Telegram bot session.
type BotPanel struct{ /* client *settings.Client, api *tg.Client, prefix, questions *settings.Questions, actorDomain string */ }

func NewBotPanel(client *settings.Client, api *tg.Client, opts ...BotOption) *BotPanel
// Open posts the home screen (or a section) in peer, acting for user.
func (p *BotPanel) Open(ctx context.Context, peer tg.InputPeerClass, user int64, page settings.Page) error
// HandleCallback handles presses of this panel's buttons; false if the data is not ours.
func (p *BotPanel) HandleCallback(ctx context.Context, update *tg.UpdateBotCallbackQuery) (bool, error)
// HandleMessage answers a pending question from this user in this chat; false if none.
func (p *BotPanel) HandleMessage(ctx context.Context, peer tg.InputPeerClass, msg *tg.Message) (bool, error)
// Command runs a text command ("set x 1"), or opens the panel when empty.
func (p *BotPanel) Command(ctx context.Context, peer tg.InputPeerClass, user int64, args string) error
```

Mapping of an `ActResponse`:
- `screen` → `MessagesEditMessage` of the panel message (ignore
  `MESSAGE_NOT_MODIFIED`);
- `post` → `MessagesSendMessage`;
- `notice` / `rejected` → `MessagesSetBotCallbackAnswer` text (alert for
  `rejected`);
- `stale` (plan 004's hybrid tokens: an expired or unknown handle, or a
  value that is gone) → `notice` as a plain toast, and `screen` edits the
  panel like any redraw. Renderers never parse tokens, handles included: a
  token is sent back exactly as received;
- `ask` → send the question text with a force-reply, keep it in `Questions`
  (conversation = peer id, user = user id, panel message id); pickers:
  bot sessions can offer `KeyboardButtonRequestPeer` for
  `telegram.user`/`telegram.chat` in private chats — map picked peers to
  `Input.Picked`;
- `closed` → edit with the screen and no markup;
- `reply` → send as a reply to the command.
- Every press is answered (`MessagesSetBotCallbackAnswer`) exactly once.

Actor: `settings.Actor{Domain: "telegram.user", ID: strconv.FormatInt(user, 10)}`.

Tests (recording invoker, fake settings server from 006's tests exported as
`go/settings/settingstest`): `TestOpenSendsTheHomeScreen`,
`TestPressEditsThePanel`, `TestRejectedPressShowsAnAlert`,
`TestStalePressShowsANoticeAndRedraws`,
`TestAskThenAnswerSendsTheAnswer`, `TestForeignCallbackDataIsIgnored`,
`TestEveryPressIsAnswered`.

Commit 1.

### Step 3: User sessions through a helper bot (`inline.go`)

```go
// InlinePanel shows a settings panel from a user account, through a helper bot
// in inline mode. user is the account's client, bot the helper bot's.
type InlinePanel struct{ /* client, user *tg.Client, bot *telegram.Client, botUser tg.InputUserClass, ownUserID int64, screens map[string]pending, questions */ }

func NewInlinePanel(client *settings.Client, user *tg.Client, bot *telegram.Client, botUser tg.InputUserClass, ownUserID int64, opts ...InlineOption) *InlinePanel

// Open renders page and posts it in peer from the user account.
func (p *InlinePanel) Open(ctx context.Context, peer tg.InputPeerClass, page settings.Page) error
// For the helper bot's dispatcher:
func (p *InlinePanel) HandleInlineQuery(ctx context.Context, update *tg.UpdateBotInlineQuery) error
func (p *InlinePanel) HandleInlineCallback(ctx context.Context, update *tg.UpdateInlineBotCallbackQuery) (bool, error)
// For the user account's own outgoing messages (answers, text commands):
func (p *InlinePanel) HandleMessage(ctx context.Context, peer tg.InputPeerClass, msg *tg.Message) (bool, error)
```

Flow of `Open` (and of `post` reactions):
1. Render (or take the reaction's screen); store it under a fresh token
   (2-minute expiry, single use).
2. `user.MessagesGetInlineBotResults(bot, peer, query = token)`.
3. The helper bot's `HandleInlineQuery` receives the query: if
   `update.UserID == ownUserID` and the token is known, answer with
   `MessagesSetInlineBotResults{Results: [one article with the screen's text,
   entities and keyboard], CacheTime: 0, Private: true}` and drop the token;
   otherwise answer with no results.
4. `user.MessagesSendInlineBotResult(peer, queryID, result id)`.

Presses arrive at the bot as `UpdateInlineBotCallbackQuery` (`MsgID`,
`Data`, `UserID`): call `Press` with the presser as actor, answer the
callback (`MessagesSetBotCallbackAnswer`), and edit via
`MessagesEditInlineBotMessage{ID: MsgID, Message, Entities, ReplyMarkup}`
invoked on `bot.DC(ctx, MsgID.GetDCID(), 1)`.

Questions: the answer is the user account's own next message in that chat
(outgoing). `HandleMessage` takes it when a question is pending for
(peer, own user), calls `Answer`, deletes the question and answer messages,
and edits the panel. Pickers are not available (typed ids only).

Fallback (decision 5): `Open` with no helper bot configured, or when step 2
or 3 fails (timeout 5s), sends the screen as text with numbered options
(`1. Logging`, `2. Modules`, …) and keeps the number → token map in
`Questions`; replying with a number presses it; text commands still work.

Tests (two recording invokers, user and bot): `TestOpenGoesThroughTheHelperBot`
(GetInlineBotResults then SendInlineBotResult with the bot's result id),
`TestInlineQueriesFromOthersGetNothing`, `TestTokensAreSingleUse`,
`TestInlinePressEditsOnTheMessagesDC` (the edit goes to the invoker for the
`DCID`), `TestOutgoingAnswerIsTaken`, `TestFallbackNumbersOptions`.

Commit 2.

### Step 4: Discord (`go/render/discord`)

```go
type Panel struct{ /* client, session *discordgo.Session, prefix, questions */ }
func NewPanel(client *settings.Client, session *discordgo.Session, opts ...Option) *Panel
func (p *Panel) Open(ctx context.Context, channelID, userID string, page settings.Page) error
// HandleInteraction handles this panel's component and modal interactions; false if not ours.
func (p *Panel) HandleInteraction(ctx context.Context, i *discordgo.InteractionCreate) (bool, error)
```

- Text: spans to Discord markdown (`**bold**`, `*italic*`, `` `code` ``,
  fenced blocks), escaping markdown characters in plain spans.
- Rows to `discordgo.ActionsRow` of `discordgo.Button{CustomID: prefix+token,
  Label: cut to 80}` (the engine pages to 5×5).
- Press: `InteractionRespond(DeferredMessageUpdate)`, then `Press`, then
  `InteractionResponseEdit` with the new screen; `notice`/`rejected` as an
  ephemeral follow-up (a `stale` response is the same: its notice as the
  follow-up, its screen as the edit).
- Ask: respond with a modal (`TextInput`), custom id `prefix + "q:" + key`;
  the submit calls `Answer`. Pickers for `discord.user`/`discord.channel` via
  select menus.
- Actor: `settings.Actor{Domain: "discord.user", ID: userID}`.

Tests (an interface over the three discordgo calls used, faked):
`TestMarkdownEscapesPlainText`, `TestRowsBecomeActionRows`,
`TestPressDefersThenEdits`, `TestAskOpensAModal`, `TestModalSubmitAnswers`.

Commit 3.

### Step 5: Docs and tag

README "Go renderers": what each covers, the helper bot setup (create a
bot, enable inline mode with @BotFather `/setinline`, give its token to the
program), the fallback, and the Discord intents needed (none beyond
interactions). Tag `v0.7.0` and `go/v0.7.0`. Commit 4.

## Test plan

The tests listed in Steps 1–4. All use recording/fake transports; none talk
to Telegram or Discord.

## Done criteria

- [ ] `go test -race ./render/...` passes; vet and gofmt clean
- [ ] `go/render/telegram` does not import discordgo
- [ ] No renderer decides anything about settings (review: every action is a
      `Press`/`Answer`/`Command` call with the token or text as given)
- [ ] Tags `v0.7.0`/`go/v0.7.0` locally; README and `plans/README.md` updated

## STOP conditions

- gotd v0.144.0 lacks a call named above, or `Client.DC` cannot reach the
  inline message's DC from a bot session.
- Telegram rejects `sendInlineBotResult` for a private inline result in a
  manual test the owner runs (ask the owner to try once with a real helper
  bot before Step 3's commit).
- A renderer needs a proto or engine change.

## Maintenance notes

- wuphf's Telegram connector plan wires `BotPanel` (bot sessions) and
  `InlinePanel` (user sessions with a helper bot), and routes `/wuphf`
  commands to `Command`. wuphf's sticker-label plan reuses the same helper
  bot for inline sticker results; it may want a small generic
  "send this through the helper bot" helper extracted from `inline.go`.
- Telegram limits a bot's inline results per query rate; panels are
  owner-only and rare, so no throttling is added.
