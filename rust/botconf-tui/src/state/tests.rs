use std::collections::BTreeMap;

use botconf::Describe;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use serde_json::json;

use super::*;

/// The sample program's configuration.
#[derive(Describe, Deserialize)]
#[allow(dead_code)]
struct Config {
    /// Where to listen.
    listen: String,
    #[serde(default)]
    #[botconf(min = 0, max = 10)]
    workers: u8,
    #[serde(default)]
    verbose: bool,
    #[serde(default)]
    level: Level,
    /// The core's token.
    #[botconf(env = token, hint = "APP_TOKEN")]
    token_env: Option<String>,
    #[serde(default)]
    connectors: Vec<Connector>,
    #[serde(default)]
    rules: BTreeMap<String, Rule>,
}

#[derive(Default, Describe, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    #[default]
    Info,
    Debug,
}

#[derive(Describe, Deserialize)]
#[allow(dead_code)]
struct Connector {
    id: String,
    #[serde(default)]
    names: Vec<String>,
}

#[derive(Describe, Deserialize)]
#[allow(dead_code)]
struct Rule {
    #[serde(default)]
    priority: i32,
    #[serde(flatten)]
    flow: Flow,
}

#[derive(Describe, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
#[allow(dead_code)]
enum Flow {
    One { source: String },
    Many { only: Vec<String> },
}

fn fake_token() -> Result<String, String> {
    Ok("generated".into())
}

fn app(values: Value, env: &str) -> App {
    App::new(
        Description::of::<Config>("Sample", ""),
        values,
        EnvFile::parse(env),
        ".env.local".into(),
        fake_token,
    )
}

fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn typing(app: &mut App, text: &str) {
    for c in text.chars() {
        key(app, KeyCode::Char(c));
    }
}

/// Moves the cursor to the row titled `title`.
fn select(app: &mut App, title: &str) {
    let index = app
        .rows()
        .iter()
        .position(|row| App::title_of(row) == title)
        .unwrap_or_else(|| panic!("no row {title}"));
    app.screens.last_mut().unwrap().cursor = index;
}

fn titles(app: &App) -> Vec<String> {
    app.rows().iter().map(App::title_of).collect()
}

#[test]
fn an_option_is_edited_in_place() {
    let mut app = app(json!({}), "");
    assert_eq!(app.shown(&app.rows()[0]), Shown::Missing);
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::Edit(_)));
    typing(&mut app, "127.0.0.1:9");
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::Browse));
    assert_eq!(app.values, json!({"listen": "127.0.0.1:9"}));
    assert!(app.dirty);
    // Editing starts from the value; an empty input unsets it.
    key(&mut app, KeyCode::Enter);
    let Mode::Edit(edit) = &app.mode else {
        panic!("editing");
    };
    assert_eq!(edit.input.text, "127.0.0.1:9");
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.values, json!({}));
}

#[test]
fn a_bad_value_keeps_the_editor_open_with_the_reason() {
    let mut app = app(json!({}), "");
    select(&mut app, "Workers");
    assert_eq!(app.shown(&app.rows()[1]), Shown::Default("0".into()));
    key(&mut app, KeyCode::Enter);
    typing(&mut app, "11");
    key(&mut app, KeyCode::Enter);
    let Mode::Edit(edit) = &app.mode else {
        panic!("still editing");
    };
    assert_eq!(edit.note, "From 0 to 10.");
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.values, json!({"workers": 1}));
}

#[test]
fn booleans_toggle_choices_are_picked_and_u_unsets() {
    let mut app = app(json!({}), "");
    select(&mut app, "Verbose");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.values["verbose"], json!(true));
    select(&mut app, "Level");
    key(&mut app, KeyCode::Enter);
    let Mode::Choose(choose) = &app.mode else {
        panic!("choosing");
    };
    assert_eq!(choose.cursor, 0, "the default is picked");
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.values["level"], json!("debug"));
    key(&mut app, KeyCode::Char('u'));
    select(&mut app, "Verbose");
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.values, json!({}));
}

#[test]
fn list_items_are_added_opened_and_deleted() {
    let mut app = app(json!({}), "");
    select(&mut app, "Connectors");
    assert_eq!(app.shown(&app.rows()[5]), Shown::Opens("none".into()));
    key(&mut app, KeyCode::Enter);
    assert!(app.is_collection() && app.rows().is_empty());
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(titles(&app), ["Id", "Names"]);
    key(&mut app, KeyCode::Enter);
    typing(&mut app, "tg");
    key(&mut app, KeyCode::Enter);
    select(&mut app, "Names");
    key(&mut app, KeyCode::Enter);
    typing(&mut app, "a, b");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    assert_eq!(titles(&app), ["1. tg"]);
    assert_eq!(
        app.values["connectors"],
        json!([{"id": "tg", "names": ["a", "b"]}])
    );
    key(&mut app, KeyCode::Char('d'));
    assert!(matches!(app.mode, Mode::Confirm(Confirm::Delete { .. })));
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(app.values["connectors"], json!([]));
}

