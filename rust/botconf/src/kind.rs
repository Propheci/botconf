//! What the value of a runtime setting looks like, so that front ends (e.g. a
//! chat settings panel) can offer a fitting editor: toggles, pickers, a list
//! of references (users, chats, ...) with pickers for their domain, a form,
//! ...
//!
//! The kind is only a hint for editing: every value is still validated by
//! deserializing the whole configuration.
//!
//! Descriptors are [`Shared`]: `const` declarations use the `const fn`
//! constructors (`Kind::one_of`, `Choices::fixed`, `Form::new`, ...), and
//! descriptors built at runtime (e.g. a connector's settings) hold owned
//! data.

use std::fmt;

use serde_json::Value;

use crate::{View, shared::Shared};

/// The shape of a runtime setting's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Any value, typed as JSON (or plain text). The fallback.
    Json,
    /// Text, taken as typed.
    Text {
        /// Whether it can be cleared (set to `null`).
        optional: bool,
    },
    /// A whole number between `min` and `max`.
    Number {
        min: i64,
        max: i64,
        /// Shown after the number, e.g. `%`.
        unit: Shared<str>,
        /// The values offered as buttons; others can be typed in.
        suggestions: Shared<[i64]>,
        /// Whether it can be cleared (set to `null`).
        optional: bool,
    },
    /// One of `choices`.
    OneOf {
        choices: Choices,
        /// Whether other values can be typed in.
        custom: bool,
        /// Whether it can be cleared (set to `null`).
        optional: bool,
    },
    /// A list of some of `choices`, toggled on and off.
    SetOf {
        choices: Choices,
        /// Whether being listed means "off" (e.g. a list of disabled
        /// features).
        inverted: bool,
    },
    /// References to things outside the configuration: users, chats, accounts.
    /// `domain` says what they refer to (e.g. `telegram.user`, `wuphf.chat`),
    /// so front ends know how to pick them and how to name them; botconf
    /// itself never interprets the ids.
    Ref {
        domain: Shared<str>,
        /// A list of references, rather than one.
        many: bool,
    },
    /// An object whose fields are edited one by one.
    Form(Shared<Form>),
    /// A map of named entries, set one by one (see
    /// [`RuntimeSetting::per_entry`](crate::RuntimeSetting::per_entry)).
    Map {
        /// The names entries may have, if limited.
        names: Option<Choices>,
        value: Shared<Kind>,
    },
}

impl Kind {
    pub const fn text(optional: bool) -> Self {
        Self::Text { optional }
    }

    pub const fn number(
        min: i64,
        max: i64,
        unit: &'static str,
        suggestions: &'static [i64],
        optional: bool,
    ) -> Self {
        Self::Number {
            min,
            max,
            unit: Shared::Static(unit),
            suggestions: Shared::Static(suggestions),
            optional,
        }
    }

    pub const fn one_of(choices: Choices, custom: bool, optional: bool) -> Self {
        Self::OneOf {
            choices,
            custom,
            optional,
        }
    }

    pub const fn set_of(choices: Choices, inverted: bool) -> Self {
        Self::SetOf { choices, inverted }
    }

    pub const fn reference(domain: &'static str, many: bool) -> Self {
        Self::Ref {
            domain: Shared::Static(domain),
            many,
        }
    }

    pub const fn form(form: &'static Form) -> Self {
        Self::Form(Shared::Static(form))
    }

    pub const fn map(names: Option<Choices>, value: &'static Kind) -> Self {
        Self::Map {
            names,
            value: Shared::Static(value),
        }
    }

    /// The value of a new entry of a map of this kind, when there is an
    /// obvious one (an empty list, a form's initial value).
    pub fn empty_value(&self) -> Option<Value> {
        match self {
            Self::SetOf { .. } | Self::Ref { many: true, .. } => Some(Value::Array(Vec::new())),
            Self::Form(form) => serde_json::from_str(&form.initial).ok(),
            _ => None,
        }
    }

    /// The choices offered as buttons, in their order.
    pub fn choices(&self, view: &dyn View) -> Vec<Choice> {
        match self {
            Self::OneOf { choices, .. } | Self::SetOf { choices, .. } => choices.resolve(view),
            Self::Map {
                names: Some(names), ..
            } => names.resolve(view),
            Self::Number {
                unit, suggestions, ..
            } => suggestions
                .iter()
                .map(|number| Choice::new(number.to_string(), format!("{number}{unit}")))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The value of the choice `value`, as stored.
    pub fn choice_value(&self, value: &str) -> Value {
        match self {
            Self::Number { .. } => value.parse::<i64>().map_or(Value::Null, Value::from),
            _ => Value::String(value.to_string()),
        }
    }
}

/// An object edited field by field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub fields: Shared<[Field]>,
    /// The value of a new entry, as JSON, e.g. `{"brightness": 100}`.
    pub initial: Shared<str>,
}

impl Form {
    pub const fn new(fields: &'static [Field], initial: &'static str) -> Self {
        Self {
            fields: Shared::Static(fields),
            initial: Shared::Static(initial),
        }
    }
}

/// A field of a [`Form`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The key in the object.
    pub key: Shared<str>,
    pub title: Shared<str>,
    /// Fields should be optional (`optional: true`), so that they can be
    /// cleared.
    pub kind: Kind,
    /// Fields of the same group exclude each other: setting one clears the
    /// others (e.g. a light is either white or coloured).
    pub group: Option<Shared<str>>,
}

