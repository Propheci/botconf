//! A platform-neutral settings panel for [`botconf`] settings.
//!
//! The engine computes the panel's screens once, in Rust: a home screen, one
//! per section, and one per setting with an editor fitting its
//! [`Kind`](botconf::Kind) (toggles, pick-one buttons, number pickers, forms,
//! reference lists, maps). Front ends (Telegram, Discord, a gRPC client, ...)
//! only draw [`Screen`]s and report what was pressed or typed:
//!
//! 1. show [`Panel::render`]'s screen: its [`Text`] in the front end's markup,
//!    its rows as buttons carrying their tokens;
//! 2. send a pressed button's token back, unread, to [`Panel::press`];
//! 3. act on the [`Reaction`]: redraw the panel, post a new one, show the
//!    notice, or ask a question ([`Ask`]) and keep its [`Question`] until the
//!    answer arrives for [`Panel::answer`];
//! 4. on [`PanelError::Stale`], show the notice and redraw.
//!
//! Each front end passes its [`Limits`] (token size, text length, rows), and
//! registers what it knows about reference [`Domain`]s.
//!
//! Button tokens are the stateless encoding of their action when it fits the
//! front end's limit, and otherwise a short handle kept in a [`TokenStore`]
//! (see [`tokens`]). Hosts whose ids are long should give the panel a
//! persistent store, so that those buttons keep working after a restart.

mod command;
mod edit;
mod input;
mod press;
mod screens;
mod setting;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
mod text;
mod token;
pub mod tokens;

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use botconf::{Change, Schema, SettingsError, SettingsStore, Snapshot, StorageError};
use futures::future::BoxFuture;
use serde_json::Value;

#[cfg(feature = "sea-orm")]
pub use self::tokens::{DEFAULT_TOKENS_TABLE, SeaOrmTokenStore, create_tokens_table};
pub use self::{
    text::{Span, Style, Text},
    token::{Action, AskKind, Page, Target},
    tokens::{MemoryTokenStore, TokenStore},
};

/// Buttons whose handles are not renewed expire after this long.
pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// A message of the panel: what to show and the buttons under it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Screen {
    pub text: Text,
    pub rows: Vec<Vec<ButtonView>>,
}

/// A button of a screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonView {
    pub label: String,
    /// Opaque. The front end sends it back to [`Panel::press`] as is.
    pub token: String,
}

/// What the front end can display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The longest token a button can carry, in bytes.
    pub token_bytes: usize,
    /// The longest message, in characters (the front end cuts it).
    pub text_chars: usize,
    /// The most rows of buttons a message can have; longer screens are cut
    /// into parts, with ◀ / ▶ buttons.
    pub max_rows: Option<usize>,
    /// The most buttons in a row; longer rows are split.
    pub max_per_row: usize,
}

/// Who is looking, and what their front end can do.
pub struct Viewer<'a> {
    /// Recorded as the author of changes.
    pub actor: Option<String>,
    pub limits: &'a Limits,
    /// Names and id resolution for reference domains, if the front end has
    /// any.
    pub names: Option<&'a dyn Names>,
    /// Whether the front end can show its pickers in this conversation (e.g.
    /// Telegram's work only in private chats).
    pub pickers: bool,
}

impl<'a> Viewer<'a> {
    /// A viewer with no names and no pickers.
    pub fn new(limits: &'a Limits) -> Self {
        Self {
            actor: None,
            limits,
            names: None,
            pickers: false,
        }
    }
}

/// The name of a user, a chat, or anything else a reference points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    /// E.g. `Ann Lee (@ann)`, `Family`.
    pub full: String,
    /// E.g. `Ann`, `Family`, for buttons.
    pub short: String,
}

/// Names for references, per domain. Lookups can't fail: implementations log
/// their errors and return nothing, and the panel shows the id.
pub trait Names: Send + Sync {
    /// The name of `id` in `domain`.
    fn name<'a>(&'a self, domain: &'a str, id: &'a Value) -> BoxFuture<'a, Option<Name>>;

    /// The id `text` refers to in `domain`, besides a plain id, e.g. an
    /// `@username`.
    fn resolve<'a>(&'a self, _domain: &'a str, _text: &'a str) -> BoxFuture<'a, Option<Value>> {
        Box::pin(async { None })
    }

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

/// What a front end knows about a reference domain: how its ids are typed,
/// counted, asked for and picked.
#[derive(Clone, Copy, Debug)]
pub struct Domain {
    pub name: &'static str,
    /// How references are counted: `("user", "users")`.
    pub noun: (&'static str, &'static str),
    /// Reads a typed id, e.g. digits for Telegram; `None` = not an id (then
    /// the names may still resolve it).
    pub parse: fn(&str) -> Option<Value>,
    /// What to send for one reference, e.g. `the chat id`.
    pub one: &'static str,
    /// What to send for several, e.g. `the chat ids`.
    pub many: &'static str,
    /// An id shown in the question for one reference, e.g. `-1001234567890`.
    pub example: Option<&'static str>,
    /// What the front end's picker picks, for one and for several (e.g. `the
    /// user`, `users`), if it has one for this domain.
    pub pick: Option<(&'static str, &'static str)>,
    /// Why a word that neither parses nor resolves is not an id.
    pub unknown: fn(&str) -> String,
}

