//! Sections registered at runtime, with a toy validator: nothing here is
//! specific to a bot.

use std::sync::Arc;

use botconf::{
    DynamicSection, Kind, MemoryStorage, RuntimeSetting, Schema, Section, SectionSettings,
    SettingsError, SettingsStore, Source, Storage, StoredOverride,
    command::{self, Outcome, SettingsCommand},
};
use figment::{
    Figment,
    providers::{Format, Toml},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The host's configuration: dynamic keys must never reach it.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    greeting: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Fixed {
    on: bool,
}

struct Host;

impl Schema for Host {
    type Config = Config;
    type Derived = ();

    fn derive(&self, _config: &Config) -> Result<(), String> {
        Ok(())
    }

    fn settings(&self) -> Vec<RuntimeSetting> {
        vec![RuntimeSetting::new("greeting", "What to say")]
    }

    fn sections(&self) -> Vec<Section> {
        vec![Section::new(
            "fixed",
            "connectors.fixed",
            "Fixed",
            "A section declared at compile time",
            SectionSettings::of::<Fixed>(&[]),
        )]
    }

    fn dynamic_namespaces(&self) -> Vec<&'static str> {
        vec!["connectors"]
    }
}

/// Accepts objects whose `limit` is a number from 1 to 10, and fills in
/// `limit: 5`.
fn limit_validator(section: &Value) -> Result<Value, String> {
    let mut section = section
        .as_object()
        .cloned()
        .ok_or("the section must be an object")?;
    let limit = section.entry("limit").or_insert(json!(5));
    match limit.as_i64() {
        Some(1..=10) => Ok(Value::Object(section)),
        _ => Err(format!(
            "`limit` must be a number from 1 to 10, not {limit}"
        )),
    }
}

fn connector(id: &str, base: Value) -> DynamicSection {
    DynamicSection {
        id: id.into(),
        path: format!("connectors.{id}"),
        title: id.into(),
        description: format!("The {id} connector"),
        settings: vec![RuntimeSetting::owned(
            "limit",
            "How many at once",
            Kind::number(1, 10, "", &[], false),
        )],
        validator: Arc::new(limit_validator),
        base,
        base_source: format!("connector {id}"),
    }
}

