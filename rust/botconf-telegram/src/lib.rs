//! A Telegram settings panel for [`botconf`] settings: the screens of the
//! [`botconf_panel`] engine drawn as messages with inline keyboards, typed
//! values asked for with a [prompt](prompts), and Telegram's user and chat
//! pickers for the [Telegram domains](domains).
//!
//! To use it:
//! 1. build a [`SettingsPanel`] over the settings store, with the hooks the bot
//!    needs ([`SettingsPanel::names`], [`SettingsPanel::on_change`], ...);
//! 2. add it to the dispatcher's dependencies (as `Arc<SettingsPanel<..>>`);
//! 3. mount [`SettingsPanel::handler`] in the handler tree, behind the bot's
//!    access checks (the panel changes settings: it is meant for the owner);
//! 4. call [`SettingsPanel::run_command`] from the bot's settings command, e.g.
//!    `/config` (no arguments open the panel, `help` lists the text
//!    subcommands).
//!
//! Prompt answers are plain messages: if the bot has other handlers for
//! plain messages, route the messages that answer a prompt (see
//! [`prompts::Prompts::waiting`]) to the panel's handler first.
//!
//! Buttons whose action does not fit Telegram's 64 bytes of callback data
//! carry a handle kept in a [`TokenStore`]: in memory by default, or in the
//! database with [`SettingsPanel::token_store`] (feature `sea-orm`:
//! `SeaOrmTokenStore`), so that they keep working after a restart.

pub mod domains;
mod handlers;
mod input;
mod names;
pub mod prompts;
mod render;
#[cfg(test)]
mod testing;

use std::{sync::Arc, time::Duration};

use botconf::{Change, Schema, SettingsStore, Snapshot};
use botconf_panel::Panel;
use futures::future::BoxFuture;
use teloxide::{
    RequestError,
    prelude::Requester,
    types::{ChatId, InlineKeyboardButton},
};

#[cfg(feature = "sea-orm")]
pub use botconf_panel::{DEFAULT_TOKENS_TABLE, SeaOrmTokenStore, create_tokens_table};
pub use botconf_panel::{MemoryTokenStore, Page, Target, TokenStore};

pub use self::{
    names::{Name, Names},
    prompts::Prompts,
};

/// The callback data prefix of the panel's buttons, unless another one is
/// set.
pub const DEFAULT_PREFIX: &str = "cfg:";

/// The bots the panel works with: `Bot`, `Throttle<Bot>`, ...
pub trait PanelBot: Requester<Err = RequestError> + Clone + Send + Sync + 'static {}

impl<R> PanelBot for R where R: Requester<Err = RequestError> + Clone + Send + Sync + 'static {}

/// Called after each change, e.g. to refresh the command menus.
pub type OnChange<S, R> = Arc<dyn Fn(R, Arc<Change<S>>) -> BoxFuture<'static, ()> + Send + Sync>;

/// The panel: its settings, its hooks and how it is reached.
pub struct SettingsPanel<S: Schema, R> {
    panel: Panel<S>,
    prompts: Arc<Prompts>,
    names: Option<Arc<dyn Names<R>>>,
    on_change: Option<OnChange<S, R>>,
    prefix: String,
    owner: &'static str,
}

impl<S: Schema, R: PanelBot> SettingsPanel<S, R> {
    /// A panel over `store`, asking questions with `prompts` (share them with
    /// the rest of the bot, so that there is one question per user and chat).
    pub fn new(store: Arc<SettingsStore<S>>, prompts: Arc<Prompts>) -> Self {
        let panel = Panel::new(store)
            .domain(domains::USERS)
            .domain(domains::CHATS)
            .stored_label("changed from Telegram ✏️");
        Self {
            panel,
            prompts,
            names: None,
            on_change: None,
            prefix: DEFAULT_PREFIX.into(),
            owner: "settings",
        }
    }

    /// Where to get names for user and chat ids (default: ids only).
    #[must_use]
    pub fn names(mut self, names: impl Names<R>) -> Self {
        self.names = Some(Arc::new(names));
        self
    }

    /// Called after each change.
    #[must_use]
    pub fn on_change(
        mut self,
        on_change: impl Fn(R, Arc<Change<S>>) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// A note shown next to a section, e.g. `off`.
    #[must_use]
    pub fn section_note(
        mut self,
        note: impl Fn(&Snapshot<S>, &str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.panel = self.panel.section_note(note);
        self
    }

    /// The bot command opening the panel, without the slash (default
    /// `config`), for the panel's messages.
    #[must_use]
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.panel = self.panel.command(command);
        self
    }

    /// The prefix of the buttons' callback data (default [`DEFAULT_PREFIX`]).
    #[must_use]
    pub fn callback_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Who asks the panel's questions, in [`Prompts`] (default `settings`).
    #[must_use]
    pub fn prompt_owner(mut self, owner: &'static str) -> Self {
        self.owner = owner;
        self
    }

    /// Where the handles of buttons too long for Telegram are kept (default:
    /// in memory, until the bot stops).
    #[must_use]
    pub fn token_store(mut self, store: impl TokenStore) -> Self {
        self.panel = self.panel.token_store(store);
        self
    }

    /// How long a handle lives without being drawn again (default 30 days).
    #[must_use]
    pub fn token_ttl(mut self, ttl: Duration) -> Self {
        self.panel = self.panel.token_ttl(ttl);
        self
    }

    pub fn store(&self) -> &Arc<SettingsStore<S>> {
        self.panel.store()
    }

    /// The engine, e.g. to purge its handles on a timer.
    pub fn engine(&self) -> &Panel<S> {
        &self.panel
    }

    /// Posts `page` as a new message in `chat`.
    pub async fn open(&self, bot: &R, chat: ChatId, page: &Page) -> anyhow::Result<()> {
        handlers::post(self, bot, chat, page).await
    }

    /// A button posting `page` as a new message, e.g. from another feature's
    /// own panel.
    pub fn button(&self, label: &str, page: Page) -> Option<InlineKeyboardButton> {
        post_button(&self.prefix, label, page)
    }
}

/// A button posting `page` of the panel whose buttons have `prefix`, for
/// panels built without access to the [`SettingsPanel`]. `None` if it does
/// not fit Telegram's callback data (section ids are short, so it does).
pub fn post_button(prefix: &str, label: &str, page: Page) -> Option<InlineKeyboardButton> {
    let token = botconf_panel::post_token(&page, &render::limits(prefix))?;
    Some(InlineKeyboardButton::callback(
        truncate(label, 48),
        format!("{prefix}{token}"),
    ))
}

/// Cuts `text` to `max` characters, with an ellipsis.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}

/// Who made a change, as stored.
fn actor(user: teloxide::types::UserId) -> Option<String> {
    Some(user.0.to_string())
}
