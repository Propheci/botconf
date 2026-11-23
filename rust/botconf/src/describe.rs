//! A description of the whole config file, built from the config type
//! (`#[derive(Describe)]`, in `botconf-derive`) so that each option is
//! described once: its key, title, description, value kind, default and
//! whether it can change at runtime. From it botconf derives the runtime
//! settings ([`Description::runtime_settings`]), the example file
//! ([`Description::example`]) and the config file an offline setup writes
//! ([`Description::render`], used by `botconf-tui`).
//!
//! Values are JSON, as everywhere in botconf; the file is TOML.
//!
//! # The derive
//!
//! `#[derive(Describe)]` (the `derive` feature) reads what serde reads:
//! field names with `rename` and `rename_all`; `default` and `default =
//! "fn"` on a field, or `default` on the type (its `Default` gives each
//! field's), as the option's default when its type is `Serialize`; `skip` and
//! `skip_deserializing`; `flatten`; enums of unit variants as one-of values,
//! internally tagged enums (`tag = "…"`) as tagged tables. A field without a
//! default whose type is not an `Option` is required. The doc comments are
//! the descriptions (a field without one takes its type's), and a key's title
//! is the key in words unless set. `#[botconf(…)]` says the rest:
//!
//! | Attribute | On | Meaning |
//! |---|---|---|
//! | `title = "…"` | field, variant | The title, instead of the key in words |
//! | `runtime` | field | Can change at runtime: one of [`Description::runtime_settings`] |
//! | `panel = <Kind>` | field | The runtime setting's panel editor, instead of the value kind's |
//! | `secret` | field, type | A secret value: masked on screens |
//! | `env`, `env = token \| secret \| plain` | field | Names an environment variable holding the value (a token can be generated) |
//! | `kind = <ValueKind>` | field, type | The value kind, instead of the type's |
//! | `min = <i64>`, `max = <i64>` | field | An integer's bounds, instead of the type's |
//! | `example = <JSON>` | field | What the generated example sets |
//! | `hint = <JSON>` | field | Shown commented out when absent and there is no default |
//! | `skip` | field, variant | Not described |
//! | `crate = "path"` | type | Where botconf is, when not `::botconf` |
//!
//! `kind`, `panel`, `min` and `max` are Rust expressions, with botconf's
//! names (`ValueKind`, `Kind`, …) in scope; `example` and `hint` are JSON,
//! as `serde_json::json!` writes it. A field read through `with` or
//! `deserialize_with` needs `kind` (or `skip`); an untagged or adjacently
//! tagged enum, or a generic type, needs a hand-written impl.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::{Choices, FixedChoice, Kind, RuntimeSetting, Shared};

/// The longest a comment line of a rendered file gets, `# ` included.
const WIDTH: usize = 80;

/// A program's whole configuration file.
#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub title: String,
    /// Shown at the top of the file and of a setup screen.
    pub description: String,
    pub table: Table,
}

/// The options of a table, in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub entries: Vec<Entry>,
}

/// One key of a table: an option, or a table of them.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub title: String,
    pub description: String,
    pub node: Node,
    /// The program refuses to start without it.
    pub required: bool,
    /// What the program uses when the key is absent.
    pub default: Option<Value>,
    /// What the generated example sets.
    pub example: Option<Value>,
    /// Shown commented out when the key is absent and there is no default.
    pub hint: Option<Value>,
    /// Can change while the program runs (a runtime setting).
    pub runtime: bool,
    /// The panel editor of a runtime setting, when the value kind's is not
    /// enough (a reference, a form).
    pub panel_kind: Option<Kind>,
}

/// What a key holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Value(ValueKind),
    /// `[key]`.
    Table(Table),
    /// `[[key]]` for tables; an array for values.
    List(Box<Node>),
    /// `[key.<name>]` for tables; `[key]` of values for values.
    Map(Box<Node>),
    /// A table whose `tag` key picks which of the variants' keys it has,
    /// besides the `common` ones (serde's internally tagged enums).
    Tagged {
        tag: String,
        common: Table,
        variants: Vec<Variant>,
    },
}

/// One variant of a tagged table.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    /// The tag's value.
    pub value: String,
    pub title: String,
    pub description: String,
    pub table: Table,
}

/// The kind of a single value.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueKind {
    Text,
    /// Text the file holds but a screen masks.
    Secret,
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Float,
    /// One of these values.
    OneOf(Vec<String>),
    /// A list of text values.
    TextList,
    /// The name of an environment variable whose value is the setting.
    EnvVar(EnvValue),
    /// Any JSON value (the fallback).
    Json,
}

/// What an [`ValueKind::EnvVar`] option's variable holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvValue {
    /// A shared secret any random value serves: masked, can be generated.
    Token,
    /// A secret issued elsewhere (an API hash, a bot token): masked.
    Secret,
    /// A plain value.
    Plain,
}

/// A type that knows how its values are written in a config file. Derive it
/// with `#[derive(botconf::Describe)]` (the `derive` feature).
pub trait Describe {
    fn node() -> Node;

    /// The type's own description, for a field that has none.
    fn doc() -> String {
        String::new()
    }
}

impl Entry {
    /// An entry with only a key and a node; the derive fills the rest.
    pub fn new(key: impl Into<String>, node: Node) -> Self {
        let key = key.into();
        Self {
            title: title_of(&key),
            key,
            description: String::new(),
            node,
            required: false,
            default: None,
            example: None,
            hint: None,
            runtime: false,
            panel_kind: None,
        }
    }

    /// The value shown commented out when the key is absent.
    pub fn placeholder(&self) -> Value {
        if let Some(value) = self.hint.as_ref().or(self.default.as_ref()) {
            return value.clone();
        }
        match &self.node {
            Node::Value(ValueKind::Bool) => Value::Bool(false),
            Node::Value(ValueKind::Int { min, .. }) => Value::from((*min).max(0)),
            Node::Value(ValueKind::Float) => Value::from(0.0),
            Node::Value(ValueKind::TextList) | Node::List(_) => Value::Array(Vec::new()),
            Node::Value(ValueKind::OneOf(choices)) => {
                Value::String(choices.first().cloned().unwrap_or_default())
            }
            _ => Value::String(String::new()),
        }
    }

    fn is_value(&self) -> bool {
        match &self.node {
            Node::Value(_) => true,
            Node::List(inner) | Node::Map(inner) => matches!(**inner, Node::Value(_)),
            _ => false,
        }
    }
}

