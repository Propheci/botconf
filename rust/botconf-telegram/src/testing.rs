//! A toy schema with every kind of editor, for the tests.

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
use teloxide::{
    Bot,
    types::{SharedUser, UserId},
};

use crate::{Name, Names, SettingsPanel, domains};

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
    /// References of a domain the Telegram panel does not know.
    #[serde(default)]
    pub things: Vec<Value>,
    #[serde(default)]
    pub features: BTreeMap<String, Value>,
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
const ADMINS: &Kind = &Kind::reference(domains::USER, true);

fn colors(_: &dyn View) -> Vec<Choice> {
    vec![Choice::new("red", "🔴 red"), Choice::new("blue", "🔵 blue")]
}

const LAMP_SETTINGS: &[RuntimeSetting] = &[
    RuntimeSetting::per_entry("presets", "Named looks", PRESET),
    RuntimeSetting::per_entry("notes", "Notes, e.g. {\"door\": \"closed\"}", &Kind::Json),
];

impl Schema for Toy {
    type Config = Config;
    type Derived = ();

    fn derive(&self, _config: &Config) -> Result<(), String> {
        Ok(())
    }

    fn settings(&self) -> Vec<RuntimeSetting> {
        vec![
            RuntimeSetting::new("disabled", "Features that are off")
                .titled("Features")
                .kind(Kind::set_of(Choices::fixed(FEATURES), true)),
            RuntimeSetting::new("greeting", "What to say").kind(Kind::text(true)),
            RuntimeSetting::new("admins", "Who's in charge")
                .kind(Kind::reference(domains::USER, true)),
            RuntimeSetting::new("level", "How much to log").kind(Kind::one_of(
                Choices::fixed(LEVELS),
                true,
                false,
            )),
            RuntimeSetting::per_entry("access", "Who may use a feature", ADMINS)
                .entry_names(Choices::fixed(FEATURES)),
            RuntimeSetting::new("alerts_chat", "Where alerts go")
                .titled("Alerts chat")
                .kind(Kind::reference(domains::CHAT, false)),
            RuntimeSetting::new("things", "Things from elsewhere")
                .kind(Kind::reference("toy.thing", true)),
        ]
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

/// Names from a fixed list, remembering the picked users.
#[derive(Default)]
pub struct FakeNames {
    pub remembered: Arc<Mutex<Vec<UserId>>>,
}

impl Names<Bot> for FakeNames {
    fn name<'a>(
        &'a self,
        _bot: &'a Bot,
        domain: &'a str,
        id: &'a Value,
    ) -> BoxFuture<'a, Option<Name>> {
        let name = |full: &str, short: &str| Name {
            full: full.into(),
            short: short.into(),
        };
        let found = match (domain, id.as_i64()) {
            (domains::USER, Some(7)) => Some(name("Ann (@ann)", "Ann")),
            (domains::CHAT, Some(-100)) => Some(name("Family", "Family")),
            _ => None,
        };
        Box::pin(async move { found })
    }

    fn resolve<'a>(&'a self, domain: &'a str, text: &'a str) -> BoxFuture<'a, Option<Value>> {
        Box::pin(async move { (domain == domains::USER && text == "@ann").then(|| Value::from(7)) })
    }

    fn remember_users<'a>(&'a self, users: &'a [SharedUser]) -> BoxFuture<'a, ()> {
        let ids = users.iter().map(|user| user.user_id);
        self.remembered.lock().unwrap().extend(ids);
        Box::pin(async {})
    }
}

pub const BASE: &str = r#"
level = "info"
"#;

/// A panel over the toy schema, and a bot (that never gets to Telegram).
pub async fn panel() -> (Arc<SettingsPanel<Toy, Bot>>, Bot) {
    let base = Figment::from(Toml::string(BASE));
    let store = SettingsStore::load(Toy, base, MemoryStorage::new())
        .await
        .expect("valid settings");
    let panel = SettingsPanel::new(Arc::new(store), Arc::default())
        .names(FakeNames::default())
        .section_note(|snapshot, id| {
            snapshot
                .config
                .disabled
                .iter()
                .any(|disabled| disabled == id)
                .then(|| "off".to_string())
        });
    (Arc::new(panel), Bot::new("1:test"))
}
