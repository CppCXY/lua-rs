use crate::{FromLua, FromLuaMulti, Function, IntoLua, LuaResult, LuaTableRef, LuaValue};

/// Safe wrapper around a Lua table handle.
#[derive(Clone, Debug)]
pub struct Table {
    pub(crate) inner: LuaTableRef,
}

impl Table {
    pub(crate) fn new(inner: LuaTableRef) -> Self {
        Table { inner }
    }

    /// Get and convert a field.
    #[inline]
    pub fn get<T: FromLua>(&self, key: impl IntoLua) -> LuaResult<T> {
        self.inner.get_typed(key)
    }

    /// Look up a nested path of string keys.
    pub fn get_path<T: FromLua>(&self, path: &[&str]) -> LuaResult<T> {
        let mut current = self.clone();
        let Some((last, prefix)) = path.split_last() else {
            return Err(luars::LuaError::RuntimeError);
        };

        for key in prefix {
            current = current.get::<Table>(*key)?;
        }

        current.get(*last)
    }

    /// Returns true if the table contains a non-nil value for `key`.
    #[inline]
    pub fn contains_key(&self, key: impl IntoLua) -> LuaResult<bool> {
        self.inner.contains_key(key)
    }

    /// Set a field.
    #[inline]
    pub fn set(&self, key: impl IntoLua, value: impl IntoLua) -> LuaResult<()> {
        self.inner.set_typed(key, value)
    }

    /// Get a named function field and call it with `args`.
    #[inline]
    pub fn call_function<A: IntoLua, R: FromLuaMulti>(&self, name: &str, args: A) -> LuaResult<R> {
        self.get::<Function>(name)?.call(args)
    }

    /// Get a named method field and call it with `self` as the first argument.
    #[inline]
    pub fn call_method<A: IntoLua, R: FromLuaMulti>(&self, name: &str, args: A) -> LuaResult<R> {
        self.get::<Function>(name)?.call((self.clone(), args))
    }

    /// Get a named method field and call it, converting only the first result.
    #[inline]
    pub fn call_method1<A: IntoLua, R: FromLua>(&self, name: &str, args: A) -> LuaResult<R> {
        self.get::<Function>(name)?.call1((self.clone(), args))
    }

    /// Get a field without metamethods.
    #[inline]
    pub fn raw_get<T: FromLua>(&self, key: impl IntoLua) -> LuaResult<T> {
        self.inner.get_typed(key)
    }

    /// Set a field without metamethods.
    #[inline]
    pub fn raw_set(&self, key: impl IntoLua, value: impl IntoLua) -> LuaResult<()> {
        self.inner.set_typed(key, value)
    }

    /// Return the array length (`#t`).
    #[inline]
    pub fn len(&self) -> LuaResult<usize> {
        self.inner.len()
    }

    /// Return the raw array length.
    #[inline]
    pub fn raw_len(&self) -> LuaResult<usize> {
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

    /// Return all pairs converted to Rust types.
    #[inline]
    pub fn pairs<K: FromLua, V: FromLua>(&self) -> LuaResult<Vec<(K, V)>> {
        self.inner.pairs_typed()
    }

    /// Return the contiguous sequence values from `1..` until nil.
    #[inline]
    pub fn sequence_values<T: FromLua>(&self) -> LuaResult<Vec<T>> {
        self.inner.sequence_values()
    }

    /// Append a value to the array portion of the table.
    #[inline]
    pub fn push(&self, value: impl IntoLua) -> LuaResult<()> {
        self.inner.push_typed(value)
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
