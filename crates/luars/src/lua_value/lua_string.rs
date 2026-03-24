#[cfg(feature = "shared-string")]
use std::sync::Arc;

use crate::StringInterner;

/// Internal inline byte storage for short Lua strings.
///
/// Lua strings are byte sequences; UTF-8 validity is tracked separately on
/// `LuaString` rather than encoded in the storage representation itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum InlineSize {
    _V0 = 0,
    _V1,
    _V2,
    _V3,
    _V4,
    _V5,
    _V6,
    _V7,
    _V8,
    _V9,
    _V10,
    _V11,
    _V12,
    _V13,
    _V14,
    _V15,
    _V16,
    _V17,
    _V18,
    _V19,
    _V20,
    _V21,
    _V22,
    _V23,
}

impl InlineSize {
    #[inline(always)]
    const fn as_usize(self) -> usize {
        self as u8 as usize
    }

    #[inline(always)]
    const unsafe fn from_u8(value: u8) -> Self {
        debug_assert!(value <= InlineSize::_V23 as u8);
        unsafe { std::mem::transmute::<u8, Self>(value) }
    }
}

#[derive(Clone)]
pub struct InlineShortString {
    len: InlineSize,
    bytes: [u8; InlineShortString::MAX_INLINE_LEN],
}

impl InlineShortString {
    pub const MAX_INLINE_LEN: usize = InlineSize::_V23 as usize;

    #[inline]
    pub fn new(bytes: &[u8]) -> Self {
        debug_assert!(bytes.len() <= Self::MAX_INLINE_LEN);
        let mut storage = [0; InlineShortString::MAX_INLINE_LEN];
        storage[..bytes.len()].copy_from_slice(bytes);
        Self {
            len: unsafe { InlineSize::from_u8(bytes.len() as u8) },
            bytes: storage,
        }
    }

    #[inline]
    pub fn new_str(s: &str) -> Self {
        Self::new(s.as_bytes())
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len.as_usize()
    }

    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len()]
    }
}

/// Immutable byte storage for Lua strings.
#[derive(Clone)]
pub enum LuaStrRepr {
    /// Compact inline storage for the hottest very short strings.
    Smol(InlineShortString),
    /// Shared storage for short strings that exceed the inline threshold.
    #[cfg(feature = "shared-string")]
    Shared(Arc<[u8]>),
    /// Heap-backed storage for any non-inline string payload.
    Heap(Box<[u8]>),
}

impl LuaStrRepr {
    #[inline(always)]
    pub fn len(&self) -> usize {
        match self {
            Self::Smol(s) => s.len(),
            #[cfg(feature = "shared-string")]
            Self::Shared(s) => s.len(),
            Self::Heap(s) => s.len(),
        }
    }

    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Smol(s) => s.as_bytes(),
            #[cfg(feature = "shared-string")]
            Self::Shared(s) => s.as_ref(),
            Self::Heap(s) => s.as_ref(),
        }
    }

    #[cfg(feature = "shared-string")]
    #[allow(unused)]
    #[inline(always)]
    pub(crate) fn shared_ptr(&self) -> Option<*const [u8]> {
        match self {
            Self::Shared(shared) => Some(Arc::as_ptr(shared)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Utf8State {
    Valid,
    Invalid,
}

impl PartialEq for LuaStrRepr {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl PartialEq<str> for LuaStrRepr {
    #[inline(always)]
    fn eq(&self, other: &str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl PartialEq<&str> for LuaStrRepr {
    #[inline(always)]
    fn eq(&self, other: &&str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for LuaStrRepr {}

impl std::fmt::Debug for LuaStrRepr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match std::str::from_utf8(self.as_bytes()) {
            Ok(s) => write!(f, "{:?}", s),
            Err(_) => write!(f, "{:?}", self.as_bytes()),
        }
    }
}

impl std::fmt::Display for LuaStrRepr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match std::str::from_utf8(self.as_bytes()) {
            Ok(s) => write!(f, "{}", s),
            Err(_) => write!(f, "{}", String::from_utf8_lossy(self.as_bytes())),
        }
    }
}

#[derive(Clone)]
pub struct LuaString {
    /// Hash comes first for cache locality: GcHeader(8B) + hash(8B) = 16B,
    /// both in the same cache line after pointer dereference.
    /// C Lua has TString.hash at offset 12 — ours is now at offset 8.
    pub hash: u64,
    pub utf8: Utf8State,
    pub str: LuaStrRepr,
}

impl std::fmt::Debug for LuaString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaString")
            .field("str", &self.str)
            .field("hash", &self.hash)
            .field("utf8", &self.utf8)
            .finish()
    }
}

impl LuaString {
    pub fn new(s: LuaStrRepr, hash: u64, utf8: Utf8State) -> Self {
        Self { hash, utf8, str: s }
    }

    #[inline(always)]
    pub fn from_utf8(s: LuaStrRepr, hash: u64) -> Self {
        Self::new(s, hash, Utf8State::Valid)
    }

    #[inline(always)]
    pub fn from_bytes(s: LuaStrRepr, hash: u64) -> Self {
        let utf8 = if std::str::from_utf8(s.as_bytes()).is_ok() {
            Utf8State::Valid
        } else {
            Utf8State::Invalid
        };
        Self::new(s, hash, utf8)
    }

    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        self.str.as_bytes()
    }

    #[inline(always)]
    pub fn as_str(&self) -> Option<&str> {
        match self.utf8 {
            Utf8State::Valid => Some(unsafe { std::str::from_utf8_unchecked(self.as_bytes()) }),
            Utf8State::Invalid => None,
        }
    }

    #[inline(always)]
    pub fn is_utf8(&self) -> bool {
        self.utf8 == Utf8State::Valid
    }

    pub fn is_short(&self) -> bool {
        self.str.len() <= StringInterner::SHORT_STRING_LIMIT
    }

    pub fn is_long(&self) -> bool {
        self.str.len() > StringInterner::SHORT_STRING_LIMIT
    }
}

impl Eq for LuaString {}

impl PartialEq for LuaString {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.str == other.str
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn test_inline_short_string_layout() {
        assert_eq!(size_of::<InlineSize>(), 1);
        assert_eq!(size_of::<InlineShortString>(), 24);
    }

    #[test]
    fn test_lua_str_repr_layout() {
        assert_eq!(size_of::<LuaStrRepr>(), 24);
    }

    #[test]
    fn test_inline_short_string_roundtrip() {
        let text = "12345678901234567890123";
        let smol = InlineShortString::new_str(text);

        assert_eq!(smol.len(), 23);
        assert_eq!(smol.as_bytes(), text.as_bytes());
    }

    #[test]
    fn test_lua_string_detects_utf8_and_binary() {
        let utf8 = LuaString::from_bytes(LuaStrRepr::Smol(InlineShortString::new_str("hello")), 1);
        let binary = LuaString::from_bytes(
            LuaStrRepr::Heap(vec![0xff, 0xfe, 0xfd].into_boxed_slice()),
            2,
        );

        assert!(utf8.is_utf8());
        assert_eq!(utf8.as_str(), Some("hello"));

        assert!(!binary.is_utf8());
        assert_eq!(binary.as_str(), None);
    }

    #[test]
    fn test_lua_str_repr_equality_is_byte_based() {
        let smol = LuaStrRepr::Smol(InlineShortString::new_str("same-bytes"));
        let heap = LuaStrRepr::Heap(Box::<[u8]>::from(&b"same-bytes"[..]));

        assert_eq!(smol, heap);
        assert_eq!(smol.as_bytes(), heap.as_bytes());
    }
}
