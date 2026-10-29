//! The reference domains the Telegram panel knows: Telegram's own users and
//! chats, picked with Telegram's pickers and stored as numbers.
//!
//! Settings declare them with [`Kind::Ref`](botconf::Kind::Ref), e.g.
//! `Kind::reference(domains::USER, true)`. Ids of any other domain are typed
//! in and named through [`Names`](crate::Names).

use botconf_panel::Domain;
use serde_json::Value;

/// A Telegram user id.
pub const USER: &str = "telegram.user";

/// A Telegram chat id: a group, a channel, or a user's private chat.
pub const CHAT: &str = "telegram.chat";

/// How the panel asks for, reads and counts users.
pub(crate) const USERS: Domain = Domain {
    noun: ("user", "users"),
    one: "the id or @username (of someone who has talked to me)",
    many: "the ids or @usernames (of people who have talked to me)",
    pick: Some(("the user", "users")),
    unknown: unknown_user,
    ..Domain::new(USER, user_id)
};

/// How the panel asks for, reads and counts chats.
pub(crate) const CHATS: Domain = Domain {
    noun: ("chat", "chats"),
    one: "the chat id",
    many: "the chat ids",
    example: Some("-1001234567890"),
    pick: Some(("the chat", "a group or a channel")),
    unknown: unknown_chat,
    ..Domain::new(CHAT, chat_id)
};

fn user_id(word: &str) -> Option<Value> {
    word.parse::<u64>().ok().map(Value::from)
}

fn chat_id(word: &str) -> Option<Value> {
    word.parse::<i64>().ok().map(Value::from)
}

fn unknown_user(word: &str) -> String {
    format!("I don't know {word}: they need to talk to me first, or pick them with the button")
}

fn unknown_chat(word: &str) -> String {
    format!("`{word}` is not a chat id, such as -1001234567890")
}
