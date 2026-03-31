use crate::{Lua, Table};
use crate::util::into_single_value;
use luars::{IntoLua, LuaResult, LuaVM};

/// Safe table builder that accepts Rust values and materializes them into a
/// `Table` only when attached to a `Lua` runtime.
pub struct TableBuilder {
    entries: Vec<(TableKey, Box<dyn BuildValue>)>,
    array: Vec<Box<dyn BuildValue>>,
}

enum TableKey {
    String(String),
    Integer(i64),
}

trait BuildValue {
    fn build(self: Box<Self>, vm: &mut LuaVM) -> LuaResult<luars::LuaValue>;
}

impl<T> BuildValue for T
where
    T: IntoLua + 'static,
{
    fn build(self: Box<Self>, vm: &mut LuaVM) -> LuaResult<luars::LuaValue> {
        into_single_value(vm, *self, "TableBuilder")
    }
}

impl TableBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        TableBuilder {
            entries: Vec::new(),
            array: Vec::new(),
        }
    }

    /// Add a string-keyed entry.
    pub fn set<T: IntoLua + 'static>(mut self, key: impl Into<String>, value: T) -> Self {
        self.entries
            .push((TableKey::String(key.into()), Box::new(value)));
        self
    }

    /// Add an integer-keyed entry.
    pub fn seti<T: IntoLua + 'static>(mut self, key: i64, value: T) -> Self {
        self.entries.push((TableKey::Integer(key), Box::new(value)));
        self
    }

    /// Append a sequential array element.
    pub fn push<T: IntoLua + 'static>(mut self, value: T) -> Self {
        self.array.push(Box::new(value));
        self
    }

    /// Build the table inside the given runtime.
    pub fn build(self, lua: &mut Lua) -> LuaResult<Table> {
        let table = lua.create_table(self.array.len(), self.entries.len())?;

        for value in self.array {
            let value = value.build(lua.vm_mut())?;
            lua.table_push(&table, value)?;
        }

        for (key, value) in self.entries {
            let value = value.build(lua.vm_mut())?;
            match key {
                TableKey::String(key) => lua.table_set(&table, &key, value)?,
                TableKey::Integer(key) => lua.table_seti(&table, key, value)?,
            }
        }

        Ok(table)
    }
}

impl Default for TableBuilder {
    fn default() -> Self {
        Self::new()
    }
}