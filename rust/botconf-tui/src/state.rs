//! The setup screens' state: the values being edited, the screens open and
//! what a key does. Pure (no terminal, no files), so it is tested by keys.

use botconf::describe::{self, Description, Entry, EnvValue, Node, Table, ValueKind, Variant};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Map, Value};

use crate::env::EnvFile;

/// One step of a path into the values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// An open screen: a table, a tagged table, a list or a map.
#[derive(Clone, Debug)]
pub struct Screen {
    pub path: Vec<Seg>,
    pub title: String,
    pub cursor: usize,
}

/// One line of a screen.
#[derive(Clone, Debug)]
pub enum Row {
    /// An option, or a table, list or map of them.
    Entry(Box<Entry>),
    /// A tagged table's tag: picks which variant's options it has.
    Tag { tag: String, variants: Vec<Variant> },
    /// One table of a list or a map.
    Item { seg: Seg, label: String },
}

#[derive(Clone, Debug)]
pub enum Mode {
    Browse,
    Edit(Edit),
    Choose(Choose),
    Confirm(Confirm),
    /// Closed by any key.
    Message(String),
    Help,
}

#[derive(Clone, Debug)]
pub struct Edit {
    pub title: String,
    pub target: Target,
    pub input: Input,
    /// Masked on screen.
    pub secret: bool,
    /// Shown under the input: what an empty input means, or the last error.
    pub note: String,
}

#[derive(Clone, Debug)]
pub enum Target {
    /// A value at a path; an empty input removes it.
    Value { path: Vec<Seg>, kind: ValueKind },
    /// An environment variable's value in the env file.
    Env { var: String },
    /// The name of a new map entry.
    NewName { path: Vec<Seg> },
}

#[derive(Clone, Debug)]
pub struct Choose {
    pub title: String,
    pub path: Vec<Seg>,
    /// `(value, label)`.
    pub options: Vec<(String, String)>,
    pub cursor: usize,
    /// Choosing a tag also drops the options of the variant left.
    pub tag: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Quit with unsaved changes.
    Quit,
    /// Delete an item of a list or a map.
    Delete { path: Vec<Seg> },
    /// Replace a token already set.
    Regenerate { var: String },
}

/// What the caller does after a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// Check the candidate files and write them; then call [`App::saved`].
    Save,
    Quit,
}

/// How a row's value reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Set(String),
    Default(String),
    Unset,
    /// Required and unset.
    Missing,
    /// Opens a screen (a table, a list, a map, an item).
    Opens(String),
}

/// A line editor.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub text: String,
    /// In characters.
    pub cursor: usize,
}

