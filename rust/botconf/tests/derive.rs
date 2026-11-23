//! `#[derive(Describe)]` on a config type using every attribute, compared
//! with the description it should give.

use std::collections::BTreeMap;

use botconf::describe::{Description, Entry, EnvValue, Node, Table, ValueKind, Variant};
use botconf::{Describe, Kind};
use figment::{
    Figment,
    providers::{Format, Toml},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The fixture program's configuration.
#[derive(Describe, Deserialize)]
#[allow(dead_code)]
struct Config {
    /// Where to listen:
    /// an address.
    #[botconf(example = "127.0.0.1:1")]
    listen: String,
    /// The database.
    #[serde(rename = "database")]
    database_url: Option<String>,
    #[serde(default = "default_limit")]
    #[botconf(title = "Size limit", runtime, min = 1, max = 100)]
    limit: u32,
    /// How much to log.
    #[serde(default)]
    #[botconf(runtime, panel = Kind::Text { optional: true })]
    level: Level,
    /// The token's variable.
    #[botconf(env = token, hint = "APP_TOKEN")]
    token_env: String,
    #[serde(default)]
    password: Option<Password>,
    #[serde(default)]
    #[botconf(secret)]
    api_hash: String,
    #[serde(default, with = "as_text")]
    #[botconf(kind = ValueKind::Text)]
    when: u64,
    #[serde(skip)]
    #[allow(dead_code)]
    cache: Vec<u8>,
    #[serde(default)]
    #[botconf(skip)]
    marker: Option<NotDescribed>,
    #[serde(default)]
    tuning: Tuning,
    #[serde(default)]
    rules: BTreeMap<String, Rule>,
    #[serde(flatten)]
    extra: Extra,
}

fn default_limit() -> u32 {
    10
}

/// How much to log.
#[derive(Clone, Copy, Debug, Default, Describe, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Level {
    #[default]
    Info,
    VeryLoud,
    #[serde(rename = "off")]
    Silent,
    #[serde(skip)]
    #[allow(dead_code)]
    Internal,
    #[serde(other)]
    Unknown,
}

/// A secret, written as text.
#[derive(Describe, Deserialize, Serialize)]
#[serde(transparent)]
#[botconf(secret)]
struct Password(String);

#[derive(Deserialize, Serialize)]
struct NotDescribed;

/// Performance knobs.
#[derive(Default, Describe, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct Tuning {
    worker_count: u8,
    names: Vec<String>,
    /// Not serializable, so no default is shown.
    opaque: Opaque,
}

#[derive(Default, Describe, Deserialize)]
#[botconf(kind = ValueKind::Json)]
struct Opaque {}

#[derive(Describe, Deserialize, Serialize)]
struct Rule {
    #[serde(default)]
    priority: i32,
    #[serde(flatten)]
    flow: Flow,
}

#[derive(Describe, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
enum Flow {
    /// One chat to one chat.
    OneToOne {
        source: String,
    },
    #[botconf(title = "Fan in")]
    #[serde(rename_all = "UPPERCASE")]
    ManyToOne {
        only: Option<Vec<String>>,
    },
    Off,
    Wrapped(Wrapped),
}

#[derive(Describe, Deserialize, Serialize)]
struct Wrapped {
    target: String,
}

#[derive(Default, Describe, Deserialize, Serialize)]
struct Extra {
    /// Flattened in.
    #[serde(default)]
    verbose: bool,
}

