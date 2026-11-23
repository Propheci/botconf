use figment::{
    Figment,
    providers::{Format, Toml},
};
use serde_json::json;

use super::*;

fn value(key: &str, kind: ValueKind) -> Entry {
    Entry::new(key, Node::Value(kind))
}

fn described(entry: Entry, description: &str) -> Entry {
    Entry {
        description: description.into(),
        ..entry
    }
}

/// Every shape: values (required, defaulted, with a hint, a choice, a
/// runtime one), a table, a list of tables, a map of tables, a tagged map.
fn sample() -> Description {
    let connector = Table {
        entries: vec![
            Entry {
                required: true,
                ..described(value("id", ValueKind::Text), "The connector's id.")
            },
            Entry {
                default: Some(json!(false)),
                ..value("threads", ValueKind::Bool)
            },
        ],
    };
    let rule = Node::Tagged {
        tag: "kind".into(),
        common: Table {
            entries: vec![Entry {
                default: Some(json!(0)),
                ..value("priority", ValueKind::Int { min: -10, max: 10 })
            }],
        },
        variants: vec![
            Variant {
                value: "one".into(),
                title: "One to one".into(),
                description: String::new(),
                table: Table {
                    entries: vec![Entry {
                        required: true,
                        ..value("source", ValueKind::Text)
                    }],
                },
            },
            Variant {
                value: "many".into(),
                title: "Many to one".into(),
                description: String::new(),
                table: Table {
                    entries: vec![value("only", ValueKind::TextList)],
                },
            },
        ],
    };
    Description {
        title: "Sample".into(),
        description: "A sample program.".into(),
        table: Table {
            entries: vec![
                Entry {
                    required: true,
                    example: Some(json!("127.0.0.1:1")),
                    ..described(value("listen", ValueKind::Text), "Where to listen.")
                },
                Entry {
                    default: Some(json!("info")),
                    runtime: true,
                    ..described(
                        value(
                            "level",
                            ValueKind::OneOf(vec!["info".into(), "debug".into()]),
                        ),
                        "How much to log.",
                    )
                },
                Entry {
                    hint: Some(json!("TOKEN")),
                    ..value("token_env", ValueKind::EnvVar(EnvValue::Token))
                },
                Entry::new(
                    "rendering",
                    Node::Table(Table {
                        entries: vec![Entry {
                            default: Some(json!(0)),
                            runtime: true,
                            ..value("limit", ValueKind::Int { min: 0, max: 100 })
                        }],
                    }),
                ),
                Entry {
                    example: Some(json!([{"id": "tg"}])),
                    ..described(
                        Entry::new("connectors", Node::List(Box::new(Node::Table(connector)))),
                        "The connectors.",
                    )
                },
                Entry::new("rules", Node::Map(Box::new(rule))),
            ],
        },
    }
}

fn parse(toml: &str) -> Value {
    Figment::from(Toml::string(toml))
        .extract::<Value>()
        .unwrap_or_else(|error| panic!("{error}\n{toml}"))
}

#[test]
fn titles_come_from_keys() {
    assert_eq!(title_of("max_message_length"), "Max message length");
    assert_eq!(title_of("token-env"), "Token env");
}

#[test]
fn the_example_sets_the_examples_and_comments_out_the_rest() {
    let example = sample().example();
    assert!(
        example.starts_with("# Sample\n#\n# A sample program.\n"),
        "{example}"
    );
    assert!(example.contains("\n# Where to listen.\n# Required.\nlisten = \"127.0.0.1:1\"\n"));
    assert!(example.contains(
        "\n# How much to log.\n# One of: info, debug.\n# Default: \"info\".\n# level = \"info\"\n"
    ));
    // A hint stands in for a missing default.
    assert!(example.contains("\n# token_env = \"TOKEN\"\n"));
    assert!(example.contains("\n[rendering]\n\n# Default: 0.\n# limit = 0\n"));
    assert!(example.contains("\n# The connectors.\n[[connectors]]\n\n# The connector's id.\n"));
    assert!(example.contains("# rules.<name>]") || example.contains("# [rules.<name>]"));
    assert_eq!(
        parse(&example),
        // An empty table header reads as an empty table, which a serde default
        // takes like an absent one.
        json!({"listen": "127.0.0.1:1", "rendering": {}, "connectors": [{"id": "tg"}]})
    );
}

#[test]
fn rendered_values_read_back() {
    let values = json!({
        "listen": "0.0.0.0:2",
        "level": "debug",
        "token_env": "WUPHF_TOKEN",
        "rendering": {"limit": 5},
        "connectors": [{"id": "tg", "threads": true}, {"id": "wa"}],
        "rules": {
            "a.b": {"kind": "one", "priority": 1, "source": "tg:1"},
            "plain": {"kind": "many", "only": ["x", "y \"z\""]},
        },
    });
    let rendered = sample().render(&values);
    assert_eq!(parse(&rendered), values, "{rendered}");
    // A name that is not a bare key is quoted.
    assert!(
        rendered.contains("[rules.\"a.b\"]\nkind = \"one\"\n"),
        "{rendered}"
    );
}

#[test]
fn unknown_keys_are_kept_as_comments() {
    let rendered = sample().render(&json!({
        "listen": "x",
        "stray": 1,
        "rendering": {"old": "y"},
    }));
    assert!(
        rendered.contains(
            "# Not in this program's description (kept, but it ignores them):\n# rendering.old = \
             \"y\"\n# stray = 1\n"
        ),
        "{rendered}"
    );
}

#[test]
fn long_descriptions_wrap() {
    let mut description = sample();
    description.description = "word ".repeat(40);
    for line in description.example().lines() {
        assert!(line.len() <= WIDTH, "{line:?}");
    }
}

#[test]
fn runtime_entries_become_runtime_settings() {
    let settings = sample().runtime_settings();
    let keys: Vec<&str> = settings.iter().map(|setting| &*setting.key).collect();
    assert_eq!(keys, ["level", "rendering.limit"]);
    assert!(matches!(settings[0].kind, Kind::OneOf { .. }));
    assert!(matches!(
        settings[1].kind,
        Kind::Number {
            min: 0,
            max: 100,
            ..
        }
    ));
    assert_eq!(settings[0].description.as_ref(), "How much to log.");
}

#[test]
fn std_types_describe_themselves() {
    assert_eq!(<Vec<String>>::node(), Node::Value(ValueKind::TextList));
    assert_eq!(
        <u8>::node(),
        Node::Value(ValueKind::Int { min: 0, max: 255 })
    );
    assert_eq!(
        <u64>::node(),
        Node::Value(ValueKind::Int {
            min: 0,
            max: i64::MAX
        })
    );
    assert_eq!(<Option<bool>>::node(), Node::Value(ValueKind::Bool));
    assert_eq!(
        <BTreeMap<String, u8>>::node(),
        Node::Map(Box::new(Node::Value(ValueKind::Int { min: 0, max: 255 })))
    );
}
