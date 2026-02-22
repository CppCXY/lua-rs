/// Lua reference mechanism (similar to luaL_ref/luaL_unref in C API)
///
/// This module provides a way to store Lua values in the registry and get a stable reference to them.
/// This is useful for keeping values alive across GC cycles and for passing values between Rust and Lua.
use std::marker::PhantomData;

use crate::lua_value::LuaValue;
use crate::lua_value::LuaValueKind;

/// A reference ID in the registry.
/// Similar to Lua's luaL_ref return value.
pub type RefId = i32;

/// Special reference constants (matching Lua's C API)
pub const LUA_REFNIL: RefId = -1; // Reference to nil (no storage needed)
pub const LUA_NOREF: RefId = -2; // Invalid reference

/// Internal state for managing references in the registry
pub(crate) struct RefManager {
    /// Next available reference ID
    next_ref_id: RefId,

    /// Free list of released reference IDs (for reuse)
    free_list: Vec<RefId>,
}

impl RefManager {
    pub fn new() -> Self {
        RefManager {
            next_ref_id: 1, // Start from 1, reserve negatives for special values
            free_list: Vec::new(),
        }
    }

    /// Allocate a new reference ID
    pub fn alloc_ref_id(&mut self) -> RefId {
        if let Some(ref_id) = self.free_list.pop() {
            ref_id
        } else {
            let ref_id = self.next_ref_id;
            self.next_ref_id = self.next_ref_id.wrapping_add(1);
            if self.next_ref_id < 0 {
                // Wrapped around, skip special values
                self.next_ref_id = 1;
            }
            ref_id
        }
    }

    /// Free a reference ID (add to free list for reuse)
    pub fn free_ref_id(&mut self, ref_id: RefId) {
        if ref_id > 0 && !self.free_list.contains(&ref_id) {
            self.free_list.push(ref_id);
        }
    }
}

/// A reference to a Lua value stored in the VM's registry.
///
/// This is similar to Lua's C API luaL_ref mechanism:
/// - For GC objects, stores them in the registry and keeps a reference ID
/// - For simple values (numbers, booleans, nil), stores them directly
/// - Must be manually released with vm.release_ref() or holds the value forever
///
/// # Examples
/// ```ignore
/// // Create a reference to a table
/// let table_ref = vm.create_ref(table_value);
///
/// // Get the value back
/// let value = vm.get_ref_value(&table_ref);
///
/// // Release the reference when done
/// vm.release_ref(table_ref);
/// ```
pub struct LuaRefValue {
    /// The actual storage
    inner: LuaRefInner,
}

enum LuaRefInner {
    /// Direct storage for non-GC values (numbers, booleans, nil)
    Direct(LuaValue),

    /// Registry-based storage for GC objects (tables, strings, functions, etc.)
    /// Stores the reference ID
    Registry { ref_id: RefId },
}

impl LuaRefValue {
    /// Create a new reference for a direct value (non-GC object)
    pub(crate) fn new_direct(value: LuaValue) -> Self {
        LuaRefValue {
            inner: LuaRefInner::Direct(value),
        }
    }

    /// Create a new reference with a registry ID
    pub(crate) fn new_registry(ref_id: RefId) -> Self {
        LuaRefValue {
            inner: LuaRefInner::Registry { ref_id },
        }
    }

    /// Get the reference ID (if stored in registry)
    pub fn ref_id(&self) -> Option<RefId> {
        match &self.inner {
            LuaRefInner::Registry { ref_id } => Some(*ref_id),
            LuaRefInner::Direct(_) => None,
        }
    }

    /// Get the Lua value from this reference (requires VM access)
    pub fn get(&self, vm: &super::LuaVM) -> LuaValue {
        match &self.inner {
            LuaRefInner::Direct(value) => *value,
            LuaRefInner::Registry { ref_id } => {
                // Look up in registry
                vm.registry_geti(*ref_id as i64).unwrap_or_default()
            }
        }
    }

