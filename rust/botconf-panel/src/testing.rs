//! A toy schema with every kind of editor, and the helpers the tests share.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use botconf::{
    Choice, Choices, DynamicChoices, Field, FixedChoice, Form, Kind, MemoryStorage, RuntimeSetting,
    Schema, Section, SectionSettings, SettingsStore, View,
};
use figment::{
    Figment,
    providers::{Format, Toml},
};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Domain, Limits, Name, Names, Panel, Viewer};

/// The reference domains of the toy, as a Telegram front end describes them.
pub const USER: &str = "telegram.user";
pub const CHAT: &str = "telegram.chat";

pub const USERS: Domain = Domain {
    noun: ("user", "users"),
    one: "the id or @username (of someone who has talked to me)",
    many: "the ids or @usernames (of people who have talked to me)",
    pick: Some(("the user", "users")),
    unknown: unknown_user,
    ..Domain::new(USER, user_id)
};

pub const CHATS: Domain = Domain {
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub greeting: Option<String>,
    #[serde(default)]
    pub admins: Vec<u64>,
    #[serde(default = "info")]
    pub level: String,
    /// The features that are turned off.
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Who may use a feature, by feature.
    #[serde(default)]
    pub access: BTreeMap<String, Vec<u64>>,
    #[serde(default)]
    pub alerts_chat: Option<i64>,
    /// References of a domain the front end does not know.
    #[serde(default)]
    pub things: Vec<Value>,
    /// Picked from a computed list.
    #[serde(default)]
    pub flavour: Option<String>,
    /// Chats of a domain whose names suggest candidates.
    #[serde(default)]
    pub rooms: Vec<String>,
    /// Features with very long names.
    #[serde(default)]
    pub notes: BTreeMap<String, String>,
    #[serde(default)]
    pub features: BTreeMap<String, Value>,
    /// Routes, each either an `a` (with `x`) or a `b` (with `y`); see
    /// `derive`.
    #[serde(default)]
    pub routes: BTreeMap<String, Value>,
}

