//! Back up the settings and restore them: one TOML file holding the
//! effective configuration (the base, file and environment, with every
//! stored override applied) written like the config file, then a reserved
//! `[backup]` table with the overrides under the host's dynamic namespaces
//! (which its own file must not hold) and, only when asked, the values of the
//! environment variables its options name.
//!
//! ```toml
//! listen = "127.0.0.1:8080"
//! # … the config file, every override applied …
//!
//! [backup]
//! format = 1
//! secrets = false
//!
//! [backup.overrides]
//! "connector_settings.telegram-main.log_level" = "debug"
//!
//! [backup.env_file]            # only with secrets
//! MYBOT_TOKEN = "…"
//! ```
//!
//! [`export`] writes it from a loaded store's [`Snapshot`], so the layering is
//! the store's own (offline, the overrides of sections registered at runtime
//! wait as parked ones, and are exported the same). [`import`] checks a
//! backup with the host's own loader before writing anything, keeps `.bak`
//! copies of the files it replaces, then writes the config file, the env
//! file's variables and the stored overrides.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::describe::{Description, Node, Table, ValueKind, toml_value};
use crate::env_file::EnvFile;
use crate::keys::is_below;
use crate::storage::{Storage, StorageError, StoredOverride};
use crate::{Schema, Snapshot};

/// The reserved top-level table of a backup.
pub const BACKUP_TABLE: &str = "backup";

/// The backup format this release writes and reads.
pub const FORMAT: i64 = 1;

/// A variable's value, for an export with secrets: from the env file, then the
/// process environment, say.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Who an imported override is recorded as changed by.
pub const IMPORTED_BY: &str = "import";

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("the configuration has its own `{BACKUP_TABLE}` key, which a backup reserves")]
    Reserved,
    #[error("the effective configuration could not be read: {0}")]
    Effective(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not a backup: {0}")]
    Invalid(String),
    #[error("a backup of format {found}, which this release does not read (it reads {FORMAT})")]
    Format { found: i64 },
    #[error("the backup does not load: {0}")]
    Rejected(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the files are written, but the stored overrides are not: {0}")]
    Storage(StorageError),
}

/// Writes the backup of `snapshot`. The overrides kept as overrides are those
/// under the schema's dynamic namespaces (e.g. `connector_settings`). With
/// `secrets` (a variable's value, from the env file before the process
/// environment) the backup holds the values of the variables its options name,
/// and the secrets of the file itself; without, it names the variables only,
/// and a secret option is left out (commented).
pub fn export<S: Schema>(
    description: &Description,
    snapshot: &Snapshot<S>,
    secrets: Option<Lookup<'_>>,
) -> Result<String, ExportError> {
    if description
        .table
        .entries
        .iter()
        .any(|entry| entry.key == BACKUP_TABLE)
    {
        return Err(ExportError::Reserved);
    }
    let namespaces = snapshot.schema.dynamic_namespaces();
    let mut config = snapshot
        .figment
        .extract::<Value>()
        .map_err(|error| ExportError::Effective(error.to_string()))?;
    let Value::Object(values) = &mut config else {
        return Err(ExportError::Effective("it is not a table".into()));
    };
    for namespace in &namespaces {
        remove_at(values, namespace);
    }
    values.remove(BACKUP_TABLE);

    let mut env = BTreeMap::new();
    match secrets {
        Some(lookup) => {
            for name in env_vars(&description.table, values) {
                if let Some(value) = lookup(&name) {
                    env.insert(name, value);
                }
            }
        }
        None => each_value(&description.table, values, &mut |kind, value| {
            if matches!(kind, ValueKind::Secret) {
                *value = Value::Null;
            }
        }),
    }

    let below = |key: &str| namespaces.iter().any(|namespace| is_below(key, namespace));
    let overrides: BTreeMap<&String, &Value> = snapshot
        .overrides
        .iter()
        .chain(&snapshot.parked)
        .filter(|(key, _)| below(key))
        .collect();

    let mut out = description.render(&config);
    out.push_str("\n# The backup's own table: the overrides of what registers at runtime");
    if secrets.is_none() {
        out.push_str(",\n# and no secrets (the env file's values are not included)");
    }
    out.push_str(".\n");
    let _ = writeln!(out, "[{BACKUP_TABLE}]");
    let _ = writeln!(out, "format = {FORMAT}");
    let _ = writeln!(out, "secrets = {}", secrets.is_some());
    let _ = writeln!(out, "\n[{BACKUP_TABLE}.overrides]");
    for (key, value) in overrides {
        let _ = writeln!(out, "{} = {}", quoted(key), toml_value(value));
    }
    if secrets.is_some() {
        let _ = writeln!(out, "\n[{BACKUP_TABLE}.env_file]");
        for (name, value) in &env {
            let _ = writeln!(
                out,
                "{} = {}",
                quoted(name),
                toml_value(&Value::String(value.clone()))
            );
        }
    }
    Ok(out)
}

/// A backup, read.
#[derive(Clone, Debug, PartialEq)]
pub struct Backup {
    /// The configuration, as the file holds it.
    pub config: Value,
    /// The overrides under the dynamic namespaces, by dotted key.
    pub overrides: BTreeMap<String, Value>,
    /// The env file's variables, when the backup has secrets.
    pub env: BTreeMap<String, String>,
    /// Whether it was exported with secrets.
    pub secrets: bool,
}

impl Backup {
    /// Reads a backup's text.
    pub fn parse(text: &str) -> Result<Self, ImportError> {
        let parsed: toml::Table =
            toml::from_str(text).map_err(|error| ImportError::Invalid(error.to_string()))?;
        let Value::Object(mut config) = serde_json::to_value(parsed)
            .map_err(|error| ImportError::Invalid(error.to_string()))?
        else {
            return Err(ImportError::Invalid("it is not a table".into()));
        };
        let Some(Value::Object(mut backup)) = config.remove(BACKUP_TABLE) else {
            return Err(ImportError::Invalid(format!(
                "it has no [{BACKUP_TABLE}] table (a config file is not a backup)"
            )));
        };
        match backup.get("format").and_then(Value::as_i64) {
            Some(FORMAT) => {}
            Some(found) => return Err(ImportError::Format { found }),
            None => {
                return Err(ImportError::Invalid(format!(
                    "[{BACKUP_TABLE}] has no format"
                )));
            }
        }
        let secrets = backup
            .get("secrets")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let overrides = match backup.remove("overrides") {
            Some(Value::Object(overrides)) => overrides.into_iter().collect(),
            None => BTreeMap::new(),
            Some(_) => {
                return Err(ImportError::Invalid(format!(
                    "[{BACKUP_TABLE}.overrides] is not a table"
                )));
            }
        };
        let env = match backup.remove("env_file") {
            Some(Value::Object(env)) => env
                .into_iter()
                .map(|(name, value)| match value {
                    Value::String(value) => Ok((name, value)),
                    other => Err(ImportError::Invalid(format!(
                        "env_file.{name} is not text: {other}"
                    ))),
                })
                .collect::<Result<_, _>>()?,
            None => BTreeMap::new(),
            Some(_) => {
                return Err(ImportError::Invalid(format!(
                    "[{BACKUP_TABLE}.env_file] is not a table"
                )));
            }
        };
        Ok(Self {
            config: Value::Object(config),
            overrides,
            env,
            secrets,
        })
    }

    /// The config file it restores, written like the example.
    pub fn config_file(&self, description: &Description) -> String {
        description.render(&self.config)
    }
}

/// Where an import writes.
pub struct ImportTarget<'a> {
    pub config: &'a Path,
    pub env_file: &'a Path,
    pub storage: &'a dyn Storage,
    /// The host's dynamic namespaces: their stored overrides are replaced by
    /// the backup's; every other stored override is removed (the file holds
    /// it now).
    pub namespaces: &'a [&'a str],
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    /// The `.bak` copies of the files it replaced.
    pub kept: Vec<PathBuf>,
    /// Stored overrides removed (now in the file, or replaced).
    pub removed: usize,
    /// Overrides stored from the backup.
    pub stored: usize,
    /// Env file variables set.
    pub env_vars: usize,
}