    /// Get direct value if this is a direct reference
    pub fn get_direct(&self) -> Option<&LuaValue> {
        match &self.inner {
            LuaRefInner::Direct(value) => Some(value),
            LuaRefInner::Registry { .. } => None,
        }
    }

    /// Check if this is a valid reference
    pub fn is_valid(&self) -> bool {
        match &self.inner {
            LuaRefInner::Direct(_) => true,
            LuaRefInner::Registry { ref_id } => *ref_id > 0,
        }
    }

    /// Check if stored in registry
    pub fn is_registry_ref(&self) -> bool {
        matches!(&self.inner, LuaRefInner::Registry { .. })
    }

    /// Convert to a raw reference ID (for C API compatibility)
    pub fn to_raw_ref(&self) -> RefId {
        match &self.inner {
            LuaRefInner::Direct(value) => {
                if value.is_nil() {
                    LUA_REFNIL
                } else {
                    LUA_NOREF
                }
            }
            LuaRefInner::Registry { ref_id } => *ref_id,
        }
    }
}

impl Clone for LuaRefValue {
    fn clone(&self) -> Self {
        match &self.inner {
            LuaRefInner::Direct(value) => LuaRefValue::new_direct(*value),
            LuaRefInner::Registry { ref_id } => {
                // For registry references, we just copy the ID
                // The caller is responsible for managing the lifecycle
                // (this matches Lua's C API behavior where ref IDs can be copied)
                LuaRefValue::new_registry(*ref_id)
            }
        }
    }
}

impl std::fmt::Debug for LuaRefValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner {
            LuaRefInner::Direct(value) => {
                write!(f, "LuaRefValue::Direct({:?})", value)
            }
            LuaRefInner::Registry { ref_id } => {
                write!(f, "LuaRefValue::Registry(ref_id={})", ref_id)
            }
        }
    }
}

// ============================================================================
// User-facing Ref types (mlua-inspired)
// ============================================================================

/// Internal core shared by all user-facing Ref types.
///
/// Holds a registry reference ID and a raw pointer to the owning `LuaVM`.
/// Automatically releases the registry entry on `Drop` (RAII).
///
/// `!Send + !Sync` by design — Lua VM is single-threaded.
struct RefInner {
    ref_id: RefId,
    vm: *mut super::LuaVM,
    /// Makes RefInner !Send + !Sync
    _marker: PhantomData<*const ()>,
}

impl RefInner {
    /// Create a new RefInner. The value must already be stored in the registry.
    fn new(ref_id: RefId, vm: *mut super::LuaVM) -> Self {
        RefInner {
            ref_id,
            vm,
            _marker: PhantomData,
        }
    }

    /// Retrieve the LuaValue from the registry.
    #[inline]
    fn to_value(&self) -> LuaValue {
        let vm = unsafe { &*self.vm };
        vm.registry_geti(self.ref_id as i64)
            .unwrap_or(LuaValue::nil())
    }

    /// Get a reference to the VM.
    #[inline]
    fn vm(&self) -> &super::LuaVM {
        unsafe { &*self.vm }
    }

    /// Get a mutable reference to the VM.
    #[allow(clippy::mut_from_ref)]
    #[inline]
    fn vm_mut(&self) -> &mut super::LuaVM {
        unsafe { &mut *self.vm }
    }
}

impl Drop for RefInner {
    fn drop(&mut self) {
        if self.ref_id > 0 && !self.vm.is_null() {
            unsafe {
                (*self.vm).release_ref_id(self.ref_id);
            }
        }
    }
}

impl std::fmt::Debug for RefInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RefInner(ref_id={})", self.ref_id)
    }
}

// ---- helper: create a registry ref for a LuaValue ---------------------------

