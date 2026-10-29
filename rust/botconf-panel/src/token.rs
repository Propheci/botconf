//! The actions of the panel's buttons and their stateless encoding,
//! `<action>[:<target>[:<argument>]]`, e.g. `o:h` or `p:5:1`. Front ends add
//! their own prefix and send the token back unread.
//!
//! Settings are referred to by their position in the catalog, which never
//! changes while the program runs. Entry names and section ids are escaped
//! (`%`, `:`, `#` and `@` as `%25`, `%3A`, `%23` and `%40`), so every action
//! has an encoding; whether it fits a front end's limit is decided when the
//! button is issued (see [`crate::tokens`]).

use botconf::keys::is_entry_name;
use serde_json::Value;

/// Separates the field of a form from the rest of a target.
const FIELD_SEPARATOR: char = '#';

/// Separates a screen's part number from the page, in `Open` and `Post`.
const PART_SEPARATOR: char = '@';

/// A setting (by its position in the catalog), one entry of a map-valued
/// setting, or one field of either when it is a form.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub setting: usize,
    pub entry: Option<String>,
    /// The position of the field in the form.
    pub field: Option<usize>,
}

impl Target {
    pub fn setting(setting: usize) -> Self {
        Self {
            setting,
            entry: None,
            field: None,
        }
    }

    pub fn entry(&self, name: &str) -> Self {
        Self {
            setting: self.setting,
            entry: Some(name.to_string()),
            field: None,
        }
    }

    pub fn field(&self, field: usize) -> Self {
        Self {
            field: Some(field),
            ..self.clone()
        }
    }

    /// The form of a field, or the map of an entry.
    pub fn parent(&self) -> Option<Self> {
        match (&self.entry, self.field) {
            (_, Some(_)) => Some(Self {
                field: None,
                ..self.clone()
            }),
            (Some(_), None) => Some(Self::setting(self.setting)),
            (None, None) => None,
        }
    }

    pub(crate) fn encode(&self) -> String {
        let mut text = self.setting.to_string();
        if let Some(entry) = &self.entry {
            text.push('.');
            text.push_str(&escape(entry));
        }
        if let Some(field) = self.field {
            text.push(FIELD_SEPARATOR);
            text.push_str(&field.to_string());
        }
        text
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (text, field) = match text.split_once(FIELD_SEPARATOR) {
            Some((text, field)) => (text, Some(field.parse().ok()?)),
            None => (text, None),
        };
        let (setting, entry) = match text.split_once('.') {
            Some((setting, entry)) => (setting, Some(unescape(entry))),
            None => (text, None),
        };
        if entry.as_deref().is_some_and(|entry| !is_entry_name(entry)) {
            return None;
        }
        Some(Self {
            setting: setting.parse().ok()?,
            entry,
            field,
        })
    }
}

/// A screen of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    /// The settings of a section, by id.
    Section(String),
    Setting(Target),
    /// An unsaved value of a form (a setting or a map entry), being filled
    /// in: nothing is stored until it is saved.
    Draft(Target, Value),
}

impl Page {
    fn encode(&self) -> String {
        match self {
            Self::Home => "h".into(),
            Self::Section(id) => format!("m:{}", escape(id)),
            Self::Setting(target) => format!("s:{}", target.encode()),
            // Escaped, so that the JSON holds no separator (`:`, `@`).
            Self::Draft(target, value) => {
                format!("d:{}:{}", target.encode(), escape(&value.to_string()))
            }
        }
    }

    fn parse(code: &str, argument: Option<&str>) -> Option<Self> {
        match (code, argument) {
            ("h", None) => Some(Self::Home),
            ("m", Some(id)) if !id.is_empty() => Some(Self::Section(unescape(id))),
            ("s", Some(target)) => Some(Self::Setting(Target::parse(target)?)),
            ("d", Some(draft)) => {
                let (target, value) = draft.split_once(':')?;
                Some(Self::Draft(
                    Target::parse(target)?,
                    serde_json::from_str(&unescape(value)).ok()?,
                ))
            }
            _ => None,
        }
    }
}

