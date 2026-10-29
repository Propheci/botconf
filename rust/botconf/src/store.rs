use std::{collections::BTreeMap, ops::Range, sync::Arc};

use arc_swap::ArcSwap;
use figment::Figment;
use serde_json::Value;
use tokio::sync::{Mutex, watch};

use crate::{
    Schema,
    dynamic::DynamicSection,
    keys::{Catalog, CatalogEntry, UnknownKey, is_below},
    provider::Override,
    section::Section,
    shared::Shared,
    snapshot::{Change, Snapshot},
    storage::{Storage, StorageError, StoredOverride},
};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    UnknownKey(#[from] UnknownKey),

    #[error("invalid value for `{key}`: {reason}")]
    InvalidValue { key: String, reason: String },

    #[error("`{0}` is not a list")]
    NotAList(String),

    #[error("`{0}` is not an entry of a map setting")]
    NotAnEntry(String),

    #[error("the configuration without runtime overrides is invalid: {0}")]
    InvalidBase(#[source] BuildError),

    #[error("the settings storage failed")]
    Storage(#[source] StorageError),
}

/// Why a configuration is invalid.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(transparent)]
    Config(#[from] Box<figment::Error>),

    #[error("invalid `{path}` section: {reason}")]
    Section { path: String, reason: String },

    #[error("{0}")]
    Invalid(String),
}

/// Owns the effective configuration and every change to it.
///
/// The effective configuration is built by layering, from the lowest to the
/// highest priority, the `base` sources (e.g. a config file, then the
/// environment) and the stored overrides of the keys of the [`Catalog`].
/// Every change goes through the same deserialization and validation as the
/// base, then atomically replaces the [`Snapshot`] that readers get from
/// [`Self::current`], so it applies immediately.
///
/// Sections can also be registered while the program runs
/// ([`Self::register_section`]), below the schema's
/// [dynamic namespaces](Schema::dynamic_namespaces).
pub struct SettingsStore<S: Schema> {
    schema: Arc<S>,
    sections: Arc<[Section]>,
    namespaces: Vec<&'static str>,
    base: Figment,
    storage: Box<dyn Storage>,
    /// Swapped, under the write lock, when sections are registered or
    /// removed; it only ever grows (see [`Catalog`]).
    catalog: ArcSwap<Catalog>,
    current: ArcSwap<Snapshot<S>>,
    /// Sent every published snapshot, for [`Self::subscribe`].
    published: watch::Sender<Arc<Snapshot<S>>>,
    /// Serialises the read-modify-write cycles of the changes, and holds the
    /// sections registered at runtime.
    write_lock: Mutex<Registry>,
}

/// The sections registered at runtime.
#[derive(Default)]
struct Registry {
    sections: Vec<Registered>,
}

struct Registered {
    section: Arc<DynamicSection>,
    /// Its settings' positions in the catalog.
    positions: Range<usize>,
}

impl Registry {
    fn sections(&self) -> Vec<Arc<DynamicSection>> {
        self.sections
            .iter()
            .map(|registered| Arc::clone(&registered.section))
            .collect()
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.sections
            .iter()
            .position(|registered| registered.section.id == id)
    }
}

impl<S: Schema> std::fmt::Debug for SettingsStore<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsStore")
            .field("catalog", &self.catalog.load())
            .field("current", &self.current.load())
            .finish_non_exhaustive()
    }
}

impl<S: Schema> SettingsStore<S> {
    /// Layers the stored overrides on top of `base`.
    ///
    /// Stored overrides that are invalid are skipped with a warning rather
    /// than failing, which would make them impossible to fix at runtime.
    /// Those below a dynamic namespace wait for their section to register.
    pub async fn load(
        schema: S,
        base: Figment,
        storage: impl Storage,
    ) -> Result<Self, SettingsError> {
        let schema = Arc::new(schema);
        let sections: Arc<[Section]> = schema.sections().into();
        let namespaces = schema.dynamic_namespaces();
        let catalog = Catalog::new(&schema.settings(), &sections);
        let storage: Box<dyn Storage> = Box::new(storage);

        let layers = Layers {
            schema: &schema,
            sections: &sections,
            dynamic: &[],
            base: &base,
        };
        let snapshot = load_snapshot(&layers, storage.as_ref(), &catalog, &namespaces).await?;
        schema.on_change(None, &snapshot);
        let snapshot = Arc::new(snapshot);

        Ok(Self {
            schema,
            sections,
            namespaces,
            base,
            storage,
            catalog: ArcSwap::from_pointee(catalog),
            current: ArcSwap::new(Arc::clone(&snapshot)),
            published: watch::Sender::new(snapshot),
            write_lock: Mutex::new(Registry::default()),
        })
    }

