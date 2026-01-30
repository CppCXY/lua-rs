/// Lua reference mechanism (similar to luaL_ref/luaL_unref in C API)
///
/// This module provides a way to store Lua values in the registry and get a stable reference to them.
/// This is useful for keeping values alive across GC cycles and for passing values between Rust and Lua.
use crate::lua_value::LuaValue;

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
            LuaRefInner::Direct(value) => value.clone(),
            LuaRefInner::Registry { ref_id } => {
                // Look up in registry
                vm.registry_geti(*ref_id as i64).unwrap_or(LuaValue::nil())
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
            LuaRefInner::Direct(value) => LuaRefValue::new_direct(value.clone()),
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
        let table_ref = vm.create_ref(table.clone());
        let number_ref = vm.create_ref(number.clone());
        let nil_ref = vm.create_ref(nil_val.clone());

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