/// Store a LuaValue in the VM registry and return its RefId.
pub(crate) fn store_in_registry(vm: &mut super::LuaVM, value: LuaValue) -> RefId {
    let ref_id = vm.ref_manager.alloc_ref_id();
    vm.registry_seti(ref_id as i64, value);
    ref_id
}

// ============================================================================
// LuaTableRef
// ============================================================================

/// A user-facing reference to a Lua table.
///
/// Holds the table in the VM registry so it won't be garbage-collected.
/// The registry entry is automatically released when this value is dropped.
///
/// `!Send + !Sync` — cannot be transferred across threads.
///
/// # Example
///
/// ```ignore
/// let tbl = vm.create_table_ref(0, 4)?;
/// tbl.set("name", LuaValue::from("Alice"))?;
/// let name: String = tbl.get_as("name")?;
/// // tbl is automatically released here
/// ```
pub struct LuaTableRef {
    inner: RefInner,
}

impl LuaTableRef {
    /// Create from an already-registered ref id. The caller guarantees the
    /// value at `ref_id` is a table.
    pub(crate) fn from_raw(ref_id: RefId, vm: *mut super::LuaVM) -> Self {
        LuaTableRef {
            inner: RefInner::new(ref_id, vm),
        }
    }

    // ==================== Read ====================

    /// Get a value by string key (raw access, no metamethods).
    pub fn get(&self, key: &str) -> super::LuaResult<LuaValue> {
        let vm = self.inner.vm_mut();
        let table = self.inner.to_value();
        let key_val = vm.create_string(key)?;
        Ok(vm.raw_get(&table, &key_val).unwrap_or_default())
    }

    /// Get a value by integer key.
    pub fn geti(&self, key: i64) -> super::LuaResult<LuaValue> {
        let vm = self.inner.vm();
        let table = self.inner.to_value();
        Ok(vm.raw_geti(&table, key).unwrap_or_default())
    }

    /// Get a value by arbitrary LuaValue key.
    pub fn get_value(&self, key: &LuaValue) -> super::LuaResult<LuaValue> {
        let vm = self.inner.vm();
        let table = self.inner.to_value();
        Ok(vm.raw_get(&table, key).unwrap_or_default())
    }

    /// Get a value by string key and convert to a Rust type via `FromLua`.
    pub fn get_as<T: crate::FromLua>(&self, key: &str) -> super::LuaResult<T> {
        let val = self.get(key)?;
        let vm = self.inner.vm_mut();
        T::from_lua(val, vm.main_state()).map_err(|msg| vm.error(msg))
    }

    // ==================== Write ====================

    /// Set a string-keyed value.
    pub fn set(&self, key: &str, value: LuaValue) -> super::LuaResult<()> {
        let vm = self.inner.vm_mut();
        let table = self.inner.to_value();
        let key_val = vm.create_string(key)?;
        vm.raw_set(&table, key_val, value);
        Ok(())
    }

    /// Set an integer-keyed value.
    pub fn seti(&self, key: i64, value: LuaValue) -> super::LuaResult<()> {
        let vm = self.inner.vm_mut();
        let table = self.inner.to_value();
        vm.raw_seti(&table, key, value);
        Ok(())
    }

    /// Set an arbitrary key-value pair.
    pub fn set_value(&self, key: LuaValue, value: LuaValue) -> super::LuaResult<()> {
        let vm = self.inner.vm_mut();
        let table = self.inner.to_value();
        vm.raw_set(&table, key, value);
        Ok(())
    }

    // ==================== Iteration ====================

    /// Get all key-value pairs (snapshot, no metamethods).
    pub fn pairs(&self) -> super::LuaResult<Vec<(LuaValue, LuaValue)>> {
        let vm = self.inner.vm();
        let table = self.inner.to_value();
        vm.table_pairs(&table)
    }

    /// Get the array length (equivalent to Lua's `#t`).
    pub fn len(&self) -> super::LuaResult<usize> {
        let vm = self.inner.vm();
        let table = self.inner.to_value();
        vm.table_length(&table)
    }