    /// The effective configuration. Cheap; hold on to it only for the
    /// duration of one operation so that changes are picked up.
    pub fn current(&self) -> Arc<Snapshot<S>> {
        self.current.load_full()
    }

    /// A receiver that sees every published snapshot: it holds the latest,
    /// and [`changed`](watch::Receiver::changed) resolves when a newer one
    /// is published (a reader that falls behind skips to the latest).
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot<S>>> {
        self.published.subscribe()
    }

    /// The keys that can be changed at runtime. Cheap; like
    /// [`Self::current`], hold on to it only for one operation.
    pub fn catalog(&self) -> Arc<Catalog> {
        self.catalog.load_full()
    }

    pub fn schema(&self) -> &S {
        &self.schema
    }

    /// The sections the schema declared (not those registered at runtime).
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The sections registered at runtime, in the order they registered.
    /// Waits for a change in progress to finish.
    pub async fn dynamic_sections(&self) -> Vec<Arc<DynamicSection>> {
        self.write_lock.lock().await.sections()
    }

    /// Rebuilds the configuration from the base sources and the storage,
    /// e.g. after another process changed the stored overrides.
    pub async fn reload(&self) -> Result<Change<S>, SettingsError> {
        let registry = self.write_lock.lock().await;
        let dynamic = registry.sections();
        let snapshot = load_snapshot(
            &self.layers(&dynamic),
            self.storage.as_ref(),
            &self.catalog.load(),
            &self.namespaces,
        )
        .await?;
        Ok(self.publish(snapshot))
    }

    /// Adds a section, or replaces the one with the same id. Its stored
    /// overrides (those that waited for it, or those of the section it
    /// replaces) are applied one at a time; invalid ones are ignored with the
    /// reason, as at load.
    ///
    /// The section's path must be below one of the schema's dynamic
    /// namespaces and overlap no other section, its base must be valid, and
    /// its runtime keys must be in its (validated) base.
    pub async fn register_section(
        &self,
        section: DynamicSection,
    ) -> Result<Change<S>, SettingsError> {
        let mut registry = self.write_lock.lock().await;
        let path = section.path.clone();
        let invalid = |reason: String| SettingsError::InvalidValue {
            key: path.clone(),
            reason,
        };
        self.check_registration(&registry, &section)
            .map_err(&invalid)?;

        // Unchanged settings keep their positions; changed ones move.
        let entries: Vec<_> = section
            .settings
            .iter()
            .map(|setting| {
                CatalogEntry::new(
                    format!("{}.{}", section.path, setting.key),
                    setting,
                    Some(Shared::from(section.id.clone())),
                )
            })
            .collect();
        let replaced = registry.position(&section.id);
        let mut catalog = Catalog::clone(&self.catalog.load());
        let positions = match replaced.map(|index| &registry.sections[index]) {
            Some(old)
                if catalog.entries()[old.positions.clone()]
                    .iter()
                    .map(Option::as_ref)
                    .eq(entries.iter().map(Some)) =>
            {
                old.positions.clone()
            }
            Some(old) => {
                catalog.tombstone(old.positions.clone());
                catalog.append(entries)
            }
            None => catalog.append(entries),
        };

        let section = Arc::new(section);
        let mut dynamic: Vec<_> = registry
            .sections()
            .into_iter()
            .filter(|other| other.id != section.id)
            .collect();
        dynamic.push(Arc::clone(&section));

        // The replaced section's overrides wait like the others, then every
        // override waiting for this section is applied one at a time.
        let previous = self.current();
        let mut overrides = previous.overrides.clone();
        let mut parked = previous.parked.clone();
        if let Some(index) = replaced {
            park_below(
                &mut overrides,
                &mut parked,
                &registry.sections[index].section.path,
            );
        }
        let waiting_keys: Vec<_> = parked
            .keys()
            .filter(|key| at_or_below(key, &section.path))
            .cloned()
            .collect();
        let waiting: Vec<_> = waiting_keys
            .into_iter()
            .filter_map(|key| parked.remove_entry(&key))
            .collect();

        let layers = self.layers(&dynamic);
        let mut snapshot = build(&layers, overrides, parked, previous.ignored.clone())
            .map_err(|error| invalid(error.to_string()))?;
        for (key, value) in waiting {
            snapshot = apply_stored(&layers, &catalog, snapshot, key, value);
        }

        let registered = Registered {
            section: Arc::clone(&section),
            positions,
        };
        match replaced {
            Some(index) => registry.sections[index] = registered,
            None => registry.sections.push(registered),
        }
        self.catalog.store(Arc::new(catalog));
        tracing::info!(id = section.id, path = section.path, "section registered");
        Ok(self.publish(snapshot))
    }

