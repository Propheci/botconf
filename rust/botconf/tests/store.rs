//! The store, with a toy schema: nothing here is specific to a bot.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use botconf::{
    Kind, MemoryStorage, RuntimeSetting, Schema, Section, SectionSettings, SettingsError,
    SettingsStore, Snapshot, Source, Storage, StoredOverride,
    command::{self, Outcome, SettingsCommand},
};
use figment::{
    Figment, Jail,
    providers::{Format, Toml},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    greeting: String,
    #[serde(default)]
    admins: Vec<i64>,
    #[serde(default)]
    limits: BTreeMap<String, u32>,
    #[serde(default)]
    features: BTreeMap<String, Value>,
}

#[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Pets {
    names: Vec<String>,
    max: u32,
}

/// Counts the snapshots it is told about.
#[derive(Default)]
struct Toy {
    changes: Arc<AtomicUsize>,
}

const PET_SETTINGS: &[RuntimeSetting] = &[RuntimeSetting::new("max", "How many pets")];

impl Schema for Toy {
    type Config = Config;
    /// The number of admins.
    type Derived = usize;

    fn derive(&self, config: &Config) -> Result<usize, String> {
        if config.greeting.is_empty() {
            return Err("the greeting must not be empty".into());
        }
        Ok(config.admins.len())
    }

    fn settings(&self) -> Vec<RuntimeSetting> {
        vec![
            RuntimeSetting::new("greeting", "What to say"),
            RuntimeSetting::new("admins", "Who's in charge")
                .kind(Kind::reference("test.user", true)),
            RuntimeSetting::per_entry("limits", "Per-feature limits", &Kind::Json),
        ]
    }

    fn sections(&self) -> Vec<Section> {
        vec![Section::new(
            "pets",
            "features.pets",
            "Pets",
            "The pets feature",
            SectionSettings::of::<Pets>(PET_SETTINGS),
        )]
    }

    fn lint(&self, snapshot: &Snapshot<Self>) -> Vec<String> {
        (snapshot.derived == 0)
            .then(|| "nobody is in charge".to_string())
            .into_iter()
            .collect()
    }

    fn on_change(&self, _previous: Option<&Snapshot<Self>>, _current: &Snapshot<Self>) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }
}

const BASE: &str = r#"
greeting = "hi"
limits = { uploads = 5 }
"#;

fn base() -> Figment {
    Figment::from(Toml::string(BASE))
}

async fn store_with(storage: Arc<MemoryStorage>) -> SettingsStore<Toy> {
    SettingsStore::load(Toy::default(), base(), storage)
        .await
        .expect("load")
}

async fn store() -> SettingsStore<Toy> {
    store_with(Arc::default()).await
}

#[tokio::test]
async fn changes_apply_immediately_and_persist() {
    let storage = Arc::new(MemoryStorage::new());
    let store = store_with(Arc::clone(&storage)).await;

    let change = store
        .set("admins", json!([5, 6]), Some("1".into()))
        .await
        .unwrap();
    assert_eq!(change.previous.derived, 0);
    assert_eq!(change.current.derived, 2);
    assert_eq!(store.current().config.admins, [5, 6]);
    assert_eq!(store.current().source("admins"), Source::Stored);

    assert_eq!(
        storage.load().await.unwrap(),
        [StoredOverride {
            key: "admins".into(),
            value: "[5,6]".into(),
            by: Some("1".into()),
        }]
    );

    // A new store (i.e. a restart) sees the change.
    let restarted = store_with(storage).await;
    assert_eq!(restarted.current().config.admins, [5, 6]);
}