impl Input {
    pub fn new(text: String) -> Self {
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    fn byte(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map_or(self.text.len(), |(byte, _)| byte)
    }

    /// Edits for a key; false when the key is not an editing one.
    fn key(&mut self, key: KeyEvent) -> bool {
        let len = self.text.chars().count();
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c) => {
                let at = self.byte(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Delete if self.cursor < len => {
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::Backspace | KeyCode::Delete => {}
            _ => return false,
        }
        true
    }
}

pub struct App {
    pub description: Description,
    /// The config file's values (an object).
    pub values: Value,
    pub env: EnvFile,
    /// How the env file is named on screen.
    pub env_name: String,
    pub screens: Vec<Screen>,
    pub mode: Mode,
    pub dirty: bool,
    /// A one-line notice, until the next key.
    pub status: Option<String>,
    token: fn() -> Result<String, String>,
}

impl App {
    pub fn new(
        description: Description,
        values: Value,
        env: EnvFile,
        env_name: String,
        token: fn() -> Result<String, String>,
    ) -> Self {
        let values = match values {
            Value::Object(_) => values,
            _ => Value::Object(Map::new()),
        };
        let root = Screen {
            path: Vec::new(),
            title: description.title.clone(),
            cursor: 0,
        };
        Self {
            description,
            values,
            env,
            env_name,
            screens: vec![root],
            mode: Mode::Browse,
            dirty: false,
            status: None,
            token,
        }
    }

    pub fn screen(&self) -> &Screen {
        self.screens.last().expect("the root screen stays")
    }

    fn screen_mut(&mut self) -> &mut Screen {
        self.screens.last_mut().expect("the root screen stays")
    }

    /// The config file and the env file a save writes.
    pub fn candidate(&self) -> (String, String) {
        (self.description.render(&self.values), self.env.render())
    }

    /// After a save: clean, or the reason it was refused.
    pub fn saved(&mut self, result: Result<String, String>) {
        match result {
            Ok(notice) => {
                self.dirty = false;
                self.status = Some(notice);
            }
            Err(error) => self.mode = Mode::Message(format!("Not saved: {error}")),
        }
    }

    // ---- What the screens show.

    /// The node the screen at `path` shows.
    fn node_at(&self, path: &[Seg]) -> Option<Node> {
        let mut node = Node::Table(self.description.table.clone());
        for (depth, seg) in path.iter().enumerate() {
            node = match (node, seg) {
                (Node::Table(table), Seg::Key(key)) => find(table, key)?,
                (
                    Node::Tagged {
                        tag,
                        common,
                        variants,
                    },
                    Seg::Key(key),
                ) => {
                    let value = get(&self.values, &path[..depth]);
                    find(tagged_table(&tag, &common, &variants, value), key)?
                }
                (Node::List(inner), Seg::Index(_)) | (Node::Map(inner), Seg::Key(_)) => *inner,
                _ => return None,
            };
        }
        Some(node)
    }

    pub fn rows(&self) -> Vec<Row> {
        let path = &self.screen().path;
        match self.node_at(path) {
            Some(Node::Table(table)) => table
                .entries
                .into_iter()
                .map(|entry| Row::Entry(Box::new(entry)))
                .collect(),
            Some(Node::Tagged {
                tag,
                common,
                variants,
            }) => {
                let table = tagged_table(&tag, &common, &variants, get(&self.values, path));
                let mut rows = vec![Row::Tag { tag, variants }];
                rows.extend(
                    table
                        .entries
                        .into_iter()
                        .map(|entry| Row::Entry(Box::new(entry))),
                );
                rows
            }
            Some(Node::List(_)) => match get(&self.values, path) {
                Some(Value::Array(items)) => items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| Row::Item {
                        seg: Seg::Index(index),
                        label: format!("{}. {}", index + 1, label_of(item)),
                    })
                    .collect(),
                _ => Vec::new(),
            },
            Some(Node::Map(_)) => match get(&self.values, path) {
                Some(Value::Object(items)) => items
                    .keys()
                    .map(|name| Row::Item {
                        seg: Seg::Key(name.clone()),
                        label: name.clone(),
                    })
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// A list or a map: items are added and deleted.
    pub fn is_collection(&self) -> bool {
        matches!(
            self.node_at(&self.screen().path),
            Some(Node::List(_) | Node::Map(_))
        )
    }

    fn child(&self, seg: Seg) -> Vec<Seg> {
        let mut path = self.screen().path.clone();
        path.push(seg);
        path
    }

    pub fn title_of(row: &Row) -> String {
        match row {
            Row::Entry(entry) => entry.title.clone(),
            Row::Tag { tag, .. } => describe::title_of(tag),
            Row::Item { label, .. } => label.clone(),
        }
    }

    pub fn shown(&self, row: &Row) -> Shown {
        match row {
            Row::Item { .. } => Shown::Opens(String::new()),
            Row::Tag { tag, variants } => {
                let path = self.child(Seg::Key(tag.clone()));
                match get(&self.values, &path).and_then(Value::as_str) {
                    Some(value) => Shown::Set(
                        variants
                            .iter()
                            .find(|variant| variant.value == value)
                            .map_or_else(|| value.to_owned(), |variant| variant.title.clone()),
                    ),
                    None => Shown::Missing,
                }
            }
            Row::Entry(entry) => {
                let value = get(&self.values, &self.child(Seg::Key(entry.key.clone())));
                match (&entry.node, value) {
                    (Node::Table(_) | Node::Tagged { .. }, _) => Shown::Opens(String::new()),
                    (Node::List(inner) | Node::Map(inner), value)
                        if !matches!(**inner, Node::Value(_)) =>
                    {
                        let count = match value {
                            Some(Value::Array(items)) => items.len(),
                            Some(Value::Object(items)) => items.len(),
                            _ => 0,
                        };
                        Shown::Opens(match count {
                            0 => "none".into(),
                            1 => "1 item".into(),
                            n => format!("{n} items"),
                        })
                    }
                    (node, Some(value)) if !value.is_null() => Shown::Set(display(node, value)),
                    (node, _) => match &entry.default {
                        Some(default) => Shown::Default(display(node, default)),
                        None if entry.required => Shown::Missing,
                        None => Shown::Unset,
                    },
                }
            }
        }
    }

    /// The env variable an option names (its value, else its default or
    /// hint), and the env file's value of it.
    pub fn env_of(&self, entry: &Entry) -> Option<(String, Option<String>)> {
        let Node::Value(ValueKind::EnvVar(_)) = entry.node else {
            return None;
        };
        let set = get(&self.values, &self.child(Seg::Key(entry.key.clone())));
        let name = [set, entry.default.as_ref(), entry.hint.as_ref()]
            .into_iter()
            .flatten()
            .find_map(Value::as_str)
            .filter(|name| !name.is_empty())?
            .to_owned();
        let value = self.env.get(&name).map(str::to_owned);
        Some((name, value))
    }

    /// The details pane: the selected row's description and facts.
    pub fn details(&self, row: &Row) -> (String, Vec<String>) {
        let mut lines = Vec::new();
        match row {
            Row::Entry(entry) => {
                if !entry.description.is_empty() {
                    lines.extend(entry.description.split("\n\n").map(str::to_owned));
                }
                let mut facts = Vec::new();
                facts.push(format!("Key: {}", entry.key));
                match &entry.node {
                    Node::Value(ValueKind::OneOf(choices)) => {
                        facts.push(format!("One of: {}", choices.join(", ")));
                    }
                    Node::Value(ValueKind::Int { min, max }) => {
                        facts.push(format!("A whole number from {min} to {max}"));
                    }
                    Node::Value(ValueKind::TextList) => {
                        facts.push("A list: values separated by commas".into());
                    }
                    Node::Value(ValueKind::Json) => facts.push("Written as JSON".into()),
                    Node::Value(ValueKind::EnvVar(what)) => {
                        facts.push(format!(
                            "Names an environment variable; its value is set in {}",
                            self.env_name
                        ));
                        match self.env_of(entry) {
                            Some((name, Some(value))) => facts.push(match what {
                                EnvValue::Plain => format!("{name} = {value}"),
                                _ => format!("{name} is set (hidden)"),
                            }),
                            Some((name, None)) => facts.push(format!("{name} is not set")),
                            None => {}
                        }
                    }
                    _ => {}
                }
                if entry.required {
                    facts.push("Required".into());
                } else if let (Some(default), true) = (&entry.default, is_value(&entry.node)) {
                    facts.push(format!("Default: {}", display(&entry.node, default)));
                }
                if entry.runtime {
                    facts.push("Can also change while the program runs (settings panel)".into());
                }
                lines.push(facts.join("\n"));
                (entry.title.clone(), lines)
            }
            Row::Tag { variants, .. } => {
                lines.push("Which kind this is: it decides the options below.".into());
                for variant in variants {
                    let mut line = format!("{}: {}", variant.value, variant.title);
                    if !variant.description.is_empty() {
                        line = format!("{line} — {}", variant.description);
                    }
                    lines.push(line);
                }
                (Self::title_of(row), lines)
            }
            Row::Item { .. } => {
                lines.push("Enter: open · d: delete".into());
                (Self::title_of(row), lines)
            }
        }
    }

    /// The key help of the current screen.
    pub fn hints(&self) -> String {
        let mut hints = vec!["↑↓ move", "enter edit/open"];
        if self.screens.len() > 1 {
            hints.push("esc back");
        }
        match self.rows().get(self.screen().cursor) {
            Some(Row::Entry(entry)) => {
                if is_value(&entry.node) {
                    hints.push("u unset");
                }
                if let Node::Value(ValueKind::EnvVar(what)) = entry.node {
                    hints.push("e set value");
                    if what == EnvValue::Token {
                        hints.push("g generate");
                    }
                }
            }
            Some(Row::Item { .. }) => hints.push("d delete"),
            _ => {}
        }
        if self.is_collection() {
            hints.push("a add");
        }
        hints.extend(["s save", "q quit", "? help"]);
        hints.join(" · ")
    }

    // ---- Keys.

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        self.status = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return Effect::Quit;
        }
        match std::mem::replace(&mut self.mode, Mode::Browse) {
            Mode::Browse => self.browse(key),
            Mode::Message(_) | Mode::Help => Effect::None,
            Mode::Edit(edit) => {
                self.edit(edit, key);
                Effect::None
            }
            Mode::Choose(choose) => {
                self.choose(choose, key);
                Effect::None
            }
            Mode::Confirm(confirm) => self.confirm(&confirm, key),
        }
    }

    fn browse(&mut self, key: KeyEvent) -> Effect {
        let rows = self.rows();
        let count = rows.len();
        let cursor = self.screen().cursor.min(count.saturating_sub(1));
        let row = rows.get(cursor).cloned();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.screen_mut().cursor = cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.screen_mut().cursor = (cursor + 1).min(count.saturating_sub(1));
            }
            KeyCode::PageUp => self.screen_mut().cursor = cursor.saturating_sub(10),
            KeyCode::PageDown => {
                self.screen_mut().cursor = (cursor + 10).min(count.saturating_sub(1));
            }
            KeyCode::Home => self.screen_mut().cursor = 0,
            KeyCode::End => self.screen_mut().cursor = count.saturating_sub(1),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(row) = row {
                    self.activate(row);
                }
            }
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => {
                if self.screens.len() > 1 {
                    self.screens.pop();
                }
            }
            KeyCode::Char('s') => return Effect::Save,
            KeyCode::Char('q') => {
                if self.dirty {
                    self.mode = Mode::Confirm(Confirm::Quit);
                } else {
                    return Effect::Quit;
                }
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('u') | KeyCode::Delete => match row {
                Some(Row::Entry(entry)) if is_value(&entry.node) => {
                    let path = self.child(Seg::Key(entry.key));
                    if remove(&mut self.values, &path).is_some() {
                        self.dirty = true;
                    }
                }
                Some(Row::Item { seg, .. }) => {
                    self.mode = Mode::Confirm(Confirm::Delete {
                        path: self.child(seg),
                    });
                }
                _ => {}
            },
            KeyCode::Char('d') => {
                if let Some(Row::Item { seg, .. }) = row {
                    self.mode = Mode::Confirm(Confirm::Delete {
                        path: self.child(seg),
                    });
                }
            }
            KeyCode::Char('a') => self.add(),
            KeyCode::Char('e') => {
                if let Some(Row::Entry(entry)) = &row {
                    self.edit_env(entry);
                }
            }
            KeyCode::Char('g') => {
                if let Some(Row::Entry(entry)) = &row {
                    self.generate(entry, false);
                }
            }
            _ => {}
        }
        Effect::None
    }