    /// Removes a section registered at runtime. Its stored overrides stay,
    /// waiting for it to register again. Returns `None` if there is no such
    /// section.
    pub async fn remove_section(&self, id: &str) -> Result<Option<Change<S>>, SettingsError> {
        let mut registry = self.write_lock.lock().await;
        let Some(index) = registry.position(id) else {
            return Ok(None);
        };
        let removed = &registry.sections[index];
        let dynamic: Vec<_> = registry
            .sections()
            .into_iter()
            .filter(|other| other.id != id)
            .collect();

        let previous = self.current();
        let mut overrides = previous.overrides.clone();
        let mut parked = previous.parked.clone();
        park_below(&mut overrides, &mut parked, &removed.section.path);
        let snapshot = build(
            &self.layers(&dynamic),
            overrides,
            parked,
            previous.ignored.clone(),
        )
        .map_err(|error| SettingsError::InvalidValue {
            key: removed.section.path.clone(),
            reason: error.to_string(),
        })?;

        let mut catalog = Catalog::clone(&self.catalog.load());
        catalog.tombstone(removed.positions.clone());
        let removed = registry.sections.remove(index);
        self.catalog.store(Arc::new(catalog));
        tracing::info!(id, path = removed.section.path, "section removed");
        Ok(Some(self.publish(snapshot)))
    }

    /// Overrides `key` with `value`, replacing the overrides of its entries.
    /// `by` records who changed it.
    pub async fn set(
        &self,
        key: &str,
        value: Value,
        by: Option<String>,
    ) -> Result<Change<S>, SettingsError> {
        self.modify(key, by, |_| Ok(value)).await
    }

    /// Appends `item` to the list at `key`, unless it is already there.
    pub async fn add(
        &self,
        key: &str,
        item: Value,
        by: Option<String>,
    ) -> Result<Change<S>, SettingsError> {
        self.extend(key, vec![item], by).await
    }

    /// Appends the `items` that are not there yet to the list at `key`, in a
    /// single change.
    pub async fn extend(
        &self,
        key: &str,
        new_items: Vec<Value>,
        by: Option<String>,
    ) -> Result<Change<S>, SettingsError> {
        self.modify(key, by, |current| {
            let mut items = as_list(key, current)?;
            for item in new_items {
                if !items.contains(&item) {
                    items.push(item);
                }
            }
            Ok(Value::Array(items))
        })
        .await
    }

    /// Removes every occurrence of `item` from the list at `key`.
    pub async fn remove(
        &self,
        key: &str,
        item: Value,
        by: Option<String>,
    ) -> Result<Change<S>, SettingsError> {
        self.modify(key, by, |current| {
            let mut items = as_list(key, current)?;
            items.retain(|existing| existing != &item);
            Ok(Value::Array(items))
        })
        .await
    }