/// What the user is asked to type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    /// The new value.
    Value,
    /// Items to add to a list.
    Items,
    /// A new entry of a map: its name, then its value.
    Entry,
}

impl AskKind {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Value => "v",
            Self::Items => "i",
            Self::Entry => "e",
        }
    }

    pub(crate) fn parse(code: &str) -> Option<Self> {
        match code {
            "v" => Some(Self::Value),
            "i" => Some(Self::Items),
            "e" => Some(Self::Entry),
            _ => None,
        }
    }
}

/// What a button does.
///
/// Choices of `Choices::Fixed` lists, which cannot change while the program
/// runs, are picked by position (`Toggle`, `Pick`); every other choice, and
/// every list item, is named by its value, so that a list that changed
/// between drawing a button and pressing it can't make the press act on
/// another value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Shows a page in place: its `part`, for screens cut into parts.
    Open(Page, usize),
    /// Posts a page as a new message, e.g. from another module's panel.
    Post(Page),
    /// Adds or removes the `n`th choice of a fixed set.
    Toggle(Target, usize),
    /// Picks the `n`th choice of a fixed list: the value of a setting, or
    /// the name of a new map entry.
    Pick(Target, usize),
    /// Adds or removes the choice with this value.
    ToggleValue(Target, Value),
    /// Picks the choice with this value.
    PickValue(Target, Value),
    /// Adds an item to a list (a suggested reference).
    Add(Target, Value),
    /// Removes an item from a list.
    Remove(Target, Value),
    Ask(Target, AskKind),
    /// Clears the value (`null`, or no field).
    Clear(Target),
    /// Goes back to the config file's value.
    Reset(Target),
    /// Deletes a map entry.
    Delete(Target),
    Reload,
    Close,
    /// `then`, pressed on a draft holding `value` (see [`Page::Draft`]): it
    /// changes the draft, not the stored settings.
    InDraft {
        value: Value,
        then: Box<Action>,
    },
    /// Stores the draft `value` of the form at the target.
    SaveDraft(Target, Value),
}

impl Action {
    /// Shows `page` in place, from its first part.
    pub fn open(page: Page) -> Self {
        Self::Open(page, 0)
    }

    /// The stateless encoding, without any prefix.
    pub fn encode(&self) -> String {
        let target = |code: &str, target: &Target| format!("{code}:{}", target.encode());
        let value =
            |code: &str, to: &Target, value: &Value| format!("{}:{value}", target(code, to));
        match self {
            Self::Open(page, 0) => format!("o:{}", page.encode()),
            Self::Open(page, part) => format!("o:{}{PART_SEPARATOR}{part}", page.encode()),
            Self::Post(page) => format!("n:{}", page.encode()),
            Self::Toggle(to, choice) => format!("{}:{choice}", target("t", to)),
            Self::Pick(to, choice) => format!("{}:{choice}", target("p", to)),
            Self::ToggleValue(to, item) => value("T", to, item),
            Self::PickValue(to, item) => value("P", to, item),
            Self::Add(to, item) => value("A", to, item),
            Self::Remove(to, item) => value("r", to, item),
            Self::Ask(to, ask) => format!("{}:{}", target("a", to), ask.code()),
            Self::Clear(to) => target("c", to),
            Self::Reset(to) => target("u", to),
            Self::Delete(to) => target("d", to),
            Self::Reload => "R".to_string(),
            Self::Close => "x".to_string(),
            // The draft is escaped, so that it holds no `:` and the rest is
            // the pressed action's own encoding.
            Self::InDraft { value, then } => {
                format!("D:{}:{}", escape(&value.to_string()), then.encode())
            }
            Self::SaveDraft(to, draft) => value("S", to, draft),
        }
    }