    fn activate(&mut self, row: Row) {
        match row {
            Row::Item { seg, label } => self.open(seg, label),
            Row::Tag { tag, variants } => {
                let path = self.child(Seg::Key(tag.clone()));
                let current = get(&self.values, &path).and_then(Value::as_str);
                let options: Vec<(String, String)> = variants
                    .iter()
                    .map(|variant| (variant.value.clone(), variant.title.clone()))
                    .collect();
                let cursor = options
                    .iter()
                    .position(|(value, _)| Some(value.as_str()) == current)
                    .unwrap_or(0);
                self.mode = Mode::Choose(Choose {
                    title: describe::title_of(&tag),
                    path,
                    options,
                    cursor,
                    tag: true,
                });
            }
            Row::Entry(entry) => {
                let path = self.child(Seg::Key(entry.key.clone()));
                match &entry.node {
                    Node::Value(ValueKind::Bool) => {
                        let current = get(&self.values, &path)
                            .or(entry.default.as_ref())
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        set(&mut self.values, &path, Value::Bool(!current));
                        self.dirty = true;
                    }
                    Node::Value(ValueKind::OneOf(choices)) => {
                        let current = get(&self.values, &path)
                            .or(entry.default.as_ref())
                            .and_then(Value::as_str);
                        self.mode = Mode::Choose(Choose {
                            title: entry.title.clone(),
                            options: choices.iter().map(|c| (c.clone(), c.clone())).collect(),
                            cursor: choices
                                .iter()
                                .position(|c| Some(c.as_str()) == current)
                                .unwrap_or(0),
                            path,
                            tag: false,
                        });
                    }
                    node if is_value(node) => {
                        let kind = match node {
                            Node::Value(kind) => kind.clone(),
                            _ => ValueKind::Json,
                        };
                        let text = get(&self.values, &path)
                            .filter(|value| !value.is_null())
                            .map(|value| edit_text(&kind, value))
                            .unwrap_or_default();
                        let note = match (&entry.default, entry.required) {
                            (_, true) => "Required.".to_owned(),
                            (Some(default), _) => {
                                format!("Empty: the default, {}.", display(node, default))
                            }
                            (None, _) => "Empty: unset.".to_owned(),
                        };
                        self.mode = Mode::Edit(Edit {
                            title: entry.title.clone(),
                            secret: kind == ValueKind::Secret,
                            target: Target::Value { path, kind },
                            input: Input::new(text),
                            note,
                        });
                    }
                    _ => self.open(Seg::Key(entry.key.clone()), entry.title.clone()),
                }
            }
        }
    }

