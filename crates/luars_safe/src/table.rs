use luars::{FromLua, IntoLua, LuaResult, LuaTableRef, LuaValue};

/// Safe wrapper around a Lua table handle.
pub struct Table {
    pub(crate) inner: LuaTableRef,
}

impl Table {
    pub(crate) fn new(inner: LuaTableRef) -> Self {
        Table { inner }
    }

    /// Get and convert a string-keyed field.
    #[inline]
    pub fn get<T: FromLua>(&self, key: &str) -> LuaResult<T> {
        self.inner.get_as(key)
    }

    /// Return the array length (`#t`).
    #[inline]
    pub fn len(&self) -> LuaResult<usize> {
        self.inner.len()
    }

    /// Returns true if the table length is zero.
    #[inline]
    pub fn is_empty(&self) -> LuaResult<bool> {
        self.len().map(|len| len == 0)
    }

    /// Return a snapshot of all key-value pairs using raw `LuaValue`s.
    #[inline]
    pub fn pairs_raw(&self) -> LuaResult<Vec<(LuaValue, LuaValue)>> {
        self.inner.pairs()
    }

    pub(crate) fn value(&self) -> luars::LuaValue {
        self.inner.to_value()
    }
}

impl IntoLua for Table {
    #[inline]
    fn into_lua(self, state: &mut luars::LuaState) -> Result<usize, String> {
        state
            .push_value(self.inner.to_value())
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl IntoLua for &Table {
    #[inline]
    fn into_lua(self, state: &mut luars::LuaState) -> Result<usize, String> {
        state
            .push_value(self.inner.to_value())
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl FromLua for Table {
    fn from_lua(value: LuaValue, state: &mut luars::LuaState) -> Result<Self, String> {
        let actual = value.type_name();
        let table = state
            .to_table_ref(value)
            .ok_or_else(|| format!("expected table, got {}", actual))?;
        Ok(Table::new(table))
    }
}