    /// Removes the entry `<map>.<name>` from its map-valued setting, wherever
    /// it is defined: this overrides the whole map.
    pub async fn delete_entry(
        &self,
        entry_key: &str,
        by: Option<String>,
    ) -> Result<Change<S>, SettingsError> {
        let catalog = self.catalog();
        let setting = catalog.resolve(entry_key)?;
        let Some(name) = setting.entry_of(entry_key) else {
            return Err(SettingsError::NotAnEntry(entry_key.to_string()));
        };
        let map_key = setting.key.clone();

        self.modify(&map_key, by, |current| {
            let mut entries = match current {
                None | Some(Value::Null) => Default::default(),
                Some(Value::Object(entries)) => entries,
                Some(_) => return Err(SettingsError::NotAnEntry(entry_key.to_string())),
            };
            entries.remove(name);
            Ok(Value::Object(entries))
        })
        .await
    }

    /// Removes the overrides of `key` and of its entries, falling back to the
    /// base values. Returns `None` if nothing was overridden.
    pub async fn unset(&self, key: &str) -> Result<Option<Change<S>>, SettingsError> {
        let registry = self.write_lock.lock().await;
        let previous = self.current();

        // Look at the storage rather than at the snapshot, so that ignored
        // and waiting overrides can be cleaned up too.
        let stored: Vec<_> = self
            .storage
            .load()
            .await
            .map_err(SettingsError::Storage)?
            .into_iter()
            .map(|stored| stored.key)
            .filter(|stored| at_or_below(stored, key))
            .collect();
        if stored.is_empty() {
            return Ok(None);
        }

        let mut overrides = previous.overrides.clone();
        let mut parked = previous.parked.clone();
        let mut ignored = previous.ignored.clone();
        for key in &stored {
            overrides.remove(key);
            parked.remove(key);
            ignored.remove(key);
        }

        let snapshot = build(
            &self.layers(&registry.sections()),
            overrides,
            parked,
            ignored,
        )
        .map_err(|error| SettingsError::InvalidValue {
            key: key.to_string(),
            reason: error.to_string(),
        })?;

        self.storage
            .write(&stored, None)
            .await
            .map_err(SettingsError::Storage)?;
        tracing::info!(key, "runtime setting removed");
        Ok(Some(self.publish(snapshot)))
    }

    async fn modify(
        &self,
        key: &str,
        by: Option<String>,
        new_value: impl FnOnce(Option<Value>) -> Result<Value, SettingsError>,
    ) -> Result<Change<S>, SettingsError> {
        let registry = self.write_lock.lock().await;
        self.catalog().resolve(key)?;
        let previous = self.current();
        let value = new_value(previous.value(key))?;

        // Setting a key replaces the overrides of its entries.
        let replaced: Vec<_> = previous
            .overrides
            .keys()
            .chain(previous.ignored.keys())
            .filter(|existing| is_below(existing, key))
            .cloned()
            .collect();

        let mut overrides = previous.overrides.clone();
        let mut ignored = previous.ignored.clone();
        for existing in &replaced {
            overrides.remove(existing);
            ignored.remove(existing);
        }
        ignored.remove(key);
        overrides.insert(key.to_string(), value.clone());

        let snapshot = build(
            &self.layers(&registry.sections()),
            overrides,
            previous.parked.clone(),
            ignored,
        )
        .map_err(|error| SettingsError::InvalidValue {
            key: key.to_string(),
            reason: error.to_string(),
        })?;

        let stored = StoredOverride {
            key: key.to_string(),
            value: value.to_string(),
            by,
        };
        self.storage
            .write(&replaced, Some(&stored))
            .await
            .map_err(SettingsError::Storage)?;

        tracing::info!(key, %value, by = ?stored.by, "runtime setting changed");
        Ok(self.publish(snapshot))
    }

    /// Why `section` cannot be registered, if it cannot.
    fn check_registration(
        &self,
        registry: &Registry,
        section: &DynamicSection,
    ) -> Result<(), String> {
        let path = &section.path;
        if !self
            .namespaces
            .iter()
            .any(|namespace| is_below(path, namespace))
        {
            let namespaces = match self.namespaces.as_slice() {
                [] => "a dynamic namespace, and the schema declares none".to_string(),
                namespaces => namespaces
                    .iter()
                    .map(|namespace| format!("`{namespace}`"))
                    .collect::<Vec<_>>()
                    .join(" or "),
            };
            return Err(format!(
                "a section registered at runtime must be below {namespaces}"
            ));
        }
        for other in self.sections.iter() {
            if other.id == section.id {
                return Err(format!("the id `{}` is taken by a section", section.id));
            }
            if overlap(path, &other.path) {
                return Err(format!("it overlaps the `{}` section", other.path));
            }
        }
        if let Some(other) = registry
            .sections
            .iter()
            .find(|other| other.section.id != section.id && overlap(path, &other.section.path))
        {
            return Err(format!("it overlaps the `{}` section", other.section.path));
        }

        if !section.base.is_object() {
            return Err("its base must be an object".into());
        }
        let validated = section
            .validator
            .validate(&section.base)
            .map_err(|reason| format!("its base is invalid: {reason}"))?;
        if let Some(setting) = section
            .settings
            .iter()
            .find(|setting| validated.pointer(&to_pointer(&setting.key)).is_none())
        {
            return Err(format!(
                "its runtime setting `{}` is not in the section",
                setting.key
            ));
        }
        Ok(())
    }