    fn open(&mut self, seg: Seg, title: String) {
        let path = self.child(seg);
        self.screens.push(Screen {
            path,
            title,
            cursor: 0,
        });
    }

    fn add(&mut self) {
        let path = self.screen().path.clone();
        match self.node_at(&path) {
            Some(Node::List(_)) => {
                let index = match get(&self.values, &path) {
                    Some(Value::Array(items)) => items.len(),
                    _ => 0,
                };
                let mut items = match get(&self.values, &path) {
                    Some(Value::Array(items)) => items.clone(),
                    _ => Vec::new(),
                };
                items.push(Value::Object(Map::new()));
                set(&mut self.values, &path, Value::Array(items));
                self.dirty = true;
                self.screen_mut().cursor = index;
                self.open(Seg::Index(index), format!("{}.", index + 1));
            }
            Some(Node::Map(_)) => {
                self.mode = Mode::Edit(Edit {
                    title: "New entry's name".into(),
                    target: Target::NewName { path },
                    input: Input::default(),
                    secret: false,
                    note: "Its key in the file.".into(),
                });
            }
            _ => {}
        }
    }

    fn edit_env(&mut self, entry: &Entry) {
        let Node::Value(ValueKind::EnvVar(what)) = entry.node else {
            return;
        };
        let Some((var, value)) = self.env_of(entry) else {
            self.status = Some("Name the variable first (enter).".into());
            return;
        };
        self.mode = Mode::Edit(Edit {
            title: format!("{var} in {}", self.env_name),
            target: Target::Env { var },
            input: Input::new(value.unwrap_or_default()),
            secret: what != EnvValue::Plain,
            note: String::new(),
        });
    }