fn info() -> String {
    "info".into()
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lamp {
    pub presets: BTreeMap<String, Preset>,
    pub notes: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

pub struct Toy;

const FEATURES: &[FixedChoice] = &[
    FixedChoice::new("lamp", "Lamp"),
    FixedChoice::new("fan", "Fan"),
];

const LEVELS: &[FixedChoice] = &[
    FixedChoice::new("info", "info"),
    FixedChoice::new("debug", "debug (verbose)"),
];

const WHITES: &[FixedChoice] = &[FixedChoice::new("warm", "warm")];

// Nested references to descriptors are not promoted to `'static` (they have
// drop glue), so each referenced part gets its own `const`.
const PRESET_FIELDS: &[Field] = &[
    Field::new(
        "brightness",
        "Brightness",
        Kind::number(1, 100, "%", &[10, 50, 100], true),
    ),
    Field::new(
        "white",
        "White",
        Kind::one_of(Choices::fixed(WHITES), true, true),
    )
    .group("look"),
    Field::new(
        "color",
        "Colour",
        Kind::one_of(Choices::Dynamic(DynamicChoices(colors)), true, true),
    )
    .group("look"),
];

const PRESET_FORM: &Form = &Form::new(PRESET_FIELDS, r#"{"brightness": 100}"#);
const PRESET: &Kind = &Kind::form(PRESET_FORM);
const ADMINS: &Kind = &Kind::reference(USER, true);
const NOTE: &Kind = &Kind::text(false);

const ROUTE_KINDS: &[FixedChoice] = &[FixedChoice::new("a", "A"), FixedChoice::new("b", "B")];

/// A route needs `x` when it is an `a`, and `y` when it is a `b`: no single
/// field edit turns one into the other, and a new route is incomplete.
const ROUTE_FIELDS: &[Field] = &[
    Field::new(
        "kind",
        "Kind",
        Kind::one_of(Choices::fixed(ROUTE_KINDS), false, false),
    )
    .required(),
    Field::new("x", "X", Kind::text(true))
        .required()
        .when("kind", "a"),
    Field::new("y", "Y", Kind::text(true))
        .required()
        .when("kind", "b"),
];
const ROUTE_FORM: &Form = &Form::new(ROUTE_FIELDS, r#"{"kind": "a"}"#);
const ROUTE: &Kind = &Kind::form(ROUTE_FORM);

fn colors(_: &dyn View) -> Vec<Choice> {
    vec![Choice::new("red", "🔴 red"), Choice::new("blue", "🔵 blue")]
}

/// The flavours on offer, in an order the tests can change.
pub static FLAVOURS: Mutex<Vec<&str>> = Mutex::new(Vec::new());

fn flavours(_: &dyn View) -> Vec<Choice> {
    FLAVOURS
        .lock()
        .unwrap()
        .iter()
        .map(|flavour| Choice::new(*flavour, *flavour))
        .collect()
}

const LAMP_SETTINGS: &[RuntimeSetting] = &[
    RuntimeSetting::per_entry("presets", "Named looks", PRESET),
    RuntimeSetting::per_entry("notes", "Notes, e.g. {\"door\": \"closed\"}", &Kind::Json),
];

impl Schema for Toy {
    type Config = Config;
    type Derived = ();

    fn derive(&self, config: &Config) -> Result<(), String> {
        for (name, route) in &config.routes {
            let field = |key: &str| route.get(key).and_then(Value::as_str);
            let valid = match field("kind") {
                Some("a") => field("x").is_some() && route.get("y").is_none(),
                Some("b") => field("y").is_some() && route.get("x").is_none(),
                _ => false,
            };
            if !valid {
                return Err(format!("route `{name}` needs x as an a, or y as a b"));
            }
        }
        Ok(())
    }

    fn settings(&self) -> Vec<RuntimeSetting> {
        vec![
            RuntimeSetting::new("disabled", "Features that are off")
                .titled("Features")
                .kind(Kind::set_of(Choices::fixed(FEATURES), true)),
            RuntimeSetting::new("greeting", "What to say").kind(Kind::text(true)),
            RuntimeSetting::new("admins", "Who's in charge").kind(Kind::reference(USER, true)),
            RuntimeSetting::new("level", "How much to log").kind(Kind::one_of(
                Choices::fixed(LEVELS),
                true,
                false,
            )),
            RuntimeSetting::per_entry("access", "Who may use a feature", ADMINS)
                .entry_names(Choices::fixed(FEATURES)),
            RuntimeSetting::new("alerts_chat", "Where alerts go")
                .titled("Alerts chat")
                .kind(Kind::reference(CHAT, false)),
            RuntimeSetting::new("things", "Things from elsewhere")
                .kind(Kind::reference("toy.thing", true)),
            RuntimeSetting::new("flavour", "Today's flavour").kind(Kind::one_of(
                Choices::Dynamic(DynamicChoices(flavours)),
                false,
                true,
            )),
            RuntimeSetting::new("rooms", "Rooms to watch").kind(Kind::reference("toy.room", true)),
            RuntimeSetting::per_entry("notes", "Notes by topic", NOTE),
            RuntimeSetting::per_entry("routes", "Where things go", ROUTE),
        ]
    }

    fn dynamic_namespaces(&self) -> Vec<&'static str> {
        vec!["connectors"]
    }

    fn sections(&self) -> Vec<Section> {
        vec![Section::new(
            "lamp",
            "features.lamp",
            "Lamp",
            "The lamp",
            SectionSettings::of::<Lamp>(LAMP_SETTINGS),
        )]
    }
}

/// Names from a fixed list; rooms are suggested.
#[derive(Default)]
pub struct FakeNames;

impl Names for FakeNames {
    fn name<'a>(&'a self, domain: &'a str, id: &'a Value) -> BoxFuture<'a, Option<Name>> {
        let name = |full: &str, short: &str| Name {
            full: full.into(),
            short: short.into(),
        };
        let found = match (domain, id.as_i64(), id.as_str()) {
            (USER, Some(7), _) => Some(name("Ann (@ann)", "Ann")),
            (CHAT, Some(-100), _) => Some(name("Family", "Family")),
            ("toy.room", _, Some("r1")) => Some(name("Kitchen", "Kitchen")),
            _ => None,
        };
        Box::pin(async move { found })
    }

    fn resolve<'a>(&'a self, domain: &'a str, text: &'a str) -> BoxFuture<'a, Option<Value>> {
        Box::pin(async move { (domain == USER && text == "@ann").then(|| Value::from(7)) })
    }

    fn candidates<'a>(
        &'a self,
        domain: &'a str,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Vec<(Value, Name)>> {
        let rooms = [("r1", "Kitchen"), ("r2", "Garden"), ("r3", "Garage")];
        let found = if domain == "toy.room" {
            rooms
                .iter()
                .filter(|(_, name)| name.to_lowercase().contains(&query.to_lowercase()))
                .take(limit)
                .map(|(id, name)| {
                    (
                        Value::from(*id),
                        Name {
                            full: (*name).into(),
                            short: (*name).into(),
                        },
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        Box::pin(async move { found })
    }
}

pub const BASE: &str = r#"
level = "info"
"#;

/// Telegram's limits.
pub const LIMITS: Limits = Limits {
    token_bytes: 60,
    text_chars: 4096,
    max_rows: None,
    max_per_row: 8,
};

/// A panel over the toy schema, with the Telegram domains and a section
/// note.
pub async fn panel() -> Panel<Toy> {
    let base = Figment::from(Toml::string(BASE));
    let store = SettingsStore::load(Toy, base, MemoryStorage::new())
        .await
        .expect("valid settings");
    Panel::new(Arc::new(store))
        .domain(USERS)
        .domain(CHATS)
        .stored_label("changed from Telegram ✏️")
        .section_note(|snapshot, id| {
            snapshot
                .config
                .disabled
                .iter()
                .any(|disabled| disabled == id)
                .then(|| "off".to_string())
        })
}

/// A viewer with the fake names and Telegram's limits.
pub fn viewer<'a>(names: &'a FakeNames, limits: &'a Limits) -> Viewer<'a> {
    Viewer {
        actor: Some("1".into()),
        limits,
        names: Some(names),
        pickers: true,
    }
}
