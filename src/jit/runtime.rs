use super::jit_value::JitValue;
use crate::value::LuaValue;

/// Runtime bridge between the interpreter's `LuaValue` registers
/// and the JIT's fixed-layout `JitValue` representation.
pub struct JitFrameState {
    registers: Vec<JitValue>,
    constants: Vec<JitValue>,
    dirty: bool,
}

impl JitFrameState {
    pub fn new(register_count: usize, constants: &[LuaValue]) -> Result<Self, String> {
        let constants = convert_constants(constants)?;
        Ok(Self {
            registers: vec![JitValue::nil(); register_count],
            constants,
            dirty: false,
        })
    }

    pub fn registers_ptr(&mut self) -> *mut JitValue {
        self.registers.as_mut_ptr()
    }

    pub fn constants_ptr(&self) -> *const JitValue {
        self.constants.as_ptr()
    }

    pub fn sync_from_lua(&mut self, registers: &[LuaValue]) -> Result<(), String> {
        self.ensure_register_capacity(registers.len());
        for (dst, src) in self.registers.iter_mut().zip(registers.iter()) {
            *dst = JitValue::try_from_lua(src)?;
        }
        self.dirty = false;
        Ok(())
    }

    pub fn spill_to_lua(&self, registers: &mut [LuaValue]) {
        for (dst, src) in registers.iter_mut().zip(self.registers.iter()) {
            *dst = src.to_lua();
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn registers(&self) -> &[JitValue] {
        &self.registers
    }

    pub fn registers_mut(&mut self) -> &mut [JitValue] {
        &mut self.registers
    }

    pub fn constants(&self) -> &[JitValue] {
        &self.constants
    }

    fn ensure_register_capacity(&mut self, len: usize) {
        if self.registers.len() < len {
            self.registers.resize(len, JitValue::nil());
        }
    }
}

pub fn convert_constants(constants: &[LuaValue]) -> Result<Vec<JitValue>, String> {
    constants
        .iter()
        .map(JitValue::try_from_lua)
        .collect()
}
