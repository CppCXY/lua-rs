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

    /// Convert this table into a JSON value using the crate's existing `serde` bridge.
    #[cfg(feature = "serde")]
    pub fn to_json_value(&self) -> Result<serde_json::Value, String> {
        crate::serde::lua_to_json(&self.value())
    }

    /// Convert this table into a JSON string using the crate's existing `serde` bridge.
    #[cfg(feature = "serde")]
    pub fn to_json_string(&self, pretty: bool) -> Result<String, String> {
        crate::serde::lua_to_json_string(&self.value(), pretty)
    }

    /// Decode this Lua table into any serde-deserializable Rust value.
    #[cfg(feature = "serde")]
    pub fn to_serde<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        let json = self.to_json_value()?;
        serde_json::from_value(json).map_err(|err| format!("Failed to deserialize table: {}", err))
    }

    /// Construct a Lua table from a JSON value inside the provided Lua runtime.
    #[cfg(feature = "serde")]
    pub fn from_json_value(lua: &mut crate::Lua, json: &serde_json::Value) -> LuaResult<Self> {
        let vm = unsafe { lua.vm_mut() };
        let value = vm
            .deserialize_from_json(json)
            .map_err(|msg| vm.error(msg))?;
        Table::from_lua(value, vm.main_state()).map_err(|msg| vm.error(msg))
    }

    /// Construct a Lua table from a JSON string inside the provided Lua runtime.
    #[cfg(feature = "serde")]
    pub fn from_json_str(lua: &mut crate::Lua, json: &str) -> LuaResult<Self> {
        let vm = unsafe { lua.vm_mut() };
        let value = vm
            .deserialize_from_json_string(json)
            .map_err(|msg| vm.error(msg))?;
        Table::from_lua(value, vm.main_state()).map_err(|msg| vm.error(msg))
    }

    /// Construct a Lua table from any serde-serializable Rust value.
    #[cfg(feature = "serde")]
    pub fn from_serde<T: serde::Serialize>(lua: &mut crate::Lua, value: &T) -> LuaResult<Self> {
        let json = match serde_json::to_value(value) {
            Ok(json) => json,
            Err(err) => {
                let vm = unsafe { lua.vm_mut() };
                return Err(vm.error(err.to_string()));
            }
        };
        Self::from_json_value(lua, &json)
    }

    pub(crate) fn value(&self) -> luars::LuaValue {
        self.inner.to_value()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Table {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(
            &self.to_json_value().map_err(serde::ser::Error::custom)?,
            serializer,
        )
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