    /// Append a value to the array part (equivalent to `table.insert`).
    pub fn push(&self, value: LuaValue) -> super::LuaResult<()> {
        let current_len = self.len()?;
        self.seti((current_len + 1) as i64, value)
    }

    // ==================== Conversion ====================

    /// Get the underlying LuaValue (retrieved from registry).
    pub fn to_value(&self) -> LuaValue {
        self.inner.to_value()
    }

    /// Get the registry reference ID.
    pub fn ref_id(&self) -> RefId {
        self.inner.ref_id
    }
}

impl std::fmt::Debug for LuaTableRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LuaTableRef(ref_id={})", self.inner.ref_id)
    }
}

// ============================================================================
// LuaFunctionRef
// ============================================================================

/// A user-facing reference to a Lua function (Lua closure, C function, or Rust closure).
///
/// The function value is held in the VM registry and released on drop.
///
/// `!Send + !Sync`.
///
/// # Example
///
/// ```ignore
/// let greet = vm.get_global_function("greet")?.unwrap();
/// let result = greet.call(vec![vm.create_string("World")?])?;
/// ```
pub struct LuaFunctionRef {
    inner: RefInner,
}

impl LuaFunctionRef {
    pub(crate) fn from_raw(ref_id: RefId, vm: *mut super::LuaVM) -> Self {
        LuaFunctionRef {
            inner: RefInner::new(ref_id, vm),
        }
    }

    /// Call the function synchronously.
    pub fn call(&self, args: Vec<LuaValue>) -> super::LuaResult<Vec<LuaValue>> {
        let vm = self.inner.vm_mut();
        let func = self.inner.to_value();
        vm.call(func, args)
    }

    /// Call the function and return the first result (or nil if no results).
    pub fn call1(&self, args: Vec<LuaValue>) -> super::LuaResult<LuaValue> {
        let results = self.call(args)?;
        Ok(results.into_iter().next().unwrap_or(LuaValue::nil()))
    }

    /// Call the function asynchronously.
    pub async fn call_async(&self, args: Vec<LuaValue>) -> super::LuaResult<Vec<LuaValue>> {
        let vm = self.inner.vm_mut();
        let func = self.inner.to_value();
        vm.call_async(func, args).await
    }

    /// Get the underlying LuaValue.
    pub fn to_value(&self) -> LuaValue {
        self.inner.to_value()
    }

    /// Get the registry reference ID.
    pub fn ref_id(&self) -> RefId {
        self.inner.ref_id
    }
}

impl std::fmt::Debug for LuaFunctionRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LuaFunctionRef(ref_id={})", self.inner.ref_id)
    }
}

// ============================================================================
// LuaStringRef
// ============================================================================

/// A user-facing reference to a Lua string.
///
/// `!Send + !Sync`.
pub struct LuaStringRef {
    inner: RefInner,
}

impl LuaStringRef {
    pub(crate) fn from_raw(ref_id: RefId, vm: *mut super::LuaVM) -> Self {
        LuaStringRef {
            inner: RefInner::new(ref_id, vm),
        }
    }

    /// Get the string content. The returned `&str` is valid as long as the
    /// underlying GC string is alive (guaranteed by the registry ref).
    pub fn as_str(&self) -> Option<&str> {
        let value = self.inner.to_value();
        // Safety: the registry ref keeps the GcString alive, and we return
        // a reference whose lifetime is tied to `&self`.
        // LuaValue::as_str() dereferences the GcString pointer directly.
        // The GC won't collect it because the registry holds a reference.
        value.as_str().map(|s| {
            // Extend lifetime from the temporary to &self.
            // This is safe because the GC object is pinned by the registry.
            unsafe { &*(s as *const str) }
        })
    }

    /// Copy the string content into an owned String.
    pub fn to_string_lossy(&self) -> String {
        self.as_str().unwrap_or("").to_owned()
    }