/// Restores `backup`: checks the config file it writes with `validate` (the
/// host's loader, given the file and the env file's variables as they would
/// be), then keeps `.bak` copies of the config file and the env file, writes
/// them, and replaces the stored overrides. Nothing is written when the
/// check fails.
pub async fn import(
    description: &Description,
    backup: &Backup,
    target: &ImportTarget<'_>,
    validate: impl FnOnce(&str, &BTreeMap<String, String>) -> Result<(), String>,
) -> Result<Imported, ImportError> {
    // A backup without secrets left the secret options out: the current
    // file's stay.
    let mut values = backup.config.clone();
    if !backup.secrets
        && let Ok(current) = toml::from_str::<toml::Table>(&read_optional(target.config)?)
        && let (Value::Object(values), Ok(Value::Object(current))) =
            (&mut values, serde_json::to_value(current))
    {
        keep_secrets(&description.table, values, &current);
    }
    let config = description.render(&values);
    let env_text = read_optional(target.env_file)?;
    let mut env = EnvFile::parse(&env_text);
    for (name, value) in &backup.env {
        env.set(name, value);
    }
    validate(&config, &env.vars()).map_err(ImportError::Rejected)?;
    let below = |key: &str| {
        target
            .namespaces
            .iter()
            .any(|namespace| is_below(key, namespace))
    };
    if let Some(key) = backup.overrides.keys().find(|key| !below(key)) {
        return Err(ImportError::Rejected(format!(
            "{key} is not below a namespace that registers at runtime ({})",
            target.namespaces.join(", ")
        )));
    }
    let stored = target.storage.load().await.map_err(ImportError::Storage)?;

    let mut imported = Imported::default();
    for path in [target.config, target.env_file] {
        if let Some(kept) = keep(path)? {
            imported.kept.push(kept);
        }
    }
    write(target.config, &config, false)?;
    if !backup.env.is_empty() {
        write(target.env_file, &env.render(), true)?;
        imported.env_vars = backup.env.len();
    }

    let deleted: Vec<String> = stored.into_iter().map(|row| row.key).collect();
    target
        .storage
        .write(&deleted, None)
        .await
        .map_err(ImportError::Storage)?;
    imported.removed = deleted.len();
    for (key, value) in &backup.overrides {
        let row = StoredOverride {
            key: key.clone(),
            value: value.to_string(),
            by: Some(IMPORTED_BY.into()),
        };
        target
            .storage
            .write(&[], Some(&row))
            .await
            .map_err(ImportError::Storage)?;
        imported.stored += 1;
    }
    Ok(imported)
}

