//! The engine's screens as Telegram messages: HTML text and inline keyboards
//! whose callback data is the panel's prefix and the button's token.

use botconf_panel::{ButtonView, Limits, Screen, Style, Text};
use teloxide::{
    types::{InlineKeyboardButton, InlineKeyboardMarkup},
    utils::html::{bold, code_block, code_inline, escape, italic},
};

/// Telegram's limits, in characters and bytes.
pub(crate) const MAX_MESSAGE_CHARS: usize = 4096;
const MAX_CALLBACK_BYTES: usize = 64;

/// What a Telegram message can hold, for buttons whose data starts with
/// `prefix`.
pub(crate) fn limits(prefix: &str) -> Limits {
    Limits {
        token_bytes: MAX_CALLBACK_BYTES.saturating_sub(prefix.len()),
        text_chars: MAX_MESSAGE_CHARS,
        max_rows: None,
        max_per_row: 8,
    }
}

/// Text as Telegram HTML.
pub(crate) fn html(text: &Text) -> String {
    text.0
        .iter()
        .map(|span| match span.style {
            Style::Plain => escape(&span.text),
            Style::Bold => bold(&escape(&span.text)),
            Style::Italic => italic(&escape(&span.text)),
            Style::Code => code_inline(&span.text),
            Style::CodeBlock => code_block(&span.text),
        })
        .collect()
}

/// The buttons as an inline keyboard.
pub(crate) fn keyboard(prefix: &str, rows: &[Vec<ButtonView>]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(rows.iter().map(|row| {
        row.iter()
            .map(|button| {
                InlineKeyboardButton::callback(&button.label, format!("{prefix}{}", button.token))
            })
            .collect::<Vec<_>>()
    }))
}

/// A screen as a message: its HTML (cut to Telegram's limit) and keyboard.
pub(crate) fn message(prefix: &str, screen: &Screen) -> (String, InlineKeyboardMarkup) {
    (
        crate::truncate(&html(&screen.text), MAX_MESSAGE_CHARS),
        keyboard(prefix, &screen.rows),
    )
}

#[cfg(test)]
mod tests {
    use botconf_panel::Span;
    use teloxide::types::InlineKeyboardButtonKind;

    use super::*;

    #[test]
    fn spans_become_escaped_html() {
        let text = Text(vec![
            Span::plain("a < b & "),
            Span::bold("<b>"),
            Span::italic("i"),
            Span::code("x<y"),
            Span::code_block("{\"a\": 1}"),
        ]);
        assert_eq!(
            html(&text),
            "a &lt; b &amp; <b>&lt;b&gt;</b><i>i</i><code>x&lt;y</code><pre>{\"a\": 1}</pre>"
        );
    }

    #[test]
    fn callback_data_is_the_prefix_and_the_token() {
        let limits = limits("cfg:");
        assert_eq!(limits.token_bytes, 60);
        let markup = keyboard(
            "cfg:",
            &[vec![ButtonView {
                label: "Home".into(),
                token: "o:h".into(),
            }]],
        );
        let InlineKeyboardButtonKind::CallbackData(data) = &markup.inline_keyboard[0][0].kind
        else {
            panic!()
        };
        assert_eq!(data, "cfg:o:h");
    }
}