impl Field {
    pub const fn new(key: &'static str, title: &'static str, kind: Kind) -> Self {
        Self {
            key: Shared::Static(key),
            title: Shared::Static(title),
            kind,
            group: None,
        }
    }

    #[must_use]
    pub const fn group(self, group: &'static str) -> Self {
        // See `RuntimeSetting::titled` for why the fields are read out.
        // SAFETY: each kept field is read once, then `self` is forgotten, so
        // nothing is dropped twice.
        let key = unsafe { std::ptr::read(&self.key) };
        let title = unsafe { std::ptr::read(&self.title) };
        let kind = unsafe { std::ptr::read(&self.kind) };
        std::mem::forget(self);
        Self {
            key,
            title,
            kind,
            group: Some(Shared::Static(group)),
        }
    }
}

/// Where the possible values of a setting come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choices {
    /// A fixed set.
    Fixed(Shared<[FixedChoice]>),
    /// The names of the entries of a map-valued config key, e.g. the lights
    /// in `modules.lights.devices`.
    KeysOf(Shared<str>),
    /// Computed from the current configuration (see [`View`] for what it
    /// can read). Only compiled-in settings can use it; it never goes over
    /// the wire as is (front ends get the resolved choices).
    Dynamic(DynamicChoices),
}

impl Choices {
    pub const fn fixed(choices: &'static [FixedChoice]) -> Self {
        Self::Fixed(Shared::Static(choices))
    }

    pub const fn keys_of(key: &'static str) -> Self {
        Self::KeysOf(Shared::Static(key))
    }
}

/// A function listing choices, e.g. the scenes a light knows.
#[derive(Clone, Copy)]
pub struct DynamicChoices(pub fn(&dyn View) -> Vec<Choice>);

impl PartialEq for DynamicChoices {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}

impl Eq for DynamicChoices {}

impl fmt::Debug for DynamicChoices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DynamicChoices(..)")
    }
}

/// A fixed choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixedChoice {
    pub value: Shared<str>,
    pub label: Shared<str>,
}

impl FixedChoice {
    pub const fn new(value: &'static str, label: &'static str) -> Self {
        Self {
            value: Shared::Static(value),
            label: Shared::Static(label),
        }
    }

    /// A choice built at runtime.
    pub fn owned(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: Shared::from(value.into()),
            label: Shared::from(label.into()),
        }
    }
}

/// A choice, resolved against the current configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

impl Choice {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl Choices {
    /// The current choices, in a stable order.
    pub fn resolve(&self, view: &dyn View) -> Vec<Choice> {
        match self {
            Self::Fixed(choices) => choices
                .iter()
                .map(|choice| Choice::new(&*choice.value, &*choice.label))
                .collect(),
            Self::KeysOf(key) => match view.value(key) {
                Some(Value::Object(entries)) => entries
                    .keys()
                    .map(|name| Choice::new(name.clone(), name.clone()))
                    .collect(),
                _ => Vec::new(),
            },
            Self::Dynamic(DynamicChoices(list)) => list(view),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuntimeSetting;

    #[test]
    fn reference_lists_start_empty_and_single_references_have_no_default() {
        let many = Kind::reference("test.user", true);
        let one = Kind::reference("test.user", false);
        assert_eq!(many.empty_value(), Some(Value::Array(Vec::new())));
        assert_eq!(one.empty_value(), None);
    }

    const LEVELS: &[FixedChoice] = &[FixedChoice::new("info", "Info")];
    const WHITE: Kind = Kind::one_of(Choices::fixed(LEVELS), true, false);
    const FIELDS: &[Field] = &[Field::new("level", "Level", WHITE).group("look")];
    // Nested references to descriptors with drop glue are not promoted to
    // `'static`, so a form used by a map value gets its own `const`.
    const FORM: &Form = &Form::new(FIELDS, "{}");
    const LOOK: &Kind = &Kind::form(FORM);
    const SETTING: RuntimeSetting = RuntimeSetting::new("level", "How much to log")
        .titled("Logging")
        .kind(Kind::one_of(Choices::fixed(LEVELS), true, false));
    const ENTRIES: RuntimeSetting = RuntimeSetting::per_entry("looks", "Named looks", LOOK)
        .entry_names(Choices::keys_of("devices"));

    #[test]
    fn const_declarations_still_work() {
        let owned_choices = Choices::Fixed(vec![FixedChoice::owned("info", "Info")].into());
        let owned = RuntimeSetting {
            key: "level".to_string().into(),
            title: Some("Logging".to_string().into()),
            description: "How much to log".to_string().into(),
            per_entry: false,
            kind: Kind::OneOf {
                choices: owned_choices.clone(),
                custom: true,
                optional: false,
            },
        };
        assert_eq!(SETTING, owned);

        let owned_form = Form {
            fields: vec![Field {
                key: "level".to_string().into(),
                title: "Level".to_string().into(),
                kind: Kind::OneOf {
                    choices: owned_choices,
                    custom: true,
                    optional: false,
                },
                group: Some("look".to_string().into()),
            }]
            .into(),
            initial: "{}".to_string().into(),
        };
        assert_eq!(*FORM, owned_form);
        assert_eq!(
            ENTRIES.kind,
            Kind::Map {
                names: Some(Choices::KeysOf("devices".to_string().into())),
                value: Shared::owned(Kind::Form(Shared::owned(owned_form))),
            }
        );
    }
}