mod as_text {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

fn entries() -> Vec<Entry> {
    Description::of::<Config>("Fixture", "").table.entries
}

fn entry(key: &str) -> Entry {
    entries()
        .into_iter()
        .find(|entry| entry.key == key)
        .unwrap_or_else(|| panic!("no {key}"))
}

#[test]
fn keys_follow_serde() {
    let keys: Vec<String> = entries().into_iter().map(|entry| entry.key).collect();
    assert_eq!(
        keys,
        [
            "listen",
            "database",
            "limit",
            "level",
            "token_env",
            "password",
            "api_hash",
            "when",
            "tuning",
            "rules",
            "verbose",
        ]
    );
}

#[test]
fn fields_are_described() {
    let listen = entry("listen");
    assert_eq!(listen.description, "Where to listen: an address.");
    assert_eq!(listen.title, "Listen");
    assert!(listen.required);
    assert_eq!(listen.example, Some(json!("127.0.0.1:1")));
    assert_eq!(listen.node, Node::Value(ValueKind::Text));

    let database = entry("database");
    assert!(!database.required, "an Option is never required");
    assert_eq!(database.default, None);

    let limit = entry("limit");
    assert_eq!(limit.title, "Size limit");
    assert!(limit.runtime && !limit.required);
    assert_eq!(limit.default, Some(json!(10)));
    assert_eq!(limit.node, Node::Value(ValueKind::Int { min: 1, max: 100 }));

    let level = entry("level");
    assert_eq!(
        level.node,
        Node::Value(ValueKind::OneOf(vec![
            "info".into(),
            "very_loud".into(),
            "off".into()
        ]))
    );
    assert_eq!(level.default, Some(json!("info")));
    assert_eq!(level.panel_kind, Some(Kind::Text { optional: true }));

    let token = entry("token_env");
    assert_eq!(token.node, Node::Value(ValueKind::EnvVar(EnvValue::Token)));
    assert_eq!(token.hint, Some(json!("APP_TOKEN")));

    let password = entry("password");
    assert_eq!(password.node, Node::Value(ValueKind::Secret));
    assert_eq!(
        password.description, "A secret, written as text.",
        "the type's doc"
    );
    assert_eq!(entry("api_hash").node, Node::Value(ValueKind::Secret));
    assert_eq!(entry("when").node, Node::Value(ValueKind::Text));
}

#[test]
fn a_type_default_gives_each_fields_default() {
    let Node::Table(tuning) = entry("tuning").node else {
        panic!("a table");
    };
    let by_key: BTreeMap<&str, &Entry> = tuning
        .entries
        .iter()
        .map(|entry| (entry.key.as_str(), entry))
        .collect();
    assert_eq!(by_key["worker-count"].default, Some(json!(0)));
    assert!(!by_key["worker-count"].required);
    assert_eq!(by_key["names"].node, Node::Value(ValueKind::TextList));
    assert_eq!(by_key["opaque"].default, None, "not Serialize");
    assert_eq!(by_key["opaque"].node, Node::Value(ValueKind::Json));
    assert_eq!(entry("tuning").description, "Performance knobs.");
}

#[test]
fn a_flattened_tagged_enum_makes_a_tagged_table() {
    let Node::Map(rule) = entry("rules").node else {
        panic!("a map");
    };
    let Node::Tagged {
        tag,
        common,
        variants,
    } = *rule
    else {
        panic!("tagged");
    };
    assert_eq!(tag, "kind");
    assert_eq!(
        common.entries.iter().map(|e| &*e.key).collect::<Vec<_>>(),
        ["priority"]
    );
    let source = Entry {
        required: true,
        ..Entry::new("source", Node::Value(ValueKind::Text))
    };
    let only = Entry {
        nullable: true,
        ..Entry::new("ONLY", Node::Value(ValueKind::TextList))
    };
    let target = Entry {
        required: true,
        ..Entry::new("target", Node::Value(ValueKind::Text))
    };
    assert_eq!(
        variants,
        [
            Variant {
                value: "one-to-one".into(),
                title: "One to one".into(),
                description: "One chat to one chat.".into(),
                table: Table {
                    entries: vec![source]
                },
            },
            Variant {
                value: "many-to-one".into(),
                title: "Fan in".into(),
                description: String::new(),
                table: Table {
                    entries: vec![only]
                },
            },
            Variant {
                value: "off".into(),
                title: "Off".into(),
                description: String::new(),
                table: Table::default(),
            },
            Variant {
                value: "wrapped".into(),
                title: "Wrapped".into(),
                description: String::new(),
                table: Table {
                    entries: vec![target]
                },
            },
        ]
    );
}

#[test]
fn the_runtime_settings_come_from_the_type() {
    let settings = Description::of::<Config>("Fixture", "").runtime_settings();
    let keys: Vec<&str> = settings.iter().map(|setting| &*setting.key).collect();
    assert_eq!(keys, ["limit", "level"]);
    assert_eq!(settings[1].kind, Kind::Text { optional: true });
}

#[test]
fn a_rendered_file_reads_back_as_the_type() {
    let description = Description::of::<Config>("Fixture", "A fixture.");
    let values = json!({
        "listen": "0.0.0.0:2",
        "limit": 5,
        "level": "very_loud",
        "token_env": "APP_TOKEN",
        "when": "7",
        "tuning": {"worker-count": 3, "names": ["a"]},
        "rules": {
            "r1": {"kind": "one-to-one", "priority": 1, "source": "x"},
            "r2": {"kind": "many-to-one", "ONLY": ["y"]},
        },
        "verbose": true,
    });
    let rendered = description.render(&values);
    let config: Config = Figment::from(Toml::string(&rendered))
        .extract()
        .unwrap_or_else(|error| panic!("{error}\n{rendered}"));
    assert_eq!(config.limit, 5);
    assert_eq!(config.when, 7);
    assert!(config.extra.verbose);
    assert!(matches!(config.rules["r2"].flow, Flow::ManyToOne { .. }));
    // The example reads too: the required keys are set or hinted.
    assert!(
        description
            .example()
            .contains("# Where to listen: an address.\n# Required.\n")
    );
}

#[test]
fn only_an_option_makes_a_runtime_setting_clearable() {
    #[derive(Describe, Deserialize)]
    #[allow(dead_code)]
    struct Runtime {
        #[serde(default)]
        #[botconf(runtime)]
        limit: u32,
        #[botconf(runtime)]
        name: Option<String>,
    }
    let settings = Description::of::<Runtime>("R", "").runtime_settings();
    assert!(matches!(
        settings[0].kind,
        Kind::Number {
            optional: false,
            ..
        }
    ));
    assert_eq!(settings[1].kind, Kind::Text { optional: true });
}