    fn layers<'a>(&'a self, dynamic: &'a [Arc<DynamicSection>]) -> Layers<'a, S> {
        Layers {
            schema: &self.schema,
            sections: &self.sections,
            dynamic,
            base: &self.base,
        }
    }

    fn publish(&self, snapshot: Snapshot<S>) -> Change<S> {
        let current = Arc::new(snapshot);
        let previous = self.current.swap(Arc::clone(&current));
        self.schema.on_change(Some(&previous), &current);
        self.published.send_replace(Arc::clone(&current));
        Change { previous, current }
    }
}

/// Parses a value typed by a user: JSON if it is valid JSON (`42`, `[1, 2]`,
/// `"text"`, `true`), a plain string otherwise (`info,sqlx=warn`).
pub fn parse_value(raw: &str) -> Value {
    let raw = raw.trim();
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn as_list(key: &str, value: Option<Value>) -> Result<Vec<Value>, SettingsError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(SettingsError::NotAList(key.to_string())),
    }
}

fn at_or_below(key: &str, parent: &str) -> bool {
    key == parent || is_below(key, parent)
}

/// Whether two paths share keys.
fn overlap(a: &str, b: &str) -> bool {
    at_or_below(a, b) || is_below(b, a)
}

/// Moves the overrides at or below `path` to the waiting ones.
fn park_below(
    overrides: &mut BTreeMap<String, Value>,
    parked: &mut BTreeMap<String, Value>,
    path: &str,
) {
    let keys: Vec<_> = overrides
        .keys()
        .filter(|key| at_or_below(key, path))
        .cloned()
        .collect();
    for key in keys {
        if let Some(value) = overrides.remove(&key) {
            parked.insert(key, value);
        }
    }
}

/// `a.b` -> `/a/b`.
fn to_pointer(key: &str) -> String {
    format!("/{}", key.replace('.', "/"))
}

/// What a configuration is built from, besides the overrides.
struct Layers<'a, S: Schema> {
    schema: &'a Arc<S>,
    sections: &'a [Section],
    dynamic: &'a [Arc<DynamicSection>],
    base: &'a Figment,
}