impl Domain {
    /// A domain whose ids are typed and parsed by `parse`.
    pub const fn new(name: &'static str, parse: fn(&str) -> Option<Value>) -> Self {
        Self {
            name,
            noun: ("item", "items"),
            parse,
            one: "the id",
            many: "the ids",
            example: None,
            pick: None,
            unknown: unknown_id,
        }
    }
}

fn unknown_id(word: &str) -> String {
    format!("`{word}` is not an id")
}

/// The front end's picker for a reference domain, to offer with a question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    pub domain: String,
    pub many: bool,
}

/// What the panel asked; kept by the front end until answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub target: Target,
    pub ask: AskKind,
    /// The draft the answer goes into (see [`Page::Draft`]), when the
    /// question was asked on one; `None` for the stored settings.
    pub draft: Option<Value>,
}

impl Question {
    /// A question about the stored settings.
    pub fn new(target: Target, ask: AskKind) -> Self {
        Self {
            target,
            ask,
            draft: None,
        }
    }

    /// A compact form, e.g. for a client that keeps it as text:
    /// `<ask>:<target>`, then `|<draft>` with `%` and `|` escaped.
    pub fn encode(&self) -> String {
        let mut text = format!("{}:{}", self.ask.code(), self.target.encode());
        if let Some(draft) = &self.draft {
            text.push('|');
            text.push_str(&draft.to_string().replace('%', "%25").replace('|', "%7C"));
        }
        text
    }

    pub fn parse(text: &str) -> Option<Self> {
        // Questions encoded before drafts existed have no `|<draft>`; an entry
        // name may hold a `|`, so the text is read whole when what follows
        // the last `|` is not a draft.
        let draft = text.rsplit_once('|').and_then(|(question, draft)| {
            let draft = draft.replace("%7C", "|").replace("%25", "%");
            Some((question, serde_json::from_str::<Value>(&draft).ok()?))
        });
        let (text, draft) = match draft {
            Some((question, draft)) => (question, Some(draft)),
            None => (text, None),
        };
        let (ask, target) = text.split_once(':')?;
        Some(Self {
            target: Target::parse(target)?,
            ask: AskKind::parse(ask)?,
            draft,
        })
    }
}

/// A question to put to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub question: Question,
    /// The setting's title, e.g. as an input placeholder.
    pub title: String,
    /// What to ask; the front end adds how to give up (`/cancel`, a button).
    pub text: Text,
    /// Offer the front end's picker for this domain.
    pub picker: Option<Picker>,
    /// The current value, e.g. for "tap to copy" (also in `text`).
    pub current: Option<String>,
    /// Suggested answers, as buttons (e.g. known chats to add).
    pub rows: Vec<Vec<ButtonView>>,
}

/// An answer to a question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Text(String),
    /// Ids picked with the front end's picker, already in the domain's JSON
    /// form.
    Picked {
        domain: String,
        ids: Vec<Value>,
    },
    Cancel,
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
    /// The panel was closed: delete it, or redraw it with `screen`.
    pub closed: bool,
    /// The settings change this made, for the host's hooks.
    pub change: Option<Change<S>>,
}

impl<S: Schema> Reaction<S> {
    pub(crate) fn none() -> Self {
        Self {
            screen: None,
            post: None,
            notice: None,
            ask: None,
            closed: false,
            change: None,
        }
    }

    pub(crate) fn screen(screen: Screen) -> Self {
        Self {
            screen: Some(screen),
            ..Self::none()
        }
    }
}

impl<S: Schema> std::fmt::Debug for Reaction<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reaction")
            .field("screen", &self.screen)
            .field("post", &self.post)
            .field("notice", &self.notice)
            .field("ask", &self.ask)
            .field("closed", &self.closed)
            .field("change", &self.change.is_some())
            .finish()
    }
}

/// What the text form of the command replies.
pub enum CommandReply<S: Schema> {
    /// Post the panel.
    Panel(Screen),
    /// Reply with this text.
    Text {
        text: Text,
        /// The settings change this made, for the host's hooks.
        change: Option<Change<S>>,
    },
}

