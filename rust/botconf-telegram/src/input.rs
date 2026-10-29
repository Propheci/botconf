//! Questions as Telegram messages, with Telegram's user and chat pickers, and
//! answers read back from the messages that answer them.

use botconf_panel::{Ask, Input};
use serde_json::Value;
use teloxide::{
    types::{
        ButtonRequest, KeyboardButton, KeyboardButtonRequestChat, KeyboardButtonRequestUsers,
        KeyboardMarkup, Message, ReplyMarkup, RequestId,
    },
    utils::html::escape,
};

use crate::{
    domains::{CHAT, USER},
    prompts::{self, CANCEL},
    render,
};

/// The text and the keyboard of a question, and whether the keyboard is a
/// reply keyboard (Telegram's pickers, which must be removed afterwards).
pub(crate) fn question(prefix: &str, ask: &Ask) -> (String, ReplyMarkup, bool) {
    let text = render::html(&ask.text);
    let pickers = ask
        .picker
        .as_ref()
        .and_then(|picker| match picker.domain.as_str() {
            USER => Some(user_pickers(if picker.many { 10 } else { 1 })),
            CHAT => Some(chat_pickers()),
            _ => None,
        });

    match pickers {
        Some(keyboard) => (
            format!("{text}\n\n{}", escape("Tap Cancel to stop.")),
            ReplyMarkup::Keyboard(keyboard),
            true,
        ),
        // Suggested answers are buttons; the answer can still be typed.
        None if !ask.rows.is_empty() => (
            format!("{text}\n\n{}", escape("Send /cancel to stop.")),
            ReplyMarkup::InlineKeyboard(render::keyboard(prefix, &ask.rows)),
            false,
        ),
        None => (
            format!("{text}\n\n{}", escape("Send /cancel to stop.")),
            prompts::force_reply(ask.title.clone()),
            false,
        ),
    }
}

/// Telegram's user picker, letting up to `max` users be picked.
fn user_pickers(max: u8) -> KeyboardMarkup {
    let users = KeyboardButtonRequestUsers::new(RequestId(1))
        .max_quantity(max)
        .request_name()
        .request_username();
    KeyboardMarkup::new([
        vec![KeyboardButton::new("👤 Choose users").request(ButtonRequest::RequestUsers(users))],
        vec![KeyboardButton::new(CANCEL)],
    ])
    .resize_keyboard()
    .one_time_keyboard()
}

fn chat_pickers() -> KeyboardMarkup {
    let chat = |id, channel| {
        ButtonRequest::RequestChat(
            KeyboardButtonRequestChat::new(RequestId(id), channel)
                .request_title()
                .request_username(),
        )
    };
    KeyboardMarkup::new([
        vec![
            KeyboardButton::new("👥 Choose a group").request(chat(2, false)),
            KeyboardButton::new("📢 Choose a channel").request(chat(3, true)),
        ],
        vec![KeyboardButton::new(CANCEL)],
    ])
    .resize_keyboard()
    .one_time_keyboard()
}

/// What a message answering a question says: typed text, or the users or
/// the chat picked with Telegram's pickers.
pub(crate) fn input(message: &Message) -> Input {
    if message.text().is_some_and(prompts::is_cancel) {
        return Input::Cancel;
    }
    if let Some(shared) = message.shared_users() {
        return Input::Picked {
            domain: USER.into(),
            ids: shared
                .users
                .iter()
                .map(|user| Value::from(user.user_id.0))
                .collect(),
        };
    }
    if let Some(shared) = message.shared_chat() {
        return Input::Picked {
            domain: CHAT.into(),
            ids: vec![Value::from(shared.chat_id.0)],
        };
    }
    Input::Text(message.text().unwrap_or_default().to_string())
}

#[cfg(test)]
mod tests {
    use botconf_panel::{AskKind, Picker, Question, Target, Text};
    use serde_json::json;

    use super::*;
    use crate::prompts::tests::{message, text};

    fn ask(picker: Option<Picker>) -> Ask {
        Ask {
            question: Question {
                target: Target::setting(1),
                ask: AskKind::Items,
            },
            title: "Admins".into(),
            text: Text::plain("Send the ids."),
            picker,
            current: None,
            rows: Vec::new(),
        }
    }

    #[test]
    fn picker_messages_become_picked_ids() {
        let users = message(json!({
            "users_shared": {
                "request_id": 1,
                "users": [{ "user_id": 8, "first_name": "Bo" }, { "user_id": 9 }],
            }
        }));
        assert_eq!(
            input(&users),
            Input::Picked {
                domain: USER.into(),
                ids: vec![json!(8), json!(9)]
            }
        );
        let chat = message(json!({
            "chat_shared": { "request_id": 2, "chat_id": -100, "title": "Family" }
        }));
        assert_eq!(
            input(&chat),
            Input::Picked {
                domain: CHAT.into(),
                ids: vec![json!(-100)]
            }
        );
        assert_eq!(input(&text(" 12 ")), Input::Text(" 12 ".into()));
        assert_eq!(input(&text("/cancel")), Input::Cancel);
        assert_eq!(input(&text(CANCEL)), Input::Cancel);
    }

    #[test]
    fn pickers_are_offered_for_the_telegram_domains() {
        let picker = |domain: &str, many| {
            Some(Picker {
                domain: domain.into(),
                many,
            })
        };
        for (picker, keyboard) in [
            (picker(USER, true), true),
            (picker(CHAT, false), true),
            (picker("toy.thing", true), false),
            (None, false),
        ] {
            let (text, markup, is_keyboard) = question("cfg:", &ask(picker));
            assert_eq!(is_keyboard, keyboard);
            assert_eq!(matches!(markup, ReplyMarkup::Keyboard(_)), keyboard);
            let stop = if keyboard {
                "Tap Cancel"
            } else {
                "Send /cancel"
            };
            assert!(text.contains(stop), "{text}");
        }
    }
}