    fn generate(&mut self, entry: &Entry, replace: bool) {
        let Node::Value(ValueKind::EnvVar(EnvValue::Token)) = entry.node else {
            return;
        };
        let Some((var, current)) = self.env_of(entry) else {
            self.status = Some("Name the variable first (enter).".into());
            return;
        };
        if current.is_some_and(|value| !value.is_empty()) && !replace {
            self.mode = Mode::Confirm(Confirm::Regenerate { var });
            return;
        }
        // A name only hinted becomes the option's value.
        let path = self.child(Seg::Key(entry.key.clone()));
        if get(&self.values, &path).is_none() && entry.default.is_none() {
            set(&mut self.values, &path, Value::String(var.clone()));
        }
        self.set_token(&var);
    }

    fn set_token(&mut self, var: &str) {
        match (self.token)() {
            Ok(token) => {
                self.env.set(var, &token);
                self.dirty = true;
                self.status = Some(format!(
                    "Generated a new {var} in {}; give the other side the same value.",
                    self.env_name
                ));
            }
            Err(error) => self.mode = Mode::Message(error),
        }
    }

    fn edit(&mut self, mut edit: Edit, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                if let Err(error) = self.commit(&edit) {
                    edit.note = error;
                    self.mode = Mode::Edit(edit);
                }
            }
            _ => {
                edit.input.key(key);
                self.mode = Mode::Edit(edit);
            }
        }
    }

    fn commit(&mut self, edit: &Edit) -> Result<(), String> {
        let text = &edit.input.text;
        match &edit.target {
            Target::Value { path, kind } => {
                match parse(kind, text)? {
                    Some(value) => set(&mut self.values, path, value),
                    None => {
                        remove(&mut self.values, path);
                    }
                }
                self.dirty = true;
            }
            Target::Env { var } => {
                self.env.set(var, text);
                self.dirty = true;
            }
            Target::NewName { path } => {
                let name = text.trim();
                if name.is_empty() {
                    return Err("A name is needed.".into());
                }
                let mut items = match get(&self.values, path) {
                    Some(Value::Object(items)) => items.clone(),
                    _ => Map::new(),
                };
                if items.contains_key(name) {
                    return Err(format!("{name} already exists."));
                }
                items.insert(name.to_owned(), Value::Object(Map::new()));
                set(&mut self.values, path, Value::Object(items));
                self.dirty = true;
                self.open(Seg::Key(name.to_owned()), name.to_owned());
            }
        }
        Ok(())
    }

    fn choose(&mut self, mut choose: Choose, key: KeyEvent) {
        let count = choose.options.len();
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Up | KeyCode::Char('k') => {
                choose.cursor = choose.cursor.saturating_sub(1);
                self.mode = Mode::Choose(choose);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                choose.cursor = (choose.cursor + 1).min(count.saturating_sub(1));
                self.mode = Mode::Choose(choose);
            }
            KeyCode::Enter => {
                let Some((value, _)) = choose.options.get(choose.cursor) else {
                    return;
                };
                set(&mut self.values, &choose.path, Value::String(value.clone()));
                if choose.tag {
                    self.drop_other_variants(&choose.path);
                }
                self.dirty = true;
            }
            _ => self.mode = Mode::Choose(choose),
        }
    }

    /// After a tag changed: the keys only the other variants have go.
    fn drop_other_variants(&mut self, tag_path: &[Seg]) {
        let table_path = &tag_path[..tag_path.len() - 1];
        let Some(Node::Tagged {
            tag,
            common,
            variants,
        }) = self.node_at(table_path)
        else {
            return;
        };
        let keep = tagged_table(&tag, &common, &variants, get(&self.values, table_path));
        if let Some(Value::Object(values)) = get_mut(&mut self.values, table_path) {
            let others: Vec<String> = variants
                .iter()
                .flat_map(|variant| variant.table.entries.iter().map(|e| e.key.clone()))
                .filter(|key| !keep.entries.iter().any(|entry| &entry.key == key))
                .collect();
            values.retain(|key, _| !others.contains(key));
        }
    }

    fn confirm(&mut self, confirm: &Confirm, key: KeyEvent) -> Effect {
        if !matches!(key.code, KeyCode::Char('y' | 'Y') | KeyCode::Enter) {
            return Effect::None;
        }
        match confirm {
            Confirm::Quit => return Effect::Quit,
            Confirm::Delete { path } => {
                remove(&mut self.values, path);
                self.dirty = true;
                let count = self.rows().len();
                let screen = self.screen_mut();
                screen.cursor = screen.cursor.min(count.saturating_sub(1));
            }
            Confirm::Regenerate { var } => {
                let var = var.clone();
                self.set_token(&var);
            }
        }
        Effect::None
    }
}

