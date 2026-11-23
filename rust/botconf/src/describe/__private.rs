//! What `#[derive(Describe)]`'s code calls; not an API.

use serde::Serialize;
use serde_json::Value;
pub use serde_json::json;

use super::{Node, Table, ValueKind, Variant};
use crate::describe::Entry;

/// A table's entries, with what its flattened fields add.
#[derive(Default)]
pub struct TableBuilder {
    entries: Vec<Entry>,
    tagged: Option<(String, Vec<Variant>)>,
}

impl TableBuilder {
    pub fn entry(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// `#[serde(flatten)]`: a table's keys join this one's; a tagged table's
    /// tag and variants make this table tagged.
    pub fn flatten(&mut self, node: Node) {
        match node {
            Node::Table(table) => self.entries.extend(table.entries),
            Node::Tagged {
                tag,
                common,
                variants,
            } => {
                assert!(
                    self.tagged.is_none(),
                    "a table can flatten one tagged enum, not two"
                );
                self.entries.extend(common.entries);
                self.tagged = Some((tag, variants));
            }
            // A catch-all for the keys the type does not name: nothing to
            // describe.
            Node::Map(_) => {}
            other => panic!("a flattened field must be a table or a map, not {other:?}"),
        }
    }

    pub fn build(self) -> Node {
        let common = Table {
            entries: self.entries,
        };
        match self.tagged {
            Some((tag, variants)) => Node::Tagged {
                tag,
                common,
                variants,
            },
            None => Node::Table(common),
        }
    }
}

/// A tagged variant's table.
pub fn table_of(node: Node) -> Table {
    match node {
        Node::Table(table) => table,
        other => panic!("a tagged variant must hold a table, not {other:?}"),
    }
}

/// `#[botconf(min = …, max = …)]` on an integer.
pub fn bounded(node: Node, min: Option<i64>, max: Option<i64>) -> Node {
    match node {
        Node::Value(ValueKind::Int {
            min: type_min,
            max: type_max,
        }) => Node::Value(ValueKind::Int {
            min: min.unwrap_or(type_min),
            max: max.unwrap_or(type_max),
        }),
        other => panic!("min and max bound an integer, not {other:?}"),
    }
}

/// A default as JSON, when its type is `Serialize` (autoref
/// specialization: `(&Json(&value)).get()` picks [`BySerialize`] when it
/// applies, else [`Otherwise`]).
pub struct Json<'a, T>(pub &'a T);

pub trait BySerialize {
    fn get(&self) -> Option<Value>;
}

impl<T: Serialize> BySerialize for Json<'_, T> {
    fn get(&self) -> Option<Value> {
        serde_json::to_value(self.0)
            .ok()
            .filter(|value| !value.is_null())
    }
}

pub trait Otherwise {
    fn get(&self) -> Option<Value>;
}

impl<T> Otherwise for &Json<'_, T> {
    fn get(&self) -> Option<Value> {
        None
    }
}