/// The base configuration with every valid stored override applied, one at a
/// time so that an invalid one is skipped without affecting the others.
/// Overrides below a dynamic namespace that no registered section takes wait
/// for their section.
async fn load_snapshot<S: Schema>(
    layers: &Layers<'_, S>,
    storage: &dyn Storage,
    catalog: &Catalog,
    namespaces: &[&'static str],
) -> Result<Snapshot<S>, SettingsError> {
    let mut snapshot = build(layers, BTreeMap::new(), BTreeMap::new(), BTreeMap::new())
        .map_err(SettingsError::InvalidBase)?;

    let mut stored = storage.load().await.map_err(SettingsError::Storage)?;
    stored.sort_by(|a, b| a.key.cmp(&b.key));
    for row in stored {
        let value = match serde_json::from_str::<Value>(&row.value) {
            Ok(value) => value,
            Err(error) => {
                let reason = format!("the stored value is not valid JSON: {error}");
                tracing::warn!(key = row.key, reason, "ignoring an invalid runtime setting");
                snapshot.ignored.insert(row.key, reason);
                continue;
            }
        };
        let waits = catalog.resolve(&row.key).is_err()
            && namespaces
                .iter()
                .any(|namespace| is_below(&row.key, namespace));
        if waits {
            snapshot.parked.insert(row.key, value);
            continue;
        }
        snapshot = apply_stored(layers, catalog, snapshot, row.key, value);
    }

    tracing::info!(
        overrides = snapshot.overrides.len(),
        waiting = snapshot.parked.len(),
        ignored = snapshot.ignored.len(),
        "runtime settings loaded"
    );
    Ok(snapshot)
}

/// `snapshot` with the stored override `key` applied, or, when it is
/// invalid, listed as ignored with the reason.
fn apply_stored<S: Schema>(
    layers: &Layers<'_, S>,
    catalog: &Catalog,
    mut snapshot: Snapshot<S>,
    key: String,
    value: Value,
) -> Snapshot<S> {
    let reason = match catalog.resolve(&key) {
        Err(error) => error.to_string(),
        Ok(_) => {
            let mut overrides = snapshot.overrides.clone();
            overrides.insert(key.clone(), value);
            match build(
                layers,
                overrides,
                snapshot.parked.clone(),
                snapshot.ignored.clone(),
            ) {
                Ok(next) => return next,
                Err(error) => error.to_string(),
            }
        }
    };

    tracing::warn!(key, reason, "ignoring an invalid runtime setting");
    snapshot.ignored.insert(key, reason);
    snapshot
}

/// Merges `overrides` (dotted keys) into `figment`.
pub(crate) fn layer<'a>(
    figment: Figment,
    overrides: impl IntoIterator<Item = (&'a str, &'a Value)>,
) -> Figment {
    // Parents first, so that entry overrides win over whole-map overrides.
    let mut ordered: Vec<_> = overrides.into_iter().collect();
    ordered.sort_by_key(|(key, _)| key.matches('.').count());

    // Figment merges maps key by key, which would mix an overriding map with
    // the base's. Overwriting the key with a scalar first makes the override
    // replace it instead.
    let cleared = Value::Bool(false);
    ordered.into_iter().fold(figment, |figment, (key, value)| {
        let figment = if value.is_object() {
            figment.merge(Override::new(key, &cleared))
        } else {
            figment
        };
        figment.merge(Override::new(key, value))
    })
}

/// Layers the overrides on the base and validates the result. The overrides
/// of sections registered at runtime are layered on their own base, never on
/// the host's configuration.
fn build<S: Schema>(
    layers: &Layers<'_, S>,
    overrides: BTreeMap<String, Value>,
    parked: BTreeMap<String, Value>,
    ignored: BTreeMap<String, String>,
) -> Result<Snapshot<S>, BuildError> {
    let owned_by_dynamic = |key: &str| {
        layers
            .dynamic
            .iter()
            .any(|section| at_or_below(key, &section.path))
    };
    let figment = layer(
        layers.base.clone(),
        overrides
            .iter()
            .filter(|(key, _)| !owned_by_dynamic(key))
            .map(|(key, value)| (key.as_str(), value)),
    );

    let config: S::Config = figment.extract().map_err(Box::new)?;

    let mut parsed = BTreeMap::new();
    for section in layers.sections {
        let raw = figment
            .find_value(&section.path)
            .ok()
            .map(|value| {
                value
                    .deserialize::<Value>()
                    .map_err(|error| BuildError::Section {
                        path: section.path.clone(),
                        reason: error.to_string(),
                    })
            })
            .transpose()?;
        let section_settings =
            section
                .settings
                .parse(raw.as_ref())
                .map_err(|reason| BuildError::Section {
                    path: section.path.clone(),
                    reason,
                })?;
        parsed.insert(section.id, (section.path.clone(), section_settings));
    }

    let mut dynamic = BTreeMap::new();
    for section in layers.dynamic {
        let own = overrides.iter().filter_map(|(key, value)| {
            let relative = key.strip_prefix(section.path.as_str())?.strip_prefix('.')?;
            Some((relative, value))
        });
        dynamic.insert(section.id.clone(), section.build(own)?);
    }

    let derived = layers.schema.derive(&config).map_err(BuildError::Invalid)?;

    Ok(Snapshot {
        config,
        derived,
        schema: Arc::clone(layers.schema),
        sections: parsed,
        dynamic,
        overrides,
        parked,
        ignored,
        figment,
    })
}
