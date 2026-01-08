use crate::{
    LuaResult, LuaValue,
    lua_value::{LuaTableImpl, lua_table::LuaInsertResult},
    lua_vm::LuaError,
};

pub struct LuaValueArray {
    pub array: Vec<LuaValue>,
}

impl LuaValueArray {
    pub fn new(capacity: usize) -> Self {
        Self {
            array: Vec::with_capacity(capacity),
        }
    }
}

impl LuaTableImpl for LuaValueArray {
    fn get_int(&self, key: i64) -> Option<LuaValue> {
        self.array.get((key - 1) as usize).copied()
    }

    fn set_int(&mut self, key: i64, value: LuaValue) -> LuaInsertResult {
        let index = (key - 1) as usize;
        if index < self.array.len() {
            self.array[index] = value;
        } else if index == self.array.len() {
            self.array.push(value);
        } else {
            return LuaInsertResult::NeedConvertToHashTable;
        }

        LuaInsertResult::Success
    }

    fn raw_get(&self, key: &LuaValue) -> Option<LuaValue> {
        if let Some(idx) = key.as_integer() {
            self.get_int(idx)
        } else if let Some(s) = key.as_str() {
            if s == "n" {
                return Some(LuaValue::integer(self.array.len() as i64));
            }
            None
        } else {
            None
        }
    }

    fn raw_set(&mut self, key: &LuaValue, value: LuaValue) -> LuaInsertResult {
        if let Some(idx) = key.as_integer() {
            self.set_int(idx, value)
        } else if let Some(s) = key.as_str() {
            if s == "n" {
                // If setting "n", resize array
                if let Some(n) = value.as_integer() {
                    let new_len = n as usize;
                    if new_len > self.array.len() {
                        self.array.resize(new_len, LuaValue::nil());
                    } else {
                        self.array.truncate(new_len);
                    }
                    LuaInsertResult::Success
                } else {
                    // Setting n to non-integer?
                    LuaInsertResult::NeedConvertToHashTable
                }
            } else {
                LuaInsertResult::NeedConvertToHashTable
            }
        } else {
            LuaInsertResult::NeedConvertToHashTable
        }
    }

    fn next(&self, input_key: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        let next_index = if input_key.is_nil() {
            0 // 第一个元素的索引是0
        } else if let Some(idx) = input_key.as_integer() {
            idx as usize // idx已经是1-based，转为0-based需要的下一个索引
        } else {
            return None;
        };

        if let Some(val) = self.array.get(next_index) {
            let key = LuaValue::integer((next_index + 1) as i64); // 返回1-based的key
            Some((key, *val))
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.array.len()
    }

    fn insert_at(&mut self, index: usize, value: LuaValue) -> LuaInsertResult {
        if index < self.array.len() {
            self.array.insert(index, value);
        } else if index == self.array.len() {
            self.array.push(value);
        } else {
            return LuaInsertResult::NeedConvertToHashTable;
        }

        LuaInsertResult::Success
    }

    fn remove_at(&mut self, index: usize) -> LuaResult<LuaValue> {
        if index < self.array.len() {
            let value = self.array.remove(index);
            Ok(value)
        } else {
            Err(LuaError::IndexOutOfBounds)
        }
    }
}