    /// Reads a stateless encoding (without the prefix).
    pub fn parse(body: &str) -> Option<Self> {
        let mut parts = body.splitn(3, ':');
        let action = parts.next()?;
        let second = parts.next();
        let argument = parts.next();
        let target = || second.and_then(Target::parse);
        let value = |argument: &str| -> Value {
            serde_json::from_str(argument).unwrap_or_else(|_| Value::String(argument.into()))
        };

        let action = match (action, argument) {
            ("o", _) => {
                let (page, part) = split_part(second?, argument)?;
                Self::Open(page, part)
            }
            ("n", _) => match split_part(second?, argument)? {
                (page, 0) => Self::Post(page),
                _ => return None,
            },
            ("t", Some(choice)) => Self::Toggle(target()?, choice.parse().ok()?),
            ("p", Some(choice)) => Self::Pick(target()?, choice.parse().ok()?),
            ("T", Some(item)) => Self::ToggleValue(target()?, serde_json::from_str(item).ok()?),
            ("P", Some(item)) => Self::PickValue(target()?, serde_json::from_str(item).ok()?),
            ("A", Some(item)) => Self::Add(target()?, serde_json::from_str(item).ok()?),
            // A non-JSON item is read as a string, as tokens drawn before
            // items were JSON carried them.
            ("r", Some(item)) => Self::Remove(target()?, value(item)),
            ("a", Some(ask)) => Self::Ask(target()?, AskKind::parse(ask)?),
            ("c", None) => Self::Clear(target()?),
            ("u", None) => Self::Reset(target()?),
            ("d", None) => Self::Delete(target()?),
            ("D", Some(then)) => Self::InDraft {
                value: serde_json::from_str(&unescape(second?)).ok()?,
                then: Box::new(Self::parse(then)?),
            },
            ("S", Some(draft)) => Self::SaveDraft(target()?, serde_json::from_str(draft).ok()?),
            ("R", None) if second.is_none() => Self::Reload,
            ("x", None) if second.is_none() => Self::Close,
            _ => return None,
        };
        Some(action)
    }
}

/// The page of an `Open` or `Post` body (its code and argument), and the
/// part number after `@`, if any.
fn split_part(code: &str, argument: Option<&str>) -> Option<(Page, usize)> {
    let (code, argument, part) = match argument {
        Some(argument) => match argument.rsplit_once(PART_SEPARATOR) {
            Some((argument, part)) => (code, Some(argument), part.parse().ok()?),
            None => (code, Some(argument), 0),
        },
        None => match code.rsplit_once(PART_SEPARATOR) {
            Some((code, part)) => (code, None, part.parse().ok()?),
            None => (code, None, 0),
        },
    };
    Some((Page::parse(code, argument)?, part))
}

