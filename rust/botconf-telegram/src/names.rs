//! Names for references, so that the panel shows `Ann (@ann)` rather than
//! `123456789`. The bot provides them, e.g. from the users it has seen.

use futures::future::BoxFuture;
use serde_json::Value;
use teloxide::types::{ChatShared, SharedUser};

/// The name of a user, a chat, or anything else a reference points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    /// E.g. `Ann Lee (@ann)`, `Family`.
    pub full: String,
    /// E.g. `Ann`, `Family`, for buttons.
    pub short: String,
}

/// Where the panel gets names from, per reference domain (see
/// [`domains`](crate::domains)); `R` is the bot, to ask Telegram.
///
/// Lookups can't fail: implementations log their errors and return `None`,
/// in which case the panel shows the id.
pub trait Names<R>: Send + Sync + 'static {
    /// The name of `id` in `domain`, e.g. a Telegram user. `None` shows the
    /// id.
    fn name<'a>(
        &'a self,
        bot: &'a R,
        domain: &'a str,
        id: &'a Value,
    ) -> BoxFuture<'a, Option<Name>>;

    /// The id `text` refers to in `domain`, besides a plain id, e.g. an
    /// `@username` for [`USER`](crate::domains::USER) (which the Bot API
    /// cannot resolve).
    fn resolve<'a>(&'a self, _domain: &'a str, _text: &'a str) -> BoxFuture<'a, Option<Value>> {
        Box::pin(async { None })
    }

    /// Users picked with Telegram's user picker, with their names.
    fn remember_users<'a>(&'a self, _users: &'a [SharedUser]) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// A chat picked with Telegram's chat picker, with its title.
    fn remember_chat<'a>(&'a self, _chat: &'a ChatShared) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}
