//! Fluent builder for constructing Lua tables from Rust.
//!
//! `TableBuilder` collects key-value pairs and then creates the table in one
//! shot via [`build`](TableBuilder::build), which requires `&mut LuaVM` for
//! GC allocation.
//!
//! # Example
//!
//! ```ignore
//! let config = TableBuilder::new()
//!     .set("host", LuaValue::from("localhost"))
//!     .set("port", LuaValue::integer(8080))
//!     .set("debug", LuaValue::boolean(true))
//!     .build(&mut vm)?;
//! vm.set_global("config", config)?;
//! ```

use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaVM};

/// Fluent builder for Lua tables.
///
/// Collects entries (keyed and/or sequential) and materialises them into a
/// single `LuaValue` table when [`build`](Self::build) is called.
pub struct TableBuilder {
    /// Named entries (string key → value).
    entries: Vec<(TableKey, LuaValue)>,
    /// Sequential array entries (1-based).
    array: Vec<LuaValue>,
}

/// Internal key representation — deferred until build-time so we can avoid
/// requiring `&mut LuaVM` during construction.
enum TableKey {
    String(String),
    Integer(i64),
    Value(LuaValue),
}

impl TableBuilder {
    /// Create an empty builder.
    #[inline]
    pub fn new() -> Self {
        TableBuilder {
            entries: Vec::new(),
            array: Vec::new(),
        }
    }

    /// Add a string-keyed entry.
    ///
    /// ```ignore
    /// builder.set("name", LuaValue::from("Alice"))
    /// ```
    #[inline]
    pub fn set(mut self, key: &str, value: LuaValue) -> Self {
        self.entries.push((TableKey::String(key.to_owned()), value));
        self
    }

    /// Add an integer-keyed entry.
    ///
    /// ```ignore
    /// builder.set_int(1, LuaValue::integer(42))
    /// ```
    #[inline]
    pub fn set_int(mut self, key: i64, value: LuaValue) -> Self {
        self.entries.push((TableKey::Integer(key), value));
        self
    }

    /// Add a `LuaValue`-keyed entry (for non-string, non-integer keys).
    #[inline]
    pub fn set_value(mut self, key: LuaValue, value: LuaValue) -> Self {
        self.entries.push((TableKey::Value(key), value));
        self
    }

    /// Append a value to the sequential (array) part of the table.
    ///
    /// Values are assigned keys 1, 2, 3, … in order.
    ///
    /// ```ignore
    /// let list = TableBuilder::new()
    ///     .push(LuaValue::integer(10))
    ///     .push(LuaValue::integer(20))
    ///     .push(LuaValue::integer(30))
    ///     .build(&mut vm)?;
    /// // Lua: {10, 20, 30}
    /// ```
    #[inline]
    pub fn push(mut self, value: LuaValue) -> Self {
        self.array.push(value);
        self
    }

    /// Materialise the table via the VM.
    pub fn build(self, vm: &mut LuaVM) -> LuaResult<LuaValue> {
        let table = vm.create_table(self.array.len(), self.entries.len())?;

        // Array part
        for (i, val) in self.array.into_iter().enumerate() {
            let key = LuaValue::integer((i + 1) as i64);
            vm.raw_set(&table, key, val);
        }

        // Keyed part
        for (k, val) in self.entries {
            let key = match k {
                TableKey::String(s) => vm.create_string(&s)?,
                TableKey::Integer(n) => LuaValue::integer(n),
                TableKey::Value(v) => v,
            };
            vm.raw_set(&table, key, val);
        }

        Ok(table)
    }
}

impl Default for TableBuilder {
    fn default() -> Self {
        Self::new()
    }
}