fn find(table: Table, key: &str) -> Option<Node> {
    table
        .entries
        .into_iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.node)
}

/// A tagged table's options: the common ones and the chosen variant's.
fn tagged_table(tag: &str, common: &Table, variants: &[Variant], value: Option<&Value>) -> Table {
    let mut table = common.clone();
    let chosen = value
        .and_then(|value| value.get(tag))
        .and_then(Value::as_str);
    if let Some(variant) = variants
        .iter()
        .find(|variant| Some(variant.value.as_str()) == chosen)
    {
        table.entries.extend(variant.table.entries.iter().cloned());
    }
    table
}

/// A value written in place (`key = value`), not a screen of its own.
fn is_value(node: &Node) -> bool {
    match node {
        Node::Value(_) => true,
        Node::List(inner) | Node::Map(inner) => matches!(**inner, Node::Value(_)),
        _ => false,
    }
}

/// A list item's label: its id, name or title, else its first text.
fn label_of(item: &Value) -> String {
    let Some(object) = item.as_object() else {
        return describe::toml_value(item);
    };
    ["id", "name", "title"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .or_else(|| object.values().find_map(Value::as_str))
        .unwrap_or("(new)")
        .to_owned()
}

fn display(node: &Node, value: &Value) -> String {
    match (node, value) {
        (Node::Value(ValueKind::Secret), Value::String(text)) if !text.is_empty() => {
            "••••••".into()
        }
        (_, Value::String(text)) => text.clone(),
        (Node::Value(ValueKind::TextList), Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map_or_else(|| item.to_string(), str::to_owned)
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => describe::toml_value(value),
    }
}

fn edit_text(kind: &ValueKind, value: &Value) -> String {
    match (kind, value) {
        (_, Value::String(text)) => text.clone(),
        (ValueKind::TextList, Value::Array(items))
            if items
                .iter()
                .all(|item| item.as_str().is_some_and(|text| !text.contains(','))) =>
        {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        }
        _ => value.to_string(),
    }
}

/// The value an input stands for; `None` (an empty input) unsets it.
fn parse(kind: &ValueKind, text: &str) -> Result<Option<Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value = match kind {
        ValueKind::Text | ValueKind::Secret => Value::String(text.to_owned()),
        ValueKind::EnvVar(_) => Value::String(trimmed.to_owned()),
        ValueKind::Bool => match trimmed {
            "true" | "yes" => Value::Bool(true),
            "false" | "no" => Value::Bool(false),
            _ => return Err("true or false.".into()),
        },
        ValueKind::Int { min, max } => {
            let number: i64 = trimmed
                .parse()
                .map_err(|_| "A whole number, please.".to_owned())?;
            if number < *min || number > *max {
                return Err(format!("From {min} to {max}."));
            }
            Value::from(number)
        }
        ValueKind::Float => {
            let number: f64 = trimmed
                .parse()
                .map_err(|_| "A number, please.".to_owned())?;
            serde_json::Number::from_f64(number)
                .map(Value::Number)
                .ok_or_else(|| "A finite number, please.".to_owned())?
        }
        ValueKind::OneOf(choices) => {
            if !choices.iter().any(|choice| choice == trimmed) {
                return Err(format!("One of: {}.", choices.join(", ")));
            }
            Value::String(trimmed.to_owned())
        }
        ValueKind::TextList if trimmed.starts_with('[') => {
            let items: Vec<String> = serde_json::from_str(trimmed)
                .map_err(|error| format!("A JSON list of texts: {error}"))?;
            Value::from(items)
        }
        ValueKind::TextList => Value::from(
            trimmed
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>(),
        ),
        ValueKind::Json => {
            serde_json::from_str(trimmed).map_err(|error| format!("Not JSON: {error}"))?
        }
    };
    Ok(Some(value))
}

// ---- Paths into the values.

pub fn get<'a>(root: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    path.iter().try_fold(root, |value, seg| match seg {
        Seg::Key(key) => value.get(key),
        Seg::Index(index) => value.get(index),
    })
}