    /// Get the byte length.
    pub fn byte_len(&self) -> usize {
        self.as_str().map(|s| s.len()).unwrap_or(0)
    }

    /// Get the underlying LuaValue.
    pub fn to_value(&self) -> LuaValue {
        self.inner.to_value()
    }

    /// Get the registry reference ID.
    pub fn ref_id(&self) -> RefId {
        self.inner.ref_id
    }
}

impl std::fmt::Debug for LuaStringRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LuaStringRef(ref_id={}, {:?})",
            self.inner.ref_id,
            self.as_str()
        )
    }
}

impl std::fmt::Display for LuaStringRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str().unwrap_or(""))
    }
}

// ============================================================================
// LuaAnyRef
// ============================================================================

/// A generic user-facing reference to any Lua value.
///
/// Can be down-cast to a typed ref (`LuaTableRef`, `LuaFunctionRef`, `LuaStringRef`)
/// when the concrete type is known.
///
/// `!Send + !Sync`.
///
/// # Example
///
/// ```ignore
/// let any = vm.to_ref(some_value);
/// if let Some(tbl) = any.as_table() {
///     tbl.set("key", LuaValue::integer(1))?;
/// }
/// ```
pub struct LuaAnyRef {
    inner: RefInner,
}

impl LuaAnyRef {
    pub(crate) fn from_raw(ref_id: RefId, vm: *mut super::LuaVM) -> Self {
        LuaAnyRef {
            inner: RefInner::new(ref_id, vm),
        }
    }

    /// Get the underlying LuaValue.
    pub fn to_value(&self) -> LuaValue {
        self.inner.to_value()
    }

    /// Try to convert to a `LuaTableRef`. Returns `None` if the value is not a table.
    /// **Creates a new registry entry** so that both refs are independent.
    pub fn as_table(&self) -> Option<LuaTableRef> {
        let value = self.inner.to_value();
        if !value.is_table() {
            return None;
        }
        let vm = self.inner.vm_mut();
        let ref_id = store_in_registry(vm, value);
        Some(LuaTableRef::from_raw(ref_id, self.inner.vm))
    }

    /// Try to convert to a `LuaFunctionRef`.
    pub fn as_function(&self) -> Option<LuaFunctionRef> {
        let value = self.inner.to_value();
        if !value.is_function() {
            return None;
        }
        let vm = self.inner.vm_mut();
        let ref_id = store_in_registry(vm, value);
        Some(LuaFunctionRef::from_raw(ref_id, self.inner.vm))
    }

    /// Try to convert to a `LuaStringRef`.
    pub fn as_string(&self) -> Option<LuaStringRef> {
        let value = self.inner.to_value();
        if !value.is_string() {
            return None;
        }
        let vm = self.inner.vm_mut();
        let ref_id = store_in_registry(vm, value);
        Some(LuaStringRef::from_raw(ref_id, self.inner.vm))
    }

    /// Get the value's type kind.
    pub fn kind(&self) -> LuaValueKind {
        self.inner.to_value().kind()
    }

    /// Extract the value as a Rust type via `FromLua`.
    pub fn get_as<T: crate::FromLua>(&self) -> super::LuaResult<T> {
        let val = self.inner.to_value();
        let vm = self.inner.vm_mut();
        T::from_lua(val, vm.main_state()).map_err(|msg| vm.error(msg))
    }

    /// Get the registry reference ID.
    pub fn ref_id(&self) -> RefId {
        self.inner.ref_id
    }
}