/// `max_message_length` → `Max message length`.
pub fn title_of(key: &str) -> String {
    let words = key.replace(['_', '-'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Description {
    /// The description of `T`, whose node must be a table.
    pub fn of<T: Describe>(title: impl Into<String>, description: impl Into<String>) -> Self {
        let Node::Table(table) = T::node() else {
            panic!("a configuration is described by a struct");
        };
        Self {
            title: title.into(),
            description: description.into(),
            table,
        }
    }

    /// The runtime settings: every entry marked runtime, with its absolute
    /// dotted key, below tables (not lists or maps).
    pub fn runtime_settings(&self) -> Vec<RuntimeSetting> {
        let mut settings = Vec::new();
        collect_runtime(&self.table, "", &mut settings);
        settings
    }

    /// The example file: each option's description, the examples set, the
    /// rest commented out.
    pub fn example(&self) -> String {
        self.render(&Value::Object(example_values(&self.table)))
    }

    /// The config file holding `values`, written from the description: each
    /// option preceded by its description, absent ones commented out. Keys
    /// the description does not know are kept, commented out, at the end.
    pub fn render(&self, values: &Value) -> String {
        let empty = Map::new();
        let values = values.as_object().unwrap_or(&empty);
        let mut out = String::new();
        comment(&mut out, &self.title);
        if !self.description.is_empty() {
            out.push_str("#\n");
            comment(&mut out, &self.description);
        }
        render_table(&mut out, &self.table, values, &[]);
        let mut unknown = Vec::new();
        collect_unknown(&self.table, values, "", &mut unknown);
        if !unknown.is_empty() {
            out.push_str("\n# Not in this program's description (kept, but it ignores them):\n");
            for (key, value) in unknown {
                let _ = writeln!(out, "# {key} = {}", toml_value(&value));
            }
        }
        out
    }
}

fn collect_runtime(table: &Table, prefix: &str, settings: &mut Vec<RuntimeSetting>) {
    for entry in &table.entries {
        let key = join(prefix, &entry.key);
        if entry.runtime {
            let kind = entry
                .panel_kind
                .clone()
                .unwrap_or_else(|| panel_kind(&entry.node, !entry.required));
            settings.push(RuntimeSetting {
                key: Shared::new(key.as_str()),
                title: Some(Shared::new(entry.title.as_str())),
                description: Shared::new(entry.description.as_str()),
                per_entry: false,
                kind,
            });
        } else if let Node::Table(inner) = &entry.node {
            collect_runtime(inner, &key, settings);
        }
    }
}

/// The panel editor for a value kind.
fn panel_kind(node: &Node, optional: bool) -> Kind {
    let choices = |pairs: Vec<(String, String)>| {
        Choices::Fixed(Shared::new(
            pairs
                .into_iter()
                .map(|(value, label)| FixedChoice {
                    value: Shared::new(value.as_str()),
                    label: Shared::new(label.as_str()),
                })
                .collect::<Vec<_>>(),
        ))
    };
    match node {
        Node::Value(ValueKind::Text | ValueKind::Secret | ValueKind::EnvVar(_)) => {
            Kind::Text { optional }
        }
        Node::Value(ValueKind::Bool) => Kind::OneOf {
            choices: choices(vec![
                ("true".into(), "✅ Yes".into()),
                ("false".into(), "❌ No".into()),
            ]),
            custom: false,
            optional,
        },
        Node::Value(ValueKind::Int { min, max }) => Kind::Number {
            min: *min,
            max: *max,
            unit: Shared::new(""),
            suggestions: Shared::new(Vec::<i64>::new()),
            optional,
        },
        Node::Value(ValueKind::OneOf(values)) => Kind::OneOf {
            choices: choices(values.iter().map(|v| (v.clone(), v.clone())).collect()),
            custom: false,
            optional,
        },
        _ => Kind::Json,
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

/// The values the example sets: every entry's example, recursively.
fn example_values(table: &Table) -> Map<String, Value> {
    let mut values = Map::new();
    for entry in &table.entries {
        let value = match (&entry.example, &entry.node) {
            (Some(example), _) => Some(example.clone()),
            (None, Node::Table(inner)) => {
                let inner = example_values(inner);
                (!inner.is_empty()).then_some(Value::Object(inner))
            }
            _ => None,
        };
        if let Some(value) = value {
            values.insert(entry.key.clone(), value);
        }
    }
    values
}

/// Writes `text` as comment lines, wrapped; blank lines separate
/// paragraphs.
fn comment(out: &mut String, text: &str) {
    for (index, paragraph) in text.split("\n\n").enumerate() {
        if index > 0 {
            out.push_str("#\n");
        }
        let mut line = String::from("#");
        for word in paragraph.split_whitespace() {
            if line.len() > 1 && line.len() + 1 + word.len() > WIDTH {
                out.push_str(&line);
                out.push('\n');
                line = String::from("#");
            }
            line.push(' ');
            line.push_str(word);
        }
        if line.len() > 1 {
            out.push_str(&line);
            out.push('\n');
        }
    }
}

/// The comment block before an option: its description, then what it
/// defaults to and whether it is required.
fn entry_comment(out: &mut String, entry: &Entry) {
    out.push('\n');
    if !entry.description.is_empty() {
        comment(out, &entry.description);
    }
    if let Node::Value(ValueKind::OneOf(choices)) = &entry.node {
        comment(out, &format!("One of: {}.", choices.join(", ")));
    }
    if entry.required {
        out.push_str("# Required.\n");
    } else if let Some(default) = entry
        .default
        .as_ref()
        .filter(|value| entry.is_value() && !value.is_null())
    {
        let _ = writeln!(out, "# Default: {}.", toml_value(default));
    }
}

/// The values first, then the tables: TOML puts a table's keys before its
/// sub-tables.
fn render_table(out: &mut String, table: &Table, values: &Map<String, Value>, path: &[String]) {
    for entry in table.entries.iter().filter(|entry| entry.is_value()) {
        entry_comment(out, entry);
        match values.get(&entry.key).filter(|value| !value.is_null()) {
            Some(value) => {
                let _ = writeln!(out, "{} = {}", toml_key(&entry.key), toml_value(value));
            }
            None => {
                let _ = writeln!(
                    out,
                    "# {} = {}",
                    toml_key(&entry.key),
                    toml_value(&entry.placeholder())
                );
            }
        }
    }
    for entry in table.entries.iter().filter(|entry| !entry.is_value()) {
        let mut path = path.to_vec();
        path.push(entry.key.clone());
        let value = values.get(&entry.key);
        match &entry.node {
            Node::Table(inner) => {
                entry_comment(out, entry);
                let _ = writeln!(out, "[{}]", header(&path));
                let empty = Map::new();
                let inner_values = value.and_then(Value::as_object).unwrap_or(&empty);
                render_table(out, inner, inner_values, &path);
            }
            Node::List(inner) => {
                entry_comment(out, entry);
                let items = value.and_then(Value::as_array).cloned().unwrap_or_default();
                if items.is_empty() {
                    let _ = writeln!(out, "# [[{}]]", header(&path));
                }
                for item in &items {
                    let _ = writeln!(out, "[[{}]]", header(&path));
                    render_item(out, inner, item, &path);
                }
            }
            Node::Map(inner) => {
                entry_comment(out, entry);
                let items = value
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                if items.is_empty() {
                    let _ = writeln!(out, "# [{}.<name>]", header(&path));
                }
                for (name, item) in &items {
                    let mut entry_path = path.clone();
                    entry_path.push(name.clone());
                    let _ = writeln!(out, "[{}]", header(&entry_path));
                    render_item(out, inner, item, &entry_path);
                }
            }
            Node::Tagged { .. } => {
                // A tagged table on its own (not in a list or a map).
                entry_comment(out, entry);
                let _ = writeln!(out, "[{}]", header(&path));
                render_item(out, &entry.node, value.unwrap_or(&Value::Null), &path);
            }
            Node::Value(_) => unreachable!("values were written first"),
        }
    }
}

/// One table of a list or a map (or a tagged table), after its header.
fn render_item(out: &mut String, node: &Node, item: &Value, path: &[String]) {
    let empty = Map::new();
    let values = item.as_object().unwrap_or(&empty);
    match node {
        Node::Table(table) => render_table(out, table, values, path),
        Node::Tagged {
            tag,
            common,
            variants,
        } => {
            let chosen = values.get(tag).and_then(Value::as_str);
            let _ = writeln!(
                out,
                "{} = {}",
                toml_key(tag),
                toml_value(&Value::String(chosen.unwrap_or_default().into()))
            );
            let mut table = common.clone();
            if let Some(variant) = variants
                .iter()
                .find(|variant| Some(variant.value.as_str()) == chosen)
            {
                table.entries.extend(variant.table.entries.iter().cloned());
            }
            render_table(out, &table, values, path);
        }
        // A map or a list of plain values: one `name = value` line each.
        Node::Value(_) | Node::List(_) | Node::Map(_) => {
            let _ = writeln!(out, "{}", toml_value(item));
        }
    }
}

/// The keys of `values` the description does not know, dotted.
fn collect_unknown(
    table: &Table,
    values: &Map<String, Value>,
    prefix: &str,
    out: &mut Vec<(String, Value)>,
) {
    for (key, value) in values {
        let path = join(prefix, key);
        match table.entries.iter().find(|entry| &entry.key == key) {
            None => out.push((path, value.clone())),
            Some(Entry {
                node: Node::Table(inner),
                ..
            }) => {
                if let Some(inner_values) = value.as_object() {
                    collect_unknown(inner, inner_values, &path, out);
                }
            }
            Some(_) => {}
        }
    }
}

/// A key as TOML writes it: bare when it can be.
fn toml_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        key.to_owned()
    } else {
        toml_string(key)
    }
}

fn header(path: &[String]) -> String {
    path.iter()
        .map(|key| toml_key(key))
        .collect::<Vec<_>>()
        .join(".")
}

fn toml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A value as TOML writes it inline.
pub fn toml_value(value: &Value) -> String {
    match value {
        Value::Null => "\"\"".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => match (number.as_i64(), number.as_u64(), number.as_f64()) {
            (Some(int), _, _) => int.to_string(),
            (None, Some(uint), _) => uint.to_string(),
            (_, _, Some(float)) if float.fract() == 0.0 => format!("{float:.1}"),
            (_, _, Some(float)) => float.to_string(),
            _ => number.to_string(),
        },
        Value::String(text) => toml_string(text),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(toml_value).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(object) => format!(
            "{{ {} }}",
            object
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| format!("{} = {}", toml_key(key), toml_value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

macro_rules! describe_int {
    ($($type:ty),*) => {$(
        impl Describe for $type {
            fn node() -> Node {
                // Bounds beyond i64 (u64's top half) are capped.
                Node::Value(ValueKind::Int {
                    min: i64::try_from(<$type>::MIN).unwrap_or(i64::MIN),
                    max: i64::try_from(<$type>::MAX).unwrap_or(i64::MAX),
                })
            }
        }
    )*};
}

describe_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl Describe for String {
    fn node() -> Node {
        Node::Value(ValueKind::Text)
    }
}

impl Describe for bool {
    fn node() -> Node {
        Node::Value(ValueKind::Bool)
    }
}

impl Describe for f32 {
    fn node() -> Node {
        Node::Value(ValueKind::Float)
    }
}

impl Describe for f64 {
    fn node() -> Node {
        Node::Value(ValueKind::Float)
    }
}

impl Describe for PathBuf {
    fn node() -> Node {
        Node::Value(ValueKind::Text)
    }
}

impl Describe for SocketAddr {
    fn node() -> Node {
        Node::Value(ValueKind::Text)
    }
}

impl Describe for Value {
    fn node() -> Node {
        Node::Value(ValueKind::Json)
    }
}

impl<T: Describe> Describe for Option<T> {
    fn node() -> Node {
        T::node()
    }

    fn doc() -> String {
        T::doc()
    }
}

impl<T: Describe> Describe for Vec<T> {
    fn node() -> Node {
        match T::node() {
            Node::Value(ValueKind::Text) => Node::Value(ValueKind::TextList),
            node => Node::List(Box::new(node)),
        }
    }

    fn doc() -> String {
        T::doc()
    }
}

impl<T: Describe> Describe for BTreeMap<String, T> {
    fn node() -> Node {
        Node::Map(Box::new(T::node()))
    }

    fn doc() -> String {
        T::doc()
    }
}

impl<T: Describe, S> Describe for HashMap<String, T, S> {
    fn node() -> Node {
        Node::Map(Box::new(T::node()))
    }

    fn doc() -> String {
        T::doc()
    }
}

#[doc(hidden)]
pub mod __private;
#[cfg(test)]
mod tests;