async fn store_with(storage: Arc<MemoryStorage>) -> SettingsStore<Host> {
    let base = Figment::from(Toml::string(r#"greeting = "hi""#));
    SettingsStore::load(Host, base, storage)
        .await
        .expect("load")
}

async fn store_raw(storage: &MemoryStorage, key: &str, value: Value) {
    let stored = StoredOverride {
        key: key.into(),
        value: value.to_string(),
        by: None,
    };
    storage.write(&[], Some(&stored)).await.unwrap();
}

fn position(store: &SettingsStore<Host>, key: &str) -> usize {
    store
        .catalog()
        .live()
        .find(|(_, entry)| entry.key == key)
        .unwrap()
        .0
}

#[tokio::test]
async fn registered_sections_layer_overrides_on_their_base() {
    let store = store_with(Arc::default()).await;
    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();

    let snapshot = store.current();
    assert_eq!(snapshot.value("connectors.a.limit"), Some(json!(3)));
    assert_eq!(
        snapshot.source("connectors.a.limit"),
        Source::Other("connector a".into())
    );

    store
        .set("connectors.a.limit", json!(7), Some("7".into()))
        .await
        .unwrap();
    let snapshot = store.current();
    assert_eq!(snapshot.value("connectors.a.limit"), Some(json!(7)));
    assert_eq!(snapshot.source("connectors.a.limit"), Source::Stored);
    assert_eq!(snapshot.dynamic("a"), Some(&json!({ "limit": 7 })));

    // The validator's defaults count as the section's value.
    store
        .register_section(connector("b", json!({})))
        .await
        .unwrap();
    assert_eq!(store.current().dynamic("b"), Some(&json!({ "limit": 5 })));
}

#[tokio::test]
async fn overrides_wait_for_their_section() {
    let storage = Arc::new(MemoryStorage::new());
    store_raw(&storage, "connectors.a.limit", json!(7)).await;
    let store = store_with(Arc::clone(&storage)).await;

    let snapshot = store.current();
    assert_eq!(snapshot.parked().get("connectors.a.limit"), Some(&json!(7)));
    assert!(snapshot.ignored().is_empty());
    let Outcome::Listing(listing) = command::execute(&store, SettingsCommand::List, None)
        .await
        .unwrap()
    else {
        panic!("not a listing");
    };
    assert_eq!(listing.parked, [("connectors.a.limit".into(), json!(7))]);

    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();
    let snapshot = store.current();
    assert_eq!(snapshot.value("connectors.a.limit"), Some(json!(7)));
    assert_eq!(snapshot.source("connectors.a.limit"), Source::Stored);
    assert!(snapshot.parked().is_empty());
}

#[tokio::test]
async fn invalid_parked_overrides_are_ignored_on_registration() {
    let storage = Arc::new(MemoryStorage::new());
    store_raw(&storage, "connectors.a.limit", json!(99)).await;
    let store = store_with(storage).await;

    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();
    let snapshot = store.current();
    assert_eq!(snapshot.value("connectors.a.limit"), Some(json!(3)));
    let reason = &snapshot.ignored()["connectors.a.limit"];
    assert!(reason.contains("from 1 to 10"), "{reason}");
    assert!(snapshot.parked().is_empty());
}

#[tokio::test]
async fn invalid_changes_are_rejected_by_the_validator() {
    let storage = Arc::new(MemoryStorage::new());
    let store = store_with(Arc::clone(&storage)).await;
    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();

    let error = store
        .set("connectors.a.limit", json!(0), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, SettingsError::InvalidValue { reason, .. } if reason.contains("from 1 to 10")),
        "{error}"
    );
    assert!(storage.load().await.unwrap().is_empty());
    assert_eq!(store.current().value("connectors.a.limit"), Some(json!(3)));

    // Keys a section does not declare cannot be set.
    assert!(matches!(
        store.set("connectors.a.other", json!(1), None).await,
        Err(SettingsError::UnknownKey(_))
    ));
}

#[tokio::test]
async fn dynamic_keys_never_reach_the_host_config() {
    let store = store_with(Arc::default()).await;
    store
        .register_section(connector("a", json!({ "limit": 3, "extra": "kept" })))
        .await
        .unwrap();
    store
        .set("connectors.a.limit", json!(4), None)
        .await
        .unwrap();

    // `Config` denies unknown fields: it would fail to extract with the
    // connector's keys in it.
    let snapshot = store.current();
    assert_eq!(snapshot.config.greeting, "hi");
    assert_eq!(snapshot.value("connectors.a.extra"), Some(json!("kept")));
    store.set("greeting", json!("hello"), None).await.unwrap();
}

#[tokio::test]
async fn re_registering_unchanged_keeps_positions() {
    let store = store_with(Arc::default()).await;
    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();
    store
        .set("connectors.a.limit", json!(7), None)
        .await
        .unwrap();
    let before = position(&store, "connectors.a.limit");

    // A new base (e.g. the connector restarted), same settings.
    store
        .register_section(connector("a", json!({ "limit": 2 })))
        .await
        .unwrap();
    assert_eq!(position(&store, "connectors.a.limit"), before);
    assert_eq!(store.catalog().entries().len(), before + 1);
    assert_eq!(
        store.current().value("connectors.a.limit"),
        Some(json!(7)),
        "the override survives"
    );
}

#[tokio::test]
async fn re_registering_changed_moves_positions() {
    let store = store_with(Arc::default()).await;
    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();
    let before = position(&store, "connectors.a.limit");

    let mut changed = connector("a", json!({ "limit": 3 }));
    changed.settings[0].description = "Something else".to_string().into();
    store.register_section(changed).await.unwrap();

    let catalog = store.catalog();
    assert!(catalog.entry(before).is_none(), "the old position is gone");
    let after = position(&store, "connectors.a.limit");
    assert!(after > before);
    assert_eq!(
        &*catalog.entry(after).unwrap().description,
        "Something else"
    );
}

#[tokio::test]
async fn paths_must_be_inside_a_dynamic_namespace() {
    let store = store_with(Arc::default()).await;
    let rejected = |section: DynamicSection| {
        let store = &store;
        async move {
            match store.register_section(section).await {
                Err(SettingsError::InvalidValue { reason, .. }) => reason,
                other => panic!("registered: {other:?}"),
            }
        }
    };

    let mut outside = connector("a", json!({}));
    outside.path = "elsewhere.a".into();
    assert!(rejected(outside).await.contains("`connectors`"));

    let mut namespace = connector("a", json!({}));
    namespace.path = "connectors".into();
    assert!(rejected(namespace).await.contains("must be below"));

    // Not over a static section, nor over another dynamic one.
    let mut fixed = connector("a", json!({}));
    fixed.path = "connectors.fixed.a".into();
    assert!(rejected(fixed).await.contains("connectors.fixed"));

    store
        .register_section(connector("a", json!({})))
        .await
        .unwrap();
    let mut nested = connector("b", json!({}));
    nested.path = "connectors.a.b".into();
    assert!(rejected(nested).await.contains("connectors.a"));

    // Settings must be in the section, and the base must be valid.
    let mut undeclared = connector("c", json!({}));
    undeclared.settings[0].key = "missing".to_string().into();
    assert!(rejected(undeclared).await.contains("missing"));
    assert!(
        rejected(connector("d", json!({ "limit": 0 })))
            .await
            .contains("base")
    );
}

#[tokio::test]
async fn removed_sections_park_their_overrides() {
    let storage = Arc::new(MemoryStorage::new());
    let store = store_with(Arc::clone(&storage)).await;
    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();
    store
        .set("connectors.a.limit", json!(7), None)
        .await
        .unwrap();
    let position = position(&store, "connectors.a.limit");

    let change = store.remove_section("a").await.unwrap();
    assert!(change.is_some());
    let snapshot = store.current();
    assert_eq!(snapshot.value("connectors.a.limit"), None);
    assert_eq!(snapshot.parked().get("connectors.a.limit"), Some(&json!(7)));
    assert!(store.catalog().entry(position).is_none());
    assert_eq!(storage.load().await.unwrap().len(), 1, "still stored");
    assert!(store.remove_section("a").await.unwrap().is_none());

    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();
    assert_eq!(store.current().value("connectors.a.limit"), Some(json!(7)));
}

#[tokio::test]
async fn unset_clears_parked_overrides() {
    let storage = Arc::new(MemoryStorage::new());
    store_raw(&storage, "connectors.a.limit", json!(7)).await;
    let store = store_with(Arc::clone(&storage)).await;
    assert_eq!(store.current().parked().len(), 1);

    assert!(store.unset("connectors.a").await.unwrap().is_some());
    assert!(store.current().parked().is_empty());
    assert!(storage.load().await.unwrap().is_empty());

    store
        .register_section(connector("a", json!({ "limit": 3 })))
        .await
        .unwrap();
    assert_eq!(store.current().value("connectors.a.limit"), Some(json!(3)));
}