#[test]
#[allow(clippy::result_large_err)] // `figment::Jail`'s API.
fn sources_tell_where_values_come_from() {
    Jail::expect_with(|jail| {
        jail.create_file("config.toml", BASE)?;
        let base = Figment::from(Toml::file("config.toml"));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let store = SettingsStore::load(Toy::default(), base, MemoryStorage::new())
                .await
                .unwrap();
            let snapshot = store.current();
            assert_eq!(snapshot.source("greeting"), Source::File);
            assert_eq!(snapshot.source("admins"), Source::Default);
            assert_eq!(snapshot.value("admins"), Some(json!([])));
        });
        Ok(())
    });
}

#[tokio::test]
async fn invalid_changes_are_rejected_and_not_stored() {
    let storage = Arc::new(MemoryStorage::new());
    let store = store_with(Arc::clone(&storage)).await;

    for (key, value) in [
        ("admins", json!("everyone")),
        ("greeting", json!("")), // rejected by the schema
        ("limits.uploads", json!(-1)),
        ("features.pets.max", json!("lots")),
    ] {
        let error = store.set(key, value, None).await.unwrap_err();
        assert!(
            matches!(error, SettingsError::InvalidValue { .. }),
            "{key}: {error}"
        );
    }
    assert!(matches!(
        store.set("secret", json!(1), None).await,
        Err(SettingsError::UnknownKey(_))
    ));
    assert!(storage.load().await.unwrap().is_empty());
    assert!(store.current().overrides().is_empty());
}

#[tokio::test]
async fn maps_are_replaced_whole_or_entry_by_entry() {
    let store = store().await;

    store.set("limits.downloads", json!(3), None).await.unwrap();
    assert_eq!(
        store.current().config.limits,
        BTreeMap::from([("downloads".into(), 3), ("uploads".into(), 5)])
    );

    // A whole map replaces the base's, and the entries' overrides.
    store
        .set("limits", json!({ "pings": 1 }), None)
        .await
        .unwrap();
    assert_eq!(
        store.current().config.limits,
        BTreeMap::from([("pings".into(), 1)])
    );
    assert_eq!(
        store.current().overrides().keys().collect::<Vec<_>>(),
        ["limits"]
    );

    store.delete_entry("limits.pings", None).await.unwrap();
    assert!(store.current().config.limits.is_empty());
    assert!(matches!(
        store.delete_entry("limits", None).await,
        Err(SettingsError::NotAnEntry(_))
    ));
}

#[tokio::test]
async fn lists_are_extended_and_trimmed() {
    let store = store().await;
    store.add("admins", json!(2), None).await.unwrap();
    store
        .extend("admins", vec![json!(2), json!(3)], None)
        .await
        .unwrap();
    store.remove("admins", json!(2), None).await.unwrap();
    assert_eq!(store.current().config.admins, [3]);
    assert!(matches!(
        store.add("greeting", json!(1), None).await,
        Err(SettingsError::NotAList(_))
    ));
}

#[tokio::test]
async fn unset_falls_back_to_the_base() {
    let store = store().await;
    assert!(store.unset("greeting").await.unwrap().is_none());

    store.set("greeting", json!("hello"), None).await.unwrap();
    store.set("limits.pings", json!(1), None).await.unwrap();
    store.unset("greeting").await.unwrap().unwrap();
    store.unset("limits").await.unwrap().unwrap();

    let snapshot = store.current();
    assert_eq!(snapshot.config.greeting, "hi");
    assert!(!snapshot.config.limits.contains_key("pings"));
    assert!(snapshot.overrides().is_empty());
}

#[tokio::test]
async fn invalid_stored_values_are_ignored_until_unset() {
    let storage = Arc::new(MemoryStorage::new());
    for (key, value) in [
        ("greeting", "\"\""),
        ("admins", "[1]"),
        ("secret", "1"),
        ("limits", "not json"),
    ] {
        let stored = StoredOverride {
            key: key.into(),
            value: value.into(),
            by: None,
        };
        storage.write(&[], Some(&stored)).await.unwrap();
    }

    let store = store_with(Arc::clone(&storage)).await;
    let snapshot = store.current();
    assert_eq!(snapshot.config.admins, [1], "valid ones still apply");
    assert_eq!(
        snapshot.ignored().keys().collect::<Vec<_>>(),
        ["greeting", "limits", "secret"]
    );

    store.unset("greeting").await.unwrap().unwrap();
    assert!(!store.current().ignored().contains_key("greeting"));
    assert_eq!(storage.load().await.unwrap().len(), 3);
}