/// Escapes the characters the encoding uses as separators.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '%' => escaped.push_str("%25"),
            ':' => escaped.push_str("%3A"),
            '#' => escaped.push_str("%23"),
            '@' => escaped.push_str("%40"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// Undoes [`escape`]. A `%` not followed by a known code is kept as is, so
/// that names written before escaping existed still read back.
fn unescape(text: &str) -> String {
    let mut text = text;
    let mut unescaped = String::with_capacity(text.len());
    while let Some(index) = text.find('%') {
        unescaped.push_str(&text[..index]);
        let rest = &text[index..];
        let decoded = match rest.get(..3) {
            Some("%25") => Some('%'),
            Some("%3A") => Some(':'),
            Some("%23") => Some('#'),
            Some("%40") => Some('@'),
            _ => None,
        };
        match decoded {
            Some(c) => {
                unescaped.push(c);
                text = &rest[3..];
            }
            None => {
                unescaped.push('%');
                text = &rest[1..];
            }
        }
    }
    unescaped.push_str(text);
    unescaped
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn buttons_round_trip() {
        let setting = Target::setting(3);
        let entry = setting.entry("lights");
        let field = entry.field(2);
        for action in [
            Action::open(Page::Home),
            Action::open(Page::Section("lights".into())),
            Action::open(Page::Section("connector:tg-1".into())),
            Action::Open(Page::Section("lights".into()), 2),
            Action::Open(Page::Home, 1),
            Action::open(Page::Setting(setting.clone())),
            Action::open(Page::Setting(entry.clone())),
            Action::Open(Page::Setting(field.clone()), 3),
            Action::open(Page::Setting(setting.field(0))),
            Action::Post(Page::Section("lights".into())),
            Action::Post(Page::Home),
            Action::Toggle(setting.clone(), 2),
            Action::Pick(entry.clone(), 0),
            Action::Pick(field.clone(), 4),
            Action::ToggleValue(setting.clone(), json!("lamp")),
            Action::PickValue(entry.clone(), json!("a:b@c")),
            Action::Add(setting.clone(), json!(-100123)),
            Action::Remove(entry.clone(), json!(-100123)),
            Action::Remove(setting.clone(), json!("a:b")),
            Action::Remove(setting.clone(), json!("42")),
            Action::Ask(setting.clone(), AskKind::Value),
            Action::Ask(entry.clone(), AskKind::Items),
            Action::Ask(setting.clone(), AskKind::Entry),
            Action::Ask(field.clone(), AskKind::Value),
            Action::Clear(field.clone()),
            Action::Reset(entry.clone()),
            Action::Delete(entry.clone()),
            Action::Delete(setting.entry("a:b#c@d%e")),
            Action::Reload,
            Action::Close,
            Action::open(Page::Draft(entry.clone(), json!({"kind": "a:b@c", "n": 1}))),
            Action::Open(Page::Draft(setting.clone(), json!({"x": "%3A"})), 2),
            Action::Post(Page::Draft(entry.clone(), json!({}))),
            Action::InDraft {
                value: json!({"kind": "a", "x": "1:2@3"}),
                then: Box::new(Action::Pick(field.clone(), 1)),
            },
            Action::InDraft {
                value: json!({}),
                then: Box::new(Action::open(Page::Setting(field.clone()))),
            },
            Action::SaveDraft(entry.clone(), json!({"kind": "b", "y": "a:b"})),
        ] {
            let token = action.encode();
            assert_eq!(Action::parse(&token), Some(action), "{token}");
        }
    }

    #[test]
    fn parents() {
        let entry = Target::setting(3).entry("night");
        assert_eq!(entry.field(1).parent(), Some(entry.clone()));
        assert_eq!(entry.parent(), Some(Target::setting(3)));
        assert_eq!(
            Target::setting(3).field(0).parent(),
            Some(Target::setting(3))
        );
        assert_eq!(Target::setting(3).parent(), None);
    }

    #[test]
    fn rejects_malformed_data() {
        for data in [
            "",
            "o",
            "o:s",
            "o:s:x",
            "o:h:extra",
            "o:h@x",
            "n:q",
            "n:h@1",
            "t:1",
            "t:1:x",
            "T:1:not json",
            "a:1:z",
            "c:1.a.b",
            "c:1#x",
            "R:1",
            "q:1",
        ] {
            assert_eq!(Action::parse(data), None, "{data}");
        }
    }

    #[test]
    fn earlier_tokens_still_read() {
        // As the Telegram panel encoded them before the engine existed.
        assert_eq!(
            Action::parse("r:3.lamp:-100123"),
            Some(Action::Remove(
                Target::setting(3).entry("lamp"),
                json!(-100123)
            ))
        );
        assert_eq!(
            Action::parse("r:3:a@b"),
            Some(Action::Remove(Target::setting(3), json!("a@b")))
        );
        assert_eq!(
            Action::parse("o:s:3.night#2"),
            Some(Action::open(Page::Setting(
                Target::setting(3).entry("night").field(2)
            )))
        );
        assert_eq!(unescape("50%off"), "50%off");
    }
}