impl std::fmt::Debug for LuaAnyRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LuaAnyRef(ref_id={}, kind={:?})",
            self.inner.ref_id,
            self.kind()
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{LuaVM, lua_vm::SafeOption};

    use super::*;

    #[test]
    fn test_lua_ref_mechanism() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Create some test values
        let table = vm.create_table(0, 2).unwrap();
        let num_key = vm.create_string("num").unwrap();
        let str_key = vm.create_string("str").unwrap();
        let str_val = vm.create_string("hello").unwrap();
        vm.raw_set(&table, num_key, LuaValue::number(42.0));
        vm.raw_set(&table, str_key, str_val);

        let number = LuaValue::number(123.456);
        let nil_val = LuaValue::nil();

        // Test 1: Create references
        let table_ref = vm.create_ref(table);
        let number_ref = vm.create_ref(number);
        let nil_ref = vm.create_ref(nil_val);

        // Verify reference types
        assert!(table_ref.is_registry_ref(), "Table should use registry");
        assert!(!number_ref.is_registry_ref(), "Number should be direct");
        assert!(!nil_ref.is_registry_ref(), "Nil should be direct");

        // Test 2: Retrieve values through references
        let retrieved_table = vm.get_ref_value(&table_ref);
        assert!(retrieved_table.is_table(), "Should retrieve table");

        let retrieved_num = vm.get_ref_value(&number_ref);
        assert_eq!(
            retrieved_num.as_number(),
            Some(123.456),
            "Should retrieve number"
        );

        let retrieved_nil = vm.get_ref_value(&nil_ref);
        assert!(retrieved_nil.is_nil(), "Should retrieve nil");

        // Test 3: Verify table contents
        let num_key2 = vm.create_string("num").unwrap();
        let val = vm.raw_get(&retrieved_table, &num_key2);
        assert_eq!(
            val.and_then(|v| v.as_number()),
            Some(42.0),
            "Table content should be preserved"
        );

        // Test 4: Get ref IDs
        let table_ref_id = table_ref.ref_id();
        assert!(table_ref_id.is_some(), "Table ref should have ID");
        assert!(table_ref_id.unwrap() > 0, "Ref ID should be positive");

        let number_ref_id = number_ref.ref_id();
        assert!(number_ref_id.is_none(), "Number ref should not have ID");

        // Test 5: Release references
        vm.release_ref(table_ref);
        vm.release_ref(number_ref);
        vm.release_ref(nil_ref);

        // Test 6: After release, ref should return nil
        let after_release = vm.get_ref_value_by_id(table_ref_id.unwrap());
        assert!(after_release.is_nil(), "Released ref should return nil");

        println!("✓ Lua ref mechanism test passed");
    }

    #[test]
    fn test_ref_id_reuse() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Create and release multiple refs to test ID reuse
        let t1 = vm.create_table(0, 0).unwrap();
        let ref1 = vm.create_ref(t1);
        let id1 = ref1.ref_id().unwrap();

        vm.release_ref(ref1);

        // Create another ref - should reuse the ID
        let t2 = vm.create_table(0, 0).unwrap();
        let ref2 = vm.create_ref(t2);
        let id2 = ref2.ref_id().unwrap();

        assert_eq!(id1, id2, "Ref IDs should be reused");

        vm.release_ref(ref2);

        println!("✓ Ref ID reuse test passed");
    }

    #[test]
    fn test_multiple_refs() {
        let mut vm = LuaVM::new(SafeOption::default());

        // Create multiple refs and verify they don't interfere
        let mut refs = Vec::new();
        for i in 0..10 {
            let table = vm.create_table(0, 1).unwrap();
            let key = vm.create_string("value").unwrap();
            let num_val = LuaValue::number(i as f64);
            vm.raw_set(&table, key, num_val);
            refs.push(vm.create_ref(table));
        }

        // Verify all refs are still valid
        for (i, lua_ref) in refs.iter().enumerate() {
            let table = vm.get_ref_value(lua_ref);
            let key = vm.create_string("value").unwrap();
            let val = vm.raw_get(&table, &key);
            assert_eq!(
                val.and_then(|v| v.as_number()),
                Some(i as f64),
                "Ref {} should have correct value",
                i
            );
        }

        // Release all refs
        for lua_ref in refs {
            vm.release_ref(lua_ref);
        }

        println!("✓ Multiple refs test passed");
    }
}