#[tokio::test]
async fn sections_are_typed() {
    let store = store().await;
    assert_eq!(
        store.current().section::<Pets>("pets"),
        Some(&Pets::default())
    );
    assert_eq!(store.current().value("features.pets.max"), Some(json!(0)));

    store
        .set("features.pets.max", json!(3), None)
        .await
        .unwrap();
    let snapshot = store.current();
    assert_eq!(snapshot.section::<Pets>("pets").unwrap().max, 3);
    assert_eq!(snapshot.section::<String>("pets"), None, "wrong type");
    assert_eq!(
        store
            .catalog()
            .resolve("features.pets.max")
            .unwrap()
            .section
            .as_deref(),
        Some("pets")
    );

    let view: &dyn botconf::View = snapshot.as_ref();
    assert_eq!(view.section::<Pets>("pets").unwrap().max, 3);
    assert_eq!(view.derived::<usize>(), Some(&0));
    assert!(view.schema::<Toy>().is_some());
}

#[tokio::test]
async fn the_schema_hears_about_every_snapshot_and_lints_changes() {
    let schema = Toy::default();
    let changes = Arc::clone(&schema.changes);
    let store = SettingsStore::load(schema, base(), MemoryStorage::new())
        .await
        .unwrap();
    assert_eq!(changes.load(Ordering::SeqCst), 1);

    let outcome = command::execute(
        &store,
        SettingsCommand::Set("greeting".into(), "hello".into()),
        None,
    )
    .await
    .unwrap();
    let Outcome::Changed { lints, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(lints, ["nobody is in charge"]);
    assert_eq!(changes.load(Ordering::SeqCst), 2);

    store.reload().await.unwrap();
    assert_eq!(changes.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn an_invalid_base_is_refused() {
    let base = Figment::from(Toml::string(r#"greeting = """#));
    assert!(matches!(
        SettingsStore::load(Toy::default(), base, MemoryStorage::new()).await,
        Err(SettingsError::InvalidBase(_))
    ));
}

#[tokio::test]
async fn subscribers_see_changes() {
    let store = store().await;
    let mut published = store.subscribe();
    assert_eq!(published.borrow_and_update().config.greeting, "hi");

    store.set("greeting", json!("hello"), None).await.unwrap();
    // Published before `set` returns.
    assert!(published.has_changed().unwrap());
    assert_eq!(published.borrow_and_update().config.greeting, "hello");

    // A rejected change publishes nothing.
    store.set("greeting", json!(""), None).await.unwrap_err();
    assert!(!published.has_changed().unwrap());
}

/// Runs `test` inside a [`Jail`] holding `config.toml` with `BASE`, on a
/// tokio runtime.
#[allow(clippy::result_large_err)] // `figment::Jail`'s API.
fn in_jail(test: impl AsyncFnOnce(&mut Jail)) {
    Jail::expect_with(|jail| {
        jail.create_file("config.toml", BASE)?;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(test(jail));
        Ok(())
    });
}

/// A source that reads `config.toml` again on every call.
#[allow(clippy::result_large_err)] // `figment::Error`, as `Figment::extract` returns.
fn rereading() -> impl Fn() -> Result<Figment, figment::Error> + Send + Sync + 'static {
    || Ok(Figment::from(Toml::file("config.toml")))
}

#[test]
fn reload_reads_an_edited_file() {
    in_jail(async |jail| {
        let store = SettingsStore::load(Toy::default(), rereading(), MemoryStorage::new())
            .await
            .unwrap();
        assert_eq!(store.current().config.greeting, "hi");

        jail.create_file("config.toml", r#"greeting = "hello""#)
            .unwrap();
        store.reload().await.unwrap();
        let snapshot = store.current();
        assert_eq!(snapshot.config.greeting, "hello");
        assert_eq!(snapshot.source("greeting"), Source::File);
    });
}

#[test]
fn reload_keeps_the_snapshot_when_the_file_becomes_invalid() {
    in_jail(async |jail| {
        let store = SettingsStore::load(Toy::default(), rereading(), MemoryStorage::new())
            .await
            .unwrap();

        // The schema rejects an empty greeting.
        jail.create_file("config.toml", r#"greeting = """#).unwrap();
        assert!(matches!(
            store.reload().await,
            Err(SettingsError::InvalidBase(_))
        ));
        assert_eq!(store.current().config.greeting, "hi");

        // The old base is still the one changes layer on.
        store.set("admins", json!([1]), None).await.unwrap();
        assert_eq!(store.current().config.greeting, "hi");
    });
}

#[test]
fn reload_keeps_the_snapshot_when_the_source_fails() {
    in_jail(async |_| {
        let calls = AtomicUsize::new(0);
        let source = move || {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(Figment::from(Toml::file("config.toml")))
            } else {
                Err("the config file is gone")
            }
        };
        let store = SettingsStore::load(Toy::default(), source, MemoryStorage::new())
            .await
            .unwrap();

        let error = store.reload().await.unwrap_err();
        assert!(
            matches!(
                error,
                SettingsError::InvalidBase(botconf::BuildError::Source(ref reason))
                    if reason == "the config file is gone"
            ),
            "{error}"
        );
        assert_eq!(store.current().config.greeting, "hi");
    });
}

#[test]
fn overrides_still_win_over_an_edited_file() {
    in_jail(async |jail| {
        let store = SettingsStore::load(Toy::default(), rereading(), MemoryStorage::new())
            .await
            .unwrap();
        store.set("greeting", json!("stored"), None).await.unwrap();

        jail.create_file("config.toml", r#"greeting = "edited""#)
            .unwrap();
        store.reload().await.unwrap();
        assert_eq!(store.current().config.greeting, "stored");

        // Later changes layer on the edited file.
        store.unset("greeting").await.unwrap().unwrap();
        assert_eq!(store.current().config.greeting, "edited");
    });
}

#[test]
fn a_fixed_base_does_not_reread() {
    in_jail(async |jail| {
        let base = Figment::from(Toml::file("config.toml"));
        let store = SettingsStore::load(Toy::default(), base, MemoryStorage::new())
            .await
            .unwrap();

        jail.create_file("config.toml", r#"greeting = "hello""#)
            .unwrap();
        store.reload().await.unwrap();
        assert_eq!(store.current().config.greeting, "hi");
    });
}

#[tokio::test]
async fn check_validates_without_storing() {
    let storage = Arc::new(MemoryStorage::new());
    let store = store_with(Arc::clone(&storage)).await;
    let mut published = store.subscribe();
    published.borrow_and_update();

    store.check("greeting", json!("hello")).await.unwrap();
    let refused = store.check("greeting", json!("")).await.unwrap_err();
    assert!(
        matches!(&refused, SettingsError::InvalidValue { key, .. } if key == "greeting"),
        "{refused}"
    );
    // The same refusal as `set`'s.
    let set = store.set("greeting", json!(""), None).await.unwrap_err();
    assert_eq!(refused.to_string(), set.to_string());
    assert!(matches!(
        store.check("nope", json!(1)).await,
        Err(SettingsError::UnknownKey(_))
    ));

    // Nothing stored, published or changed.
    assert_eq!(store.current().config.greeting, "hi");
    assert!(store.current().overrides().is_empty());
    assert!(storage.load().await.unwrap().is_empty());
    assert!(!published.has_changed().unwrap());
}
