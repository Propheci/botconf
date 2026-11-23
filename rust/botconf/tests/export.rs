//! A backup of a store (its file, environment and stored overrides) and its
//! restore into fresh files and storage.

use std::collections::BTreeMap;
use std::path::PathBuf;

use botconf::describe::Description;
use botconf::export::{Backup, ImportError, ImportTarget, export, import, import_online};
use botconf::storage::{MemoryStorage, Storage, StoredOverride};
use botconf::{Describe, RuntimeSetting, Schema, SettingsStore};
use figment::{
    Figment,
    providers::{Format, Toml},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A sample bot's configuration.
#[derive(Debug, Describe, Deserialize, Serialize)]
struct Config {
    /// Where it listens.
    listen: String,
    /// What it says.
    #[serde(default)]
    #[botconf(runtime)]
    greeting: String,
    /// The variable holding its token.
    #[botconf(env = token)]
    token_env: String,
    /// The password of its admin page.
    #[botconf(secret)]
    password: Option<String>,
    #[serde(default)]
    peers: Vec<Peer>,
}

#[derive(Debug, Describe, Deserialize, Serialize)]
struct Peer {
    name: String,
    /// The variable holding the peer's token.
    #[botconf(env = token)]
    token_env: String,
}

struct Bot;

impl Schema for Bot {
    type Config = Config;
    type Derived = ();

    fn derive(&self, config: &Config) -> Result<(), String> {
        if config.listen.is_empty() {
            return Err("listen must not be empty".into());
        }
        Ok(())
    }

    fn settings(&self) -> Vec<RuntimeSetting> {
        description().runtime_settings()
    }

    fn dynamic_namespaces(&self) -> Vec<&'static str> {
        vec!["plugins"]
    }
}

fn description() -> Description {
    Description::of::<Config>("Sample bot", "")
}

const FILE: &str = r#"
listen = "127.0.0.1:1"
greeting = "hi"
token_env = "BOT_TOKEN"
password = "hunter2"

[[peers]]
name = "a"
token_env = "PEER_TOKEN"
"#;

fn stored(key: &str, value: Value) -> StoredOverride {
    StoredOverride {
        key: key.into(),
        value: value.to_string(),
        by: Some("someone".into()),
    }
}

/// A store over FILE with an override of its own key and one below the
/// dynamic namespace (parked: nothing registers it offline).
async fn exported(secrets: bool) -> String {
    let storage = MemoryStorage::new();
    for row in [
        stored("greeting", json!("hello")),
        stored("plugins.echo.level", json!(3)),
    ] {
        storage.write(&[], Some(&row)).await.unwrap();
    }
    let store = SettingsStore::load(Bot, Figment::from(Toml::string(FILE)), storage)
        .await
        .unwrap();
    let env = BTreeMap::from([("BOT_TOKEN", "t1"), ("PEER_TOKEN", "t2"), ("OTHER", "x")]);
    let lookup = |name: &str| env.get(name).map(|value| (*value).to_owned());
    export(
        &description(),
        &store.current(),
        secrets.then_some(&lookup as &dyn Fn(&str) -> Option<String>),
    )
    .unwrap()
}

fn temporary() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "botconf-export-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn a_backup_holds_the_effective_values_and_the_dynamic_overrides() {
    let text = exported(false).await;
    let backup = Backup::parse(&text).unwrap();
    assert_eq!(
        backup.config["greeting"],
        json!("hello"),
        "the override applied:\n{text}"
    );
    assert_eq!(backup.config["token_env"], json!("BOT_TOKEN"));
    assert!(
        backup.config.get("password").is_none(),
        "no secret without the flag:\n{text}"
    );
    assert!(text.contains("# password = "), "{text}");
    assert_eq!(
        backup.overrides,
        BTreeMap::from([("plugins.echo.level".to_owned(), json!(3))])
    );
    assert!(backup.env.is_empty() && !backup.secrets);
    // It reads as the config file: each option described.
    assert!(text.contains("# Where it listens."), "{text}");
}

#[tokio::test]
async fn secrets_come_only_with_the_flag() {
    let backup = Backup::parse(&exported(true).await).unwrap();
    assert_eq!(backup.config["password"], json!("hunter2"));
    assert_eq!(
        backup.env,
        BTreeMap::from([
            ("BOT_TOKEN".to_owned(), "t1".to_owned()),
            ("PEER_TOKEN".to_owned(), "t2".to_owned())
        ]),
        "only the variables its options name"
    );
}

