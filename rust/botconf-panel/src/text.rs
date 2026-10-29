//! Formatted text, as a list of styled spans. The engine never escapes
//! anything: each front end turns the spans into its own markup (Telegram
//! HTML, Discord markdown, ...).

/// How a span is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Plain,
    Bold,
    Italic,
    /// Inline code, e.g. a key or an id.
    Code,
    /// A block of code, e.g. a pretty-printed JSON value.
    CodeBlock,
}

/// A run of text in one style. Lines are separated by `\n` in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub style: Style,
    pub text: String,
}

impl Span {
    pub fn new(style: Style, text: impl Into<String>) -> Self {
        Self {
            style,
            text: text.into(),
        }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self::new(Style::Plain, text)
    }

    pub fn bold(text: impl Into<String>) -> Self {
        Self::new(Style::Bold, text)
    }

    pub fn italic(text: impl Into<String>) -> Self {
        Self::new(Style::Italic, text)
    }

    pub fn code(text: impl Into<String>) -> Self {
        Self::new(Style::Code, text)
    }

    pub fn code_block(text: impl Into<String>) -> Self {
        Self::new(Style::CodeBlock, text)
    }
}

/// Formatted text: spans, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text(pub Vec<Span>);

impl Text {
    pub fn new() -> Self {
        Self::default()
    }

    /// Text in one plain span.
    pub fn plain(text: impl Into<String>) -> Self {
        Self(vec![Span::plain(text)])
    }

    /// Appends a span.
    pub fn push(&mut self, span: Span) -> &mut Self {
        self.0.push(span);
        self
    }

    /// Appends plain text.
    pub fn push_plain(&mut self, text: impl Into<String>) -> &mut Self {
        self.push(Span::plain(text))
    }

    /// Appends the spans of `other`.
    pub fn append(&mut self, other: Text) -> &mut Self {
        self.0.extend(other.0);
        self
    }

    /// `notice`, a blank line, then this text.
    #[must_use]
    pub fn with_notice(self, notice: &str) -> Self {
        let mut text = Text::plain(format!("{notice}\n\n"));
        text.append(self);
        text
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|span| span.text.is_empty())
    }

    /// The text without its styles, e.g. for tests and logs.
    pub fn to_plain(&self) -> String {
        self.0.iter().map(|span| span.text.as_str()).collect()
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Self {
        Self::plain(text)
    }
}

impl From<String> for Text {
    fn from(text: String) -> Self {
        Self::plain(text)
    }
}

/// Cuts `text` to `max` characters, with an ellipsis.
pub(crate) fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}