impl<S: Schema> std::fmt::Debug for CommandReply<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Panel(screen) => f.debug_tuple("Panel").field(screen).finish(),
            Self::Text { text, change } => f
                .debug_struct("Text")
                .field("text", text)
                .field("change", &change.is_some())
                .finish(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PanelError {
    /// The press or answer made no sense (a malformed token, a bad value):
    /// show it to the user and keep the panel.
    #[error("{0}")]
    Rejected(String),
    /// The button was out of date: show `notice` and redraw the panel
    /// message with `screen`.
    #[error("{notice}")]
    Stale { notice: String, screen: Screen },
    /// The settings storage failed: an infrastructure error for the host.
    #[error(transparent)]
    Storage(SettingsError),
    /// The token store failed: an infrastructure error for the host.
    #[error("the panel's token store failed")]
    Tokens(#[source] StorageError),
}

/// A short note about a section, e.g. `off` for a disabled feature.
pub type SectionNote<S> = Arc<dyn Fn(&Snapshot<S>, &str) -> Option<String> + Send + Sync>;

/// The current time, in milliseconds since the Unix epoch.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The engine over a settings store.
pub struct Panel<S: Schema> {
    store: Arc<SettingsStore<S>>,
    tokens: tokens::Tokens,
    domains: Vec<Domain>,
    command: String,
    section_note: Option<SectionNote<S>>,
    stored_label: String,
    clock: Clock,
}

impl<S: Schema> Panel<S> {
    /// Uses a [`MemoryTokenStore`] and a 30-day token TTL.
    pub fn new(store: Arc<SettingsStore<S>>) -> Self {
        Self {
            store,
            tokens: tokens::Tokens::new(
                Box::new(MemoryTokenStore::new()),
                ttl_ms(DEFAULT_TOKEN_TTL),
            ),
            domains: Vec::new(),
            command: "config".into(),
            section_note: None,
            stored_label: "changed at runtime ✏️".into(),
            clock: Arc::new(system_now),
        }
    }

    /// Where the handles of long buttons are kept.
    #[must_use]
    pub fn token_store(mut self, store: impl TokenStore) -> Self {
        self.tokens = tokens::Tokens::new(Box::new(store), self.tokens.ttl_ms);
        self
    }

    /// How long a handle lives without being drawn again.
    #[must_use]
    pub fn token_ttl(mut self, ttl: Duration) -> Self {
        self.tokens.ttl_ms = ttl_ms(ttl);
        self
    }

    /// The time source, in milliseconds since the Unix epoch (default: the
    /// system clock), e.g. for tests.
    #[must_use]
    pub fn clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// What the front end knows about a reference domain.
    #[must_use]
    pub fn domain(mut self, domain: Domain) -> Self {
        self.domains.retain(|known| known.name != domain.name);
        self.domains.push(domain);
        self
    }

    /// A note shown next to a section, e.g. `off`.
    #[must_use]
    pub fn section_note(
        mut self,
        note: impl Fn(&Snapshot<S>, &str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.section_note = Some(Arc::new(note));
        self
    }

    /// The command opening the panel, without the slash (default `config`),
    /// for the panel's texts.
    #[must_use]
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = command.into();
        self
    }

    /// How a setting changed from the panel is described (default `changed at
    /// runtime ✏️`), e.g. `changed from Telegram ✏️`.
    #[must_use]
    pub fn stored_label(mut self, label: impl Into<String>) -> Self {
        self.stored_label = label.into();
        self
    }

    pub fn store(&self) -> &Arc<SettingsStore<S>> {
        &self.store
    }

    /// The command opening the panel, without the slash.
    pub fn command_name(&self) -> &str {
        &self.command
    }

    /// Deletes expired handles now (also done at most hourly on its own).
    pub async fn purge_tokens(&self) -> Result<u64, PanelError> {
        self.tokens
            .purge(self.now())
            .await
            .map_err(PanelError::Tokens)
    }

    /// A stateless token that posts `page` as a new message, for other
    /// panels' buttons; `None` if it does not fit.
    pub fn post_token(&self, page: &Page, limits: &Limits) -> Option<String> {
        post_token(page, limits)
    }

    /// The same, falling back to a stored handle when it does not fit.
    pub async fn post_token_stored(
        &self,
        page: &Page,
        limits: &Limits,
    ) -> Result<String, PanelError> {
        self.tokens
            .issue(&Action::Post(page.clone()), limits.token_bytes, self.now())
            .await
            .map_err(PanelError::Tokens)?
            .ok_or_else(|| PanelError::Rejected("the page's button does not fit".into()))
    }

    pub(crate) fn now(&self) -> i64 {
        (self.clock)()
    }

    pub(crate) fn stored_text(&self) -> &str {
        &self.stored_label
    }

    pub(crate) fn domain_of(&self, name: &str) -> Option<&Domain> {
        self.domains.iter().find(|domain| domain.name == name)
    }

    pub(crate) fn note(&self, snapshot: &Snapshot<S>, id: &str) -> Option<String> {
        self.section_note.as_ref()?(snapshot, id)
    }
}

/// A stateless token that posts `page`, for panels built without access to
/// the [`Panel`]; `None` if it does not fit.
pub fn post_token(page: &Page, limits: &Limits) -> Option<String> {
    let token = Action::Post(page.clone()).encode();
    (token.len() <= limits.token_bytes).then_some(token)
}

fn ttl_ms(ttl: Duration) -> i64 {
    i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX)
}

fn system_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}