#[tokio::test]
async fn an_import_restores_the_files_and_the_overrides() {
    let dir = temporary();
    let (config, env_file) = (dir.join("config.toml"), dir.join(".env"));
    std::fs::write(
        &config,
        "listen = \"old\"\ntoken_env = \"X\"\npassword = \"kept\"\n",
    )
    .unwrap();
    std::fs::write(&env_file, "# mine\nOTHER=1\n").unwrap();
    let storage = MemoryStorage::new();
    for row in [
        stored("greeting", json!("stale")),
        stored("plugins.old.level", json!(1)),
    ] {
        storage.write(&[], Some(&row)).await.unwrap();
    }
    let backup = Backup::parse(&exported(false).await).unwrap();
    let target = ImportTarget {
        config: &config,
        env_file: &env_file,
        storage: &storage,
        namespaces: &["plugins"],
    };
    let imported = import(&description(), &backup, &target, |text, _env| {
        toml::from_str::<Config>(text)
            .map(drop)
            .map_err(|error| error.to_string())
    })
    .await
    .unwrap();
    assert_eq!(
        (imported.removed, imported.stored, imported.env_vars),
        (2, 1, 0)
    );
    assert_eq!(
        imported.kept,
        [dir.join("config.toml.bak"), dir.join(".env.bak")]
    );
    let written: Config = toml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(written.greeting, "hello", "the override is in the file now");
    assert_eq!(
        written.password.as_deref(),
        Some("kept"),
        "a backup without secrets keeps the file's"
    );
    assert_eq!(written.peers.len(), 1);
    assert_eq!(
        std::fs::read_to_string(dir.join("config.toml.bak")).unwrap(),
        "listen = \"old\"\ntoken_env = \"X\"\npassword = \"kept\"\n"
    );
    let rows: BTreeMap<String, String> = storage
        .load()
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.key, row.value))
        .collect();
    assert_eq!(
        rows,
        BTreeMap::from([("plugins.echo.level".to_owned(), "3".to_owned())])
    );
    assert_eq!(
        std::fs::read_to_string(&env_file).unwrap(),
        "# mine\nOTHER=1\n",
        "no secrets: untouched"
    );

    // With secrets, the env file's variables are set, its other lines kept.
    let with = Backup::parse(&exported(true).await).unwrap();
    let imported = import(&description(), &with, &target, |_, env| {
        (env.get("BOT_TOKEN").map(String::as_str) == Some("t1"))
            .then_some(())
            .ok_or_else(|| "the token is checked as it will be".to_owned())
    })
    .await
    .unwrap();
    assert_eq!(imported.env_vars, 2);
    assert_eq!(
        std::fs::read_to_string(&env_file).unwrap(),
        "# mine\nOTHER=1\nBOT_TOKEN=t1\nPEER_TOKEN=t2\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_refused_backup_writes_nothing() {
    let dir = temporary();
    let (config, env_file) = (dir.join("config.toml"), dir.join(".env"));
    std::fs::write(&config, "listen = \"old\"\n").unwrap();
    let storage = MemoryStorage::new();
    storage
        .write(&[], Some(&stored("greeting", json!("kept"))))
        .await
        .unwrap();
    let backup = Backup::parse(&exported(true).await).unwrap();
    let target = ImportTarget {
        config: &config,
        env_file: &env_file,
        storage: &storage,
        namespaces: &["plugins"],
    };
    let refused = import(&description(), &backup, &target, |_, _| Err("no".into())).await;
    assert!(matches!(refused, Err(ImportError::Rejected(reason)) if reason == "no"));
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "listen = \"old\"\n"
    );
    assert!(!env_file.exists() && !dir.join("config.toml.bak").exists());
    assert_eq!(storage.load().await.unwrap().len(), 1);

    // An override outside the dynamic namespaces is not the host's to store.
    let mut stray = backup.clone();
    stray.overrides.insert("greeting".into(), json!("x"));
    let refused = import(&description(), &stray, &target, |_, _| Ok(())).await;
    assert!(
        matches!(refused, Err(ImportError::Rejected(_))),
        "{refused:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_config_file_is_not_a_backup() {
    assert!(matches!(Backup::parse(FILE), Err(ImportError::Invalid(_))));
    assert!(matches!(
        Backup::parse("[backup]\nformat = 9\n"),
        Err(ImportError::Format { found: 9 })
    ));
}

#[tokio::test]
async fn an_online_import_sets_what_changes_at_runtime_and_lists_the_rest() {
    let mut backup = Backup::parse(&exported(false).await).unwrap();
    backup.config["listen"] = json!("0.0.0.0:9");
    let store = SettingsStore::load(Bot, Figment::from(Toml::string(FILE)), MemoryStorage::new())
        .await
        .unwrap();

    let dry = import_online(&store, &backup, Some("me".into()), true).await;
    assert_eq!(dry.applied, ["greeting"]);
    assert_eq!(
        store.current().config.greeting,
        "hi",
        "a dry run changes nothing"
    );

    let done = import_online(&store, &backup, Some("me".into()), false).await;
    assert_eq!(done.applied, ["greeting"]);
    assert_eq!(done.offline, ["listen"], "only the file holds it");
    // Nothing registered the plugin's section: its override is refused.
    assert_eq!(done.rejected.len(), 1);
    assert_eq!(done.rejected[0].0, "plugins.echo.level");
    assert_eq!(store.current().config.greeting, "hello");
    // A second import finds nothing left to set; the password, absent from a
    // backup without secrets, is not a difference.
    let again = import_online(&store, &backup, None, false).await;
    assert!(
        again.applied.is_empty() && again.offline == ["listen"],
        "{again:?}"
    );
}