#[test]
fn a_tag_picks_the_options_and_drops_the_other_variants() {
    let mut app = app(json!({}), "");
    select(&mut app, "Rules");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('a'));
    typing(&mut app, "r1");
    key(&mut app, KeyCode::Enter);
    assert_eq!(titles(&app), ["Kind", "Priority"]);
    assert_eq!(app.shown(&app.rows()[0]), Shown::Missing);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    assert_eq!(titles(&app), ["Kind", "Priority", "Source"]);
    select(&mut app, "Source");
    key(&mut app, KeyCode::Enter);
    typing(&mut app, "tg:1");
    key(&mut app, KeyCode::Enter);
    select(&mut app, "Kind");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(titles(&app), ["Kind", "Priority", "Only"]);
    assert_eq!(app.values["rules"], json!({"r1": {"kind": "many"}}));
    // A taken name is refused.
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('a'));
    typing(&mut app, "r1");
    key(&mut app, KeyCode::Enter);
    let Mode::Edit(edit) = &app.mode else {
        panic!("still naming");
    };
    assert_eq!(edit.note, "r1 already exists.");
}

#[test]
fn a_token_is_generated_into_the_env_file() {
    let mut app = app(json!({}), "# mine\nOTHER=1\n");
    select(&mut app, "Token env");
    key(&mut app, KeyCode::Char('g'));
    // The hinted name is written, so the file names the variable.
    assert_eq!(app.values["token_env"], json!("APP_TOKEN"));
    assert_eq!(app.env.get("APP_TOKEN"), Some("generated"));
    assert_eq!(app.candidate().1, "# mine\nOTHER=1\nAPP_TOKEN=generated\n");
    // Replacing one asks first.
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(
        app.mode.clone_confirm(),
        Some(Confirm::Regenerate {
            var: "APP_TOKEN".into()
        })
    );
    key(&mut app, KeyCode::Char('n'));
    // e sets it by hand.
    key(&mut app, KeyCode::Char('e'));
    let Mode::Edit(edit) = &app.mode else {
        panic!("editing");
    };
    assert!(edit.secret);
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    typing(&mut app, "mine");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.env.get("APP_TOKEN"), Some("mine"));
    let (_, facts) = app.details(&app.rows()[4]);
    assert!(facts.last().unwrap().contains("APP_TOKEN is set (hidden)"));
}

#[test]
fn quitting_with_changes_asks() {
    let mut app = app(json!({}), "");
    assert_eq!(key(&mut app, KeyCode::Char('q')), Effect::Quit);
    select(&mut app, "Verbose");
    key(&mut app, KeyCode::Enter);
    assert_eq!(key(&mut app, KeyCode::Char('q')), Effect::None);
    assert_eq!(key(&mut app, KeyCode::Char('y')), Effect::Quit);
    assert_eq!(key(&mut app, KeyCode::Char('s')), Effect::Save);
    app.saved(Err("listen: missing".into()));
    assert!(matches!(&app.mode, Mode::Message(text) if text == "Not saved: listen: missing"));
    key(&mut app, KeyCode::Char('x'));
    app.saved(Ok("Saved".into()));
    assert!(!app.dirty);
}

#[test]
fn the_candidate_reads_back_as_the_type() {
    let values = json!({
        "listen": "0.0.0.0:1",
        "connectors": [{"id": "tg"}],
        "rules": {"r": {"kind": "one", "source": "x"}},
        "stray": true,
    });
    let app = app(values, "");
    let (config, _) = app.candidate();
    let parsed: Config =
        toml::from_str(&config).unwrap_or_else(|error| panic!("{error}\n{config}"));
    assert_eq!(parsed.connectors.len(), 1);
    assert!(config.contains("# stray = true"), "{config}");
}

impl Mode {
    fn clone_confirm(&self) -> Option<Confirm> {
        match self {
            Mode::Confirm(confirm) => Some(confirm.clone()),
            _ => None,
        }
    }
}

#[test]
fn every_mode_draws_at_any_size() {
    use ratatui::{Terminal, backend::TestBackend};

    let reach: [(&str, &[KeyCode]); 6] = [
        ("browse", &[]),
        ("edit", &[KeyCode::Enter]),
        (
            "choose",
            &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
        ),
        (
            "confirm",
            &[
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Enter,
                KeyCode::Char('q'),
            ],
        ),
        ("help", &[KeyCode::Char('?')]),
        ("collection", &[KeyCode::End, KeyCode::Up, KeyCode::Enter]),
    ];
    for (width, height) in [(120, 30), (60, 12), (10, 3), (1, 1)] {
        for (name, keys) in reach {
            let mut app = app(json!({"connectors": [{"id": "tg"}]}), "");
            for code in keys {
                key(&mut app, *code);
            }
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::view::draw(frame, &app))
                .unwrap_or_else(|error| panic!("{name} at {width}x{height}: {error}"));
            if width == 120 {
                let screen: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(screen.contains("Sample"), "{name}");
            }
        }
    }
}