/// Copies into `values` the secret options it lacks from `current`, in
/// tables (not in lists or maps, whose items a backup may have reordered).
fn keep_secrets(table: &Table, values: &mut Map<String, Value>, current: &Map<String, Value>) {
    for entry in &table.entries {
        match (&entry.node, current.get(&entry.key)) {
            (Node::Value(ValueKind::Secret), Some(secret)) => {
                if values.get(&entry.key).is_none_or(Value::is_null) {
                    values.insert(entry.key.clone(), secret.clone());
                }
            }
            (Node::Table(inner), Some(Value::Object(current))) => {
                if let Value::Object(values) = values
                    .entry(entry.key.clone())
                    .or_insert_with(|| Value::Object(Map::new()))
                {
                    keep_secrets(inner, values, current);
                }
            }
            _ => {}
        }
    }
}

/// A key as TOML writes it, quoted (a dotted key is one key here).
fn quoted(key: &str) -> String {
    toml_value(&Value::String(key.to_owned()))
}

/// Removes the value at the dotted `key`.
fn remove_at(values: &mut Map<String, Value>, key: &str) {
    match key.split_once('.') {
        None => {
            values.remove(key);
        }
        Some((first, rest)) => {
            if let Some(Value::Object(inner)) = values.get_mut(first) {
                remove_at(inner, rest);
            }
        }
    }
}

/// The variables the options name (the values of `EnvVar` options), in
/// order, once each.
fn env_vars(table: &Table, values: &mut Map<String, Value>) -> Vec<String> {
    let mut names = Vec::new();
    each_value(table, values, &mut |kind, value| {
        if let (ValueKind::EnvVar(_), Value::String(name)) = (kind, &*value)
            && !name.is_empty()
            && !names.contains(name)
        {
            names.push(name.clone());
        }
    });
    names
}

/// Calls `f` with every value the description knows in `values`, in tables,
/// lists, maps and tagged tables.
fn each_value(
    table: &Table,
    values: &mut Map<String, Value>,
    f: &mut impl FnMut(&ValueKind, &mut Value),
) {
    for entry in &table.entries {
        if let Some(value) = values.get_mut(&entry.key) {
            each_in(&entry.node, value, f);
        }
    }
}

fn each_in(node: &Node, value: &mut Value, f: &mut impl FnMut(&ValueKind, &mut Value)) {
    match (node, value) {
        (Node::Value(kind), value) => f(kind, value),
        (Node::Table(table), Value::Object(values)) => each_value(table, values, f),
        (Node::List(inner), Value::Array(items)) => {
            for item in items {
                each_in(inner, item, f);
            }
        }
        (Node::Map(inner), Value::Object(items)) => {
            for item in items.values_mut() {
                each_in(inner, item, f);
            }
        }
        (
            Node::Tagged {
                tag,
                common,
                variants,
            },
            Value::Object(values),
        ) => {
            each_value(common, values, f);
            let chosen = values.get(tag).and_then(Value::as_str).map(str::to_owned);
            if let Some(variant) = variants
                .iter()
                .find(|variant| Some(&variant.value) == chosen.as_ref())
            {
                each_value(&variant.table, values, f);
            }
        }
        _ => {}
    }
}

fn read_optional(path: &Path) -> Result<String, ImportError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(ImportError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Copies `path` to `<path>.bak`, when it exists.
fn keep(path: &Path) -> Result<Option<PathBuf>, ImportError> {
    if !path.exists() {
        return Ok(None);
    }
    let mut kept = path.as_os_str().to_owned();
    kept.push(".bak");
    let kept = PathBuf::from(kept);
    std::fs::copy(path, &kept).map_err(|source| ImportError::Io {
        path: kept.clone(),
        source,
    })?;
    Ok(Some(kept))
}

/// Writes through a temporary file, so a crash leaves the old file whole. A
/// new private file is readable by its owner only.
fn write(path: &Path, text: &str, private: bool) -> Result<(), ImportError> {
    let io = |source| ImportError::Io {
        path: path.to_owned(),
        source,
    };
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = match std::fs::metadata(path) {
            Ok(metadata) => metadata.permissions().mode() & 0o777,
            Err(_) if private => 0o600,
            Err(_) => 0o644,
        };
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode)).map_err(io)?;
    }
    std::fs::rename(&temporary, path).map_err(io)
}
