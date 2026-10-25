//! The reference domains the Telegram panel knows: Telegram's own users and
//! chats, picked with Telegram's pickers and stored as numbers.
//!
//! Settings declare them with [`Kind::Ref`](botconf::Kind::Ref), e.g.
//! `Kind::Ref { domain: domains::USER, many: true }`. Ids of any other domain
//! are typed in and named through [`Names`](crate::Names).

/// A Telegram user id.
pub const USER: &str = "telegram.user";

/// A Telegram chat id: a group, a channel, or a user's private chat.
pub const CHAT: &str = "telegram.chat";

/// How a list of references of `domain` is counted: `("user", "users")`.
pub(crate) fn nouns(domain: &str) -> (&'static str, &'static str) {
    match domain {
        USER => ("user", "users"),
        CHAT => ("chat", "chats"),
        _ => ("item", "items"),
    }
}
