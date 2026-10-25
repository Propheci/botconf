//! Sections registered at runtime: parts of the configuration described while
//! the program runs (e.g. by a connector when it connects), with their own
//! validator and base values, below a [dynamic
//! namespace](crate::Schema::dynamic_namespaces).

use std::{fmt, sync::Arc};

use figment::{
    Error, Figment, Metadata, Profile, Provider,
    providers::Serialized,
    value::{Dict, Map},
};
use serde_json::Value;

use crate::{keys::RuntimeSetting, store::BuildError};

/// A section described at runtime, e.g. by a connector when it connects.
#[derive(Clone)]
pub struct DynamicSection {
    /// Unique among sections, e.g. `connector:telegram-1`.
    pub id: String,
    /// Where it is in the configuration, e.g. `connectors.telegram-1`; must be
    /// below one of the schema's dynamic namespaces.
    pub path: String,
    pub title: String,
    pub description: String,
    /// The keys that can change at runtime, relative to `path`.
    pub settings: Vec<RuntimeSetting>,
    pub validator: Arc<dyn Validator>,
    /// The section's base values (an object), below the stored overrides.
    pub base: Value,
    /// Where the base comes from, shown as its source, e.g. `connector
    /// telegram-1`.
    pub base_source: String,
}

impl fmt::Debug for DynamicSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The base is left out: it may hold secrets.
        f.debug_struct("DynamicSection")
            .field("id", &self.id)
            .field("path", &self.path)
            .field("title", &self.title)
            .field("settings", &self.settings)
            .field("base_source", &self.base_source)
            .finish_non_exhaustive()
    }
}

/// Checks a dynamic section, in place of the Rust type a static section is
/// deserialized into (e.g. with a JSON Schema the section's owner sent).
pub trait Validator: Send + Sync + 'static {
    /// The section's effective value (with whatever defaults the validator
    /// fills in), or why it is invalid.
    fn validate(&self, section: &Value) -> Result<Value, String>;
}

impl<F> Validator for F
where
    F: Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
{
    fn validate(&self, section: &Value) -> Result<Value, String> {
        self(section)
    }
}

/// A validated dynamic section, as kept in a snapshot.
pub(crate) struct DynamicParsed {
    pub(crate) path: String,
    pub(crate) value: Value,
    pub(crate) figment: Figment,
}

impl DynamicSection {
    /// Layers `overrides` (keys relative to the section's path) on the base,
    /// then validates the result.
    pub(crate) fn build<'a>(
        &self,
        overrides: impl IntoIterator<Item = (&'a str, &'a Value)>,
    ) -> Result<DynamicParsed, BuildError> {
        let base = Figment::from(Base {
            name: &self.base_source,
            value: &self.base,
        });
        let figment = crate::store::layer(base, overrides);
        let invalid = |reason: String| BuildError::Section {
            path: self.path.clone(),
            reason,
        };
        let raw: Value = figment
            .extract()
            .map_err(|error| invalid(error.to_string()))?;
        let value = self.validator.validate(&raw).map_err(invalid)?;
        Ok(DynamicParsed {
            path: self.path.clone(),
            value,
            figment,
        })
    }
}

/// A dynamic section's base, named after where it comes from.
struct Base<'a> {
    name: &'a str,
    value: &'a Value,
}

impl Provider for Base<'_> {
    fn metadata(&self) -> Metadata {
        Metadata::named(self.name.to_string())
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        Serialized::defaults(self.value).data()
    }
}
