//! What a [`Target`] refers to in the catalog, and the edits that change it.

use serde_json::{Map, Value};

use botconf::{Catalog, CatalogEntry, Field, Kind, Schema, Snapshot};

use crate::{edit::Edit, token::Target};

/// What a [`Target`] refers to.
#[derive(Clone, Copy, Debug)]
pub struct Setting<'a> {
    pub setting: &'a CatalogEntry,
    /// The entry name, for an entry of a map.
    pub entry: Option<&'a str>,
    /// The field, for a field of a form.
    pub field: Option<&'a Field>,
    /// The kind of the value: the map's values' kind for an entry, the
    /// field's for a field.
    pub kind: &'a Kind,
}

impl<'a> Setting<'a> {
    pub fn resolve(catalog: &'a Catalog, target: &'a Target) -> Option<Self> {
        let setting = catalog.entry(target.setting)?;
        let (entry, kind): (_, &'a Kind) = match (&target.entry, &setting.kind) {
            (None, kind) => (None, kind),
            (Some(name), Kind::Map { value, .. }) => (Some(name.as_str()), &**value),
            (Some(_), _) => return None,
        };
        let (field, kind) = match (target.field, kind) {
            (None, kind) => (None, kind),
            (Some(field), Kind::Form(form)) => {
                let field = form.fields.get(field)?;
                (Some(field), &field.kind)
            }
            (Some(_), _) => return None,
        };
        Some(Self {
            setting,
            entry,
            field,
            kind,
        })
    }

    /// The key the value is stored at (a form's, for a field).
    pub fn key(&self) -> String {
        match self.entry {
            Some(name) => format!("{}.{name}", self.setting.key),
            None => self.setting.key.clone(),
        }
    }

    pub fn title(&self) -> String {
        let mut title = self.setting.title.clone();
        if let Some(name) = self.entry {
            title.push_str(&format!(" › {name}"));
        }
        if let Some(field) = self.field {
            title.push_str(&format!(" › {}", field.title));
        }
        title
    }

    /// The current value, if set.
    pub fn value<S: Schema>(&self, snapshot: &Snapshot<S>) -> Option<Value> {
        let value = snapshot.value(&self.key())?;
        let value = match self.field {
            Some(field) => value.get(&*field.key)?.clone(),
            None => value,
        };
        (!value.is_null()).then_some(value)
    }

    /// The edit setting the value to `value` (clearing it for `None`). For a
    /// field, the form is rewritten, without the fields of its group.
    pub fn set<S: Schema>(&self, snapshot: &Snapshot<S>, value: Option<Value>) -> Edit {
        let key = self.key();
        let Some(field) = self.field else {
            return Edit::Set(key, value.unwrap_or(Value::Null));
        };

        let mut form = match snapshot.value(&key) {
            Some(Value::Object(form)) => form,
            _ => Map::new(),
        };
        match value {
            Some(value) => {
                if let (Some(group), Kind::Form(kind)) = (&field.group, self.form_kind()) {
                    for other in kind
                        .fields
                        .iter()
                        .filter(|other| other.group.as_ref() == Some(group))
                    {
                        form.remove(&*other.key);
                    }
                }
                form.insert(field.key.to_string(), value);
            }
            None => {
                form.remove(&*field.key);
            }
        }
        Edit::Set(key, Value::Object(form))
    }

    /// The edit adding `items` to the list (and removing `removed`).
    pub fn change_list<S: Schema>(
        &self,
        snapshot: &Snapshot<S>,
        added: Vec<Value>,
        removed: Option<Value>,
    ) -> Edit {
        if self.field.is_none() {
            let key = self.key();
            return match removed {
                Some(item) => Edit::Remove(key, item),
                None => Edit::Extend(key, added),
            };
        }

        let mut items = match self.value(snapshot) {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        };
        items.retain(|item| Some(item) != removed.as_ref());
        for item in added {
            if !items.contains(&item) {
                items.push(item);
            }
        }
        self.set(snapshot, Some(Value::Array(items)))
    }

    /// The kind of the form holding the field.
    pub(crate) fn form_kind(&self) -> &'a Kind {
        match (self.entry, &self.setting.kind) {
            (Some(_), Kind::Map { value, .. }) => value,
            (_, kind) => kind,
        }
    }
}