fn get_mut<'a>(root: &'a mut Value, path: &[Seg]) -> Option<&'a mut Value> {
    path.iter().try_fold(root, |value, seg| match seg {
        Seg::Key(key) => value.get_mut(key),
        Seg::Index(index) => value.get_mut(index),
    })
}

/// Sets the value at `path`, making the tables on the way.
fn set(root: &mut Value, path: &[Seg], new: Value) {
    let Some((last, parents)) = path.split_last() else {
        *root = new;
        return;
    };
    let mut value = root;
    for seg in parents {
        value = match seg {
            Seg::Key(key) => {
                if !value.is_object() {
                    *value = Value::Object(Map::new());
                }
                value
                    .as_object_mut()
                    .expect("made an object")
                    .entry(key.clone())
                    .or_insert_with(|| Value::Object(Map::new()))
            }
            Seg::Index(index) => match value.get_mut(*index) {
                Some(item) => item,
                None => return,
            },
        };
    }
    match last {
        Seg::Key(key) => {
            if !value.is_object() {
                *value = Value::Object(Map::new());
            }
            value
                .as_object_mut()
                .expect("made an object")
                .insert(key.clone(), new);
        }
        Seg::Index(index) => {
            if let Some(item) = value.get_mut(*index) {
                *item = new;
            }
        }
    }
}

fn remove(root: &mut Value, path: &[Seg]) -> Option<Value> {
    let (last, parents) = path.split_last()?;
    let parent = get_mut(root, parents)?;
    match (last, parent) {
        (Seg::Key(key), Value::Object(map)) => map.remove(key),
        (Seg::Index(index), Value::Array(items)) if *index < items.len() => {
            Some(items.remove(*index))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
