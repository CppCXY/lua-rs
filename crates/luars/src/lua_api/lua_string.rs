use crate::{FromLua, IntoLua, LuaState, LuaStringRef, LuaValue};

/// Safe handle to a Lua string kept alive in the registry.
#[derive(Clone, Debug)]
pub struct LuaString {
    pub(crate) inner: LuaStringRef,
}

impl LuaString {
    pub(crate) fn new(inner: LuaStringRef) -> Self {
        LuaString { inner }
    }

    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        self.inner.as_str()
    }

    #[inline]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.inner.as_bytes()
    }

    #[inline]
    pub fn to_string_lossy(&self) -> String {
        self.inner.to_string_lossy()
    }

    #[inline]
    pub fn byte_len(&self) -> usize {
        self.inner.byte_len()
    }
}

impl IntoLua for LuaString {
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        state
            .push_value(self.inner.to_value())
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl IntoLua for &LuaString {
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        state
            .push_value(self.inner.to_value())
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl FromLua for LuaString {
    fn from_lua(value: LuaValue, state: &mut LuaState) -> Result<Self, String> {
        let actual = value.type_name();
        let string = state
            .vm_mut()
            .to_string_ref(value)
            .ok_or_else(|| format!("expected string, got {}", actual))?;
        Ok(LuaString::new(string))
    }
}
