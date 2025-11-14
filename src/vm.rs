// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture

use crate::gc::{GC, GcObjectType};
use crate::lib_registry;
use crate::lua_value::{
    Chunk, LuaFunction, LuaString, LuaTable, LuaUpvalue, LuaUserdata, LuaValue, LuaValueKind,
};
use crate::opcode::{Instruction, OpCode};
use std::cell::RefCell;
use std::rc::Rc;

pub struct VM {
    // Global environment table (_G and _ENV point to this)
    globals: Rc<RefCell<LuaTable>>,

    // Call stack
    pub frames: Vec<CallFrame>,

    // Garbage collector
    gc: GC,

    // Multi-return value buffer (temporary storage for function returns)
    pub return_values: Vec<LuaValue>,

    // Open upvalues list (for closing when frames exit)
    open_upvalues: Vec<Rc<LuaUpvalue>>,

    // Next frame ID (for tracking frames)
    next_frame_id: usize,
}

pub struct CallFrame {
    pub frame_id: usize, // Unique ID for this frame
    pub function: Rc<LuaFunction>,
    pub pc: usize,                // Program counter
    pub registers: Vec<LuaValue>, // Register file
    pub base: usize,              // Stack base for this frame
    pub result_reg: usize,        // Register to store return value
    pub num_results: usize,       // Number of expected return values
}

impl VM {
    pub fn new() -> Self {
        let mut vm = VM {
            globals: Rc::new(RefCell::new(LuaTable::new())),
            frames: Vec::new(),
            gc: GC::new(),
            return_values: Vec::new(),
            open_upvalues: Vec::new(),
            next_frame_id: 0,
        };

        // Register built-in functions
        vm.register_builtins();

        // Set _G to point to the global table itself
        let globals_ref = vm.globals.clone();
        vm.set_global("_G", LuaValue::from_table_rc(globals_ref.clone()));
        vm.set_global("_ENV", LuaValue::from_table_rc(globals_ref));

        vm
    }

    fn register_builtins(&mut self) {
        let _ = lib_registry::create_standard_registry().load_all(self);
    }

    pub fn execute(&mut self, chunk: Rc<Chunk>) -> Result<LuaValue, String> {
        // Register all constants in the chunk with GC
        self.register_chunk_constants(&chunk);

        // Create main function
        let main_func = LuaFunction {
            chunk: chunk.clone(),
            upvalues: Vec::new(),
        };

        // Create initial call frame
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let frame = CallFrame {
            frame_id,
            function: Rc::new(main_func),
            pc: 0,
            registers: vec![LuaValue::nil(); chunk.max_stack_size],
            base: 0,
            result_reg: 0,
            num_results: 0,
        };

        self.frames.push(frame);

        // Execute
        let result = self.run()?;

        // Clean up
        self.frames.clear();

        Ok(result)
    }

    fn run(&mut self) -> Result<LuaValue, String> {
        loop {
            // Check if we have frames to execute
            if self.frames.is_empty() {
                return Ok(LuaValue::nil());
            }

            let frame_idx = self.frames.len() - 1;

            // Fetch instruction
            let pc = self.frames[frame_idx].pc;
            let chunk_ptr = Rc::clone(&self.frames[frame_idx].function.chunk);

            if pc >= chunk_ptr.code.len() {
                // End of code
                self.frames.pop();
                continue;
            }

            let instr = chunk_ptr.code[pc];
            self.frames[frame_idx].pc += 1;

            // Decode and execute (interpreter path)
            let opcode = Instruction::get_opcode(instr);

            match opcode {
                OpCode::Move => self.op_move(instr)?,
                OpCode::LoadK => self.op_loadk(instr)?,
                OpCode::LoadNil => self.op_loadnil(instr)?,
                OpCode::LoadBool => self.op_loadbool(instr)?,
                OpCode::NewTable => self.op_newtable(instr)?,
                OpCode::GetTable => self.op_gettable(instr)?,
                OpCode::SetTable => self.op_settable(instr)?,
                OpCode::GetTableI => self.op_gettable_i(instr)?,
                OpCode::SetTableI => self.op_settable_i(instr)?,
                OpCode::GetTableK => self.op_gettable_k(instr)?,
                OpCode::SetTableK => self.op_settable_k(instr)?,
                OpCode::Add => self.op_add(instr)?,
                OpCode::Sub => self.op_sub(instr)?,
                OpCode::Mul => self.op_mul(instr)?,
                OpCode::Div => self.op_div(instr)?,
                OpCode::Mod => self.op_mod(instr)?,
                OpCode::Pow => self.op_pow(instr)?,
                OpCode::Unm => self.op_unm(instr)?,
                OpCode::Not => self.op_not(instr)?,
                OpCode::Len => self.op_len(instr)?,
                OpCode::Eq => self.op_eq(instr)?,
                OpCode::Lt => self.op_lt(instr)?,
                OpCode::Le => self.op_le(instr)?,
                OpCode::Ne => self.op_ne(instr)?,
                OpCode::Gt => self.op_gt(instr)?,
                OpCode::Ge => self.op_ge(instr)?,
                OpCode::And => self.op_and(instr)?,
                OpCode::Or => self.op_or(instr)?,
                OpCode::BAnd => self.op_band(instr)?,
                OpCode::BOr => self.op_bor(instr)?,
                OpCode::BXor => self.op_bxor(instr)?,
                OpCode::Shl => self.op_shl(instr)?,
                OpCode::Shr => self.op_shr(instr)?,
                OpCode::BNot => self.op_bnot(instr)?,
                OpCode::IDiv => self.op_idiv(instr)?,
                OpCode::ForPrep => self.op_forprep(instr)?,
                OpCode::ForLoop => self.op_forloop(instr)?,
                OpCode::Jmp => self.op_jmp(instr)?,
                OpCode::Test => self.op_test(instr)?,
                OpCode::TestSet => self.op_testset(instr)?,
                OpCode::Call => self.op_call(instr)?,
                OpCode::Return => {
                    let result = self.op_return(instr)?;
                    if self.frames.is_empty() {
                        return Ok(result);
                    }
                }
                OpCode::GetUpval => self.op_getupval(instr)?,
                OpCode::SetUpval => self.op_setupval(instr)?,
                OpCode::Closure => self.op_closure(instr)?,
                OpCode::Concat => self.op_concat(instr)?,
                OpCode::GetGlobal => self.op_getglobal(instr)?,
                OpCode::SetGlobal => self.op_setglobal(instr)?,
            }
        }
    }

    // Opcode implementations
    #[inline(always)]
    fn op_move(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let frame = self.current_frame_mut();
        // SAFETY: Compiler guarantees a,b are within max_stack_size
        unsafe {
            *frame.registers.get_unchecked_mut(a) = frame.registers.get_unchecked(b).clone();
        }
        Ok(())
    }

    #[inline(always)]
    fn op_loadk(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;
        let frame = self.current_frame_mut();
        let constant = frame.function.chunk.constants[bx].clone();
        frame.registers[a] = constant;
        Ok(())
    }

    fn op_loadnil(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let frame = self.current_frame_mut();
        frame.registers[a] = LuaValue::nil();
        Ok(())
    }

    fn op_loadbool(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr);
        let c = Instruction::get_c(instr);
        let frame = self.current_frame_mut();
        frame.registers[a] = LuaValue::boolean(b != 0);
        if c != 0 {
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_newtable(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let table = self.create_table();
        let frame = self.current_frame_mut();
        frame.registers[a] = LuaValue::from_table_rc(table);
        Ok(())
    }

    fn op_gettable(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        // Check types first to determine fast path
        let (is_tbl_int, is_tbl_str) = {
            let frame = self.current_frame();
            let is_table = frame.registers[b].is_table();
            (
                is_table && frame.registers[c].is_integer(),
                is_table && frame.registers[c].is_string(),
            )
        };

        // Fast path: integer key
        if is_tbl_int {
            let frame = self.current_frame();
            if let (Some(tbl), Some(idx)) = (
                &frame.registers[b].as_table_rc(),
                &frame.registers[c].as_integer(),
            ) {
                let value = tbl.borrow().get_int(*idx).unwrap_or(LuaValue::nil());
                let frame_idx = self.frames.len() - 1;
                self.frames[frame_idx].registers[a] = value;
                return Ok(());
            }
        }

        // Fast path: string key
        if is_tbl_str {
            let frame = self.current_frame();
            if let (Some(tbl), Some(key_str)) = (
                &frame.registers[b].as_table_rc(),
                &frame.registers[c].as_string(),
            ) {
                let value = tbl.borrow().get_str(key_str).unwrap_or(LuaValue::nil());
                let frame_idx = self.frames.len() - 1;
                self.frames[frame_idx].registers[a] = value;
                return Ok(());
            }
        }

        // Slow path: clone for complex cases
        let (table, key) = {
            let frame = self.current_frame();
            (frame.registers[b].clone(), frame.registers[c].clone())
        };

        if let Some(tbl) = table.as_table() {
            // Use VM's table_get which handles metamethods
            let value = self.table_get(tbl, &key).unwrap_or(LuaValue::nil());
            let frame = self.current_frame_mut();
            frame.registers[a] = value;
            Ok(())
        } else if let Some(ud) = table.as_userdata() {
            // Handle userdata __index metamethod
            let value = self.userdata_get(ud, &key).unwrap_or(LuaValue::nil());
            let frame = self.current_frame_mut();
            frame.registers[a] = value;
            Ok(())
        } else {
            Err("Attempt to index a non-table value".to_string())
        }
    }

    fn op_settable(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        // Check types and clone necessary values in one go
        let (tbl_clone, idx_opt, key_opt, value) = {
            let frame = self.current_frame();
            match (frame.registers[a].kind(), frame.registers[b].kind()) {
                (LuaValueKind::Table, LuaValueKind::Integer) => {
                    let tbl = frame.registers[a].as_table_rc().unwrap();
                    let idx = frame.registers[b].as_integer().unwrap();
                    (Some(tbl), Some(idx), None, frame.registers[c].clone())
                }
                (LuaValueKind::Table, LuaValueKind::String) => {
                    let tbl = frame.registers[a].as_table_rc().unwrap();
                    let key_str = frame.registers[b].as_string_rc().unwrap();
                    (Some(tbl), None, Some(key_str), frame.registers[c].clone())
                }
                _ => (None, None, None, LuaValue::nil()),
            }
        };

        // Fast path: integer key
        if let (Some(tbl), Some(idx)) = (tbl_clone.as_ref(), idx_opt) {
            tbl.borrow_mut().set_int(idx, value);
            return Ok(());
        }

        // Fast path: string key
        if let (Some(tbl), Some(key)) = (tbl_clone.as_ref(), key_opt.as_ref()) {
            tbl.borrow_mut().set_str(Rc::clone(key), value);
            return Ok(());
        }

        // Slow path
        let (table, key, value) = {
            let frame = self.current_frame();
            (
                frame.registers[a].clone(),
                frame.registers[b].clone(),
                frame.registers[c].clone(),
            )
        };

        if let Some(tbl) = table.as_table() {
            self.table_set(tbl, key, value)?;
            Ok(())
        } else {
            Err("Attempt to index a non-table value".to_string())
        }
    }

    /// Optimized: R(A) := R(B)[C] where C is a literal integer
    #[inline]
    fn op_gettable_i(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as i64;

        let table = {
            let frame = self.current_frame();
            frame.registers[b].clone()
        };

        if let Some(tbl) = table.as_table() {
            // Fast path: direct integer access
            let value = tbl.borrow().get_int(c).unwrap_or(LuaValue::nil());
            let frame = self.current_frame_mut();
            frame.registers[a] = value;
            Ok(())
        } else {
            Err("Attempt to index a non-table value".to_string())
        }
    }

    /// Optimized: R(A)[B] := R(C) where B is a literal integer
    #[inline]
    fn op_settable_i(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as i64;
        let c = Instruction::get_c(instr) as usize;

        let (table, value) = {
            let frame = self.current_frame();
            (frame.registers[a].clone(), frame.registers[c].clone())
        };

        if let Some(tbl) = table.as_table() {
            tbl.borrow_mut().set_int(b, value);
            Ok(())
        } else {
            Err("Attempt to index a non-table value".to_string())
        }
    }

    /// Optimized: R(A) := R(B)[K(C)] where K(C) is a string constant
    #[inline]
    fn op_gettable_k(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let (table, key) = {
            let frame = self.current_frame();
            (
                frame.registers[b].clone(),
                frame.function.chunk.constants[c].clone(),
            )
        };

        if let Some(tbl) = table.as_table() {
            if let Some(key_str) = key.as_string() {
                // Fast path: direct string key access
                let value = tbl.borrow().get_str(&key_str).unwrap_or(LuaValue::nil());
                let frame = self.current_frame_mut();
                frame.registers[a] = value;
                Ok(())
            } else {
                // Fallback: use generic get with metamethods
                let value = self.table_get(tbl, &key).unwrap_or(LuaValue::nil());
                let frame = self.current_frame_mut();
                frame.registers[a] = value;
                Ok(())
            }
        } else {
            Err("Attempt to index a non-table value".to_string())
        }
    }

    /// Optimized: R(A)[K(B)] := R(C) where K(B) is a string constant
    #[inline]
    fn op_settable_k(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let (table, key, value) = {
            let frame = self.current_frame();
            (
                frame.registers[a].clone(),
                frame.function.chunk.constants[b].clone(),
                frame.registers[c].clone(),
            )
        };

        if let Some(tbl) = table.as_table() {
            if let Some(key_str) = key.as_string() {
                // Fast path: direct string key set
                tbl.borrow_mut().set_str(key_str, value);
                Ok(())
            } else {
                // Fallback: use generic set with metamethods
                self.table_set(tbl, key, value)?;
                Ok(())
            }
        } else {
            Err("Attempt to index a non-table value".to_string())
        }
    }

    #[inline]
    fn op_add(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();

        // SAFETY: Compiler guarantees indices are within bounds
        // Fast path: avoid cloning and bounds check
        unsafe {
            let left = frame.registers.get_unchecked(b);
            let right = frame.registers.get_unchecked(c);

            match (left.kind(), right.kind()) {
                (LuaValueKind::Integer, LuaValueKind::Integer) => {
                    let i = left.as_integer().unwrap();
                    let j = right.as_integer().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::integer(i + j);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Float) => {
                    let l = left.as_float().unwrap();
                    let r = right.as_float().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(l + r);
                    return Ok(());
                }
                (LuaValueKind::Integer, LuaValueKind::Float) => {
                    let i = left.as_integer().unwrap();
                    let f = right.as_float().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(i as f64 + f);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Integer) => {
                    let f = left.as_float().unwrap();
                    let i = right.as_integer().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(f + i as f64);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Slow path: clone for metamethods
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if self.call_binop_metamethod(&left, &right, "__add", a)? {
            Ok(())
        } else {
            Err(format!(
                "attempt to add non-number values ({:?} + {:?})",
                left, right
            ))
        }
    }

    #[inline]
    fn op_sub(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();

        // SAFETY: Compiler guarantees indices are within bounds
        unsafe {
            let left = frame.registers.get_unchecked(b);
            let right = frame.registers.get_unchecked(c);
            
            match (left.kind(), right.kind()) {
                (LuaValueKind::Integer, LuaValueKind::Integer) => {
                    let i = left.as_integer().unwrap();
                    let j = right.as_integer().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::integer(i - j);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Float) => {
                    let l = left.as_float().unwrap();
                    let r = right.as_float().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(l - r);
                    return Ok(());
                }
                (LuaValueKind::Integer, LuaValueKind::Float) => {
                    let i = left.as_integer().unwrap();
                    let f = right.as_float().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(i as f64 - f);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Integer) => {
                    let f = left.as_float().unwrap();
                    let i = right.as_integer().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(f - i as f64);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Slow path
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if self.call_binop_metamethod(&left, &right, "__sub", a)? {
            Ok(())
        } else {
            Err(format!("attempt to subtract non-number values"))
        }
    }

    #[inline]
    fn op_mul(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();

        // SAFETY: Compiler guarantees indices are within bounds
        unsafe {
            let left = frame.registers.get_unchecked(b);
            let right = frame.registers.get_unchecked(c);
            
            match (left.kind(), right.kind()) {
                (LuaValueKind::Integer, LuaValueKind::Integer) => {
                    let i = left.as_integer().unwrap();
                    let j = right.as_integer().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::integer(i * j);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Float) => {
                    let l = left.as_float().unwrap();
                    let r = right.as_float().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(l * r);
                    return Ok(());
                }
                (LuaValueKind::Integer, LuaValueKind::Float) => {
                    let i = left.as_integer().unwrap();
                    let f = right.as_float().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(i as f64 * f);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Integer) => {
                    let f = left.as_float().unwrap();
                    let i = right.as_integer().unwrap();
                    *frame.registers.get_unchecked_mut(a) = LuaValue::float(f * i as f64);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Slow path
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if self.call_binop_metamethod(&left, &right, "__mul", a)? {
            Ok(())
        } else {
            Err(format!("attempt to multiply non-number values"))
        }
    }

    #[inline]
    fn op_div(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();

        // Fast path: division always returns float in Lua
        match (frame.registers[b].kind(), frame.registers[c].kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let i = frame.registers[b].as_integer().unwrap();
                let j = frame.registers[c].as_integer().unwrap();
                frame.registers[a] = LuaValue::float(i as f64 / j as f64);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = frame.registers[b].as_float().unwrap();
                let r = frame.registers[c].as_float().unwrap();
                frame.registers[a] = LuaValue::float(l / r);
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = frame.registers[b].as_integer().unwrap();
                let f = frame.registers[c].as_float().unwrap();
                frame.registers[a] = LuaValue::float(i as f64 / f);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = frame.registers[b].as_float().unwrap();
                let i = frame.registers[c].as_integer().unwrap();
                frame.registers[a] = LuaValue::float(f / i as f64);
                return Ok(());
            }
            _ => {}
        }

        // Slow path
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if self.call_binop_metamethod(&left, &right, "__div", a)? {
            Ok(())
        } else {
            Err(format!("attempt to divide non-number values"))
        }
    }

    #[inline]
    fn op_mod(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();

        // Fast path
        match (frame.registers[b].kind(), frame.registers[c].kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let i = frame.registers[b].as_integer().unwrap();
                let j = frame.registers[c].as_integer().unwrap();
                frame.registers[a] = LuaValue::integer(i % j);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = frame.registers[b].as_float().unwrap();
                let r = frame.registers[c].as_float().unwrap();
                frame.registers[a] = LuaValue::float(l % r);
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = frame.registers[b].as_integer().unwrap();
                let f = frame.registers[c].as_float().unwrap();
                frame.registers[a] = LuaValue::float((i as f64) % f);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = frame.registers[b].as_float().unwrap();
                let i = frame.registers[c].as_integer().unwrap();
                frame.registers[a] = LuaValue::float(f % (i as f64));
                return Ok(());
            }
            _ => {}
        }

        // Slow path
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if self.call_binop_metamethod(&left, &right, "__mod", a)? {
            Ok(())
        } else {
            Err(format!("attempt to perform modulo on non-number values"))
        }
    }

    fn op_pow(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        match (&left, &right) {
            (l, r) if l.is_number() && r.is_number() => {
                let l_num = l.as_number().unwrap();
                let r_num = r.as_number().unwrap();
                let frame = self.current_frame_mut();
                frame.registers[a] = LuaValue::number(l_num.powf(r_num));
                Ok(())
            }
            _ => {
                // Try __pow metamethod
                if self.call_binop_metamethod(&left, &right, "__pow", a)? {
                    Ok(())
                } else {
                    Err(format!("attempt to exponentiate non-number values"))
                }
            }
        }
    }

    #[inline(always)]
    fn op_unm(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let frame = self.current_frame_mut();

        // Fast path: avoid clone
        match frame.registers[b].kind() {
            LuaValueKind::Integer => {
                let i = frame.registers[b].as_integer().unwrap();
                frame.registers[a] = LuaValue::integer(-i);
                return Ok(());
            }
            LuaValueKind::Float => {
                let f = frame.registers[b].as_float().unwrap();
                frame.registers[a] = LuaValue::float(-f);
                return Ok(());
            }
            _ => {}
        }

        // Slow path: need to clone for metamethod
        let value = frame.registers[b].clone();
        if self.call_unop_metamethod(&value, "__unm", a)? {
            Ok(())
        } else {
            Err(format!("attempt to negate non-number value"))
        }
    }

    #[inline]
    fn op_idiv(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();

        // Fast path
        match (frame.registers[b].kind(), frame.registers[c].kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let i = frame.registers[b].as_integer().unwrap();
                let j = frame.registers[c].as_integer().unwrap();
                if j == 0 {
                    return Err("attempt to divide by zero".to_string());
                }
                frame.registers[a] = LuaValue::integer(i / j);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = frame.registers[b].as_float().unwrap();
                let r = frame.registers[c].as_float().unwrap();
                let result = (l / r).floor();
                frame.registers[a] = LuaValue::integer(result as i64);
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = frame.registers[b].as_integer().unwrap();
                let f = frame.registers[c].as_float().unwrap();
                let result = (i as f64 / f).floor();
                frame.registers[a] = LuaValue::integer(result as i64);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = frame.registers[b].as_float().unwrap();
                let i = frame.registers[c].as_integer().unwrap();
                let result = (f / i as f64).floor();
                frame.registers[a] = LuaValue::integer(result as i64);
                return Ok(());
            }
            _ => {}
        }

        // Slow path
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if self.call_binop_metamethod(&left, &right, "__idiv", a)? {
            Ok(())
        } else {
            Err(format!(
                "attempt to perform integer division on non-number values"
            ))
        }
    }

    #[inline(always)]
    fn op_not(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let frame = self.current_frame_mut();
        let value = frame.registers[b].is_truthy();
        frame.registers[a] = LuaValue::boolean(!value);
        Ok(())
    }

    fn op_len(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let value = {
            let frame = self.current_frame();
            frame.registers[b].clone()
        };

        // Strings have raw length
        if let Some(s) = value.as_string() {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::integer(s.as_str().len() as i64);
            return Ok(());
        }

        // Try __len metamethod for tables
        if value.as_table().is_some() {
            if self.call_unop_metamethod(&value, "__len", a)? {
                return Ok(());
            }

            // No __len metamethod, use raw length
            if let Some(t) = value.as_table() {
                let frame = self.current_frame_mut();
                frame.registers[a] = LuaValue::integer(t.borrow().len() as i64);
                return Ok(());
            }
        }

        Err("attempt to get length of a non-sequence value".to_string())
    }

    #[inline]
    fn op_eq(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        // Fast path: direct comparison for primitives
        let fast_result = {
            let frame = self.current_frame();
            let left = &frame.registers[b];
            let right = &frame.registers[c];
            
            match (left.kind(), right.kind()) {
                (LuaValueKind::Nil, LuaValueKind::Nil) => Some(true),
                (LuaValueKind::Boolean, LuaValueKind::Boolean) => {
                    Some(left.as_bool().unwrap() == right.as_bool().unwrap())
                }
                (LuaValueKind::Integer, LuaValueKind::Integer) => {
                    Some(left.as_integer().unwrap() == right.as_integer().unwrap())
                }
                (LuaValueKind::Float, LuaValueKind::Float) => {
                    Some(left.as_float().unwrap() == right.as_float().unwrap())
                }
                (LuaValueKind::Integer, LuaValueKind::Float) => {
                    let i = left.as_integer().unwrap();
                    let f = right.as_float().unwrap();
                    Some(i as f64 == f)
                }
                (LuaValueKind::Float, LuaValueKind::Integer) => {
                    let f = left.as_float().unwrap();
                    let i = right.as_integer().unwrap();
                    Some(f == i as f64)
                }
                _ => None,
            }
        };

        if let Some(result) = fast_result {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::boolean(result);
            return Ok(());
        }

        // Slow path: need to handle tables/strings/metamethods
        let (left, right) = {
            let frame = self.current_frame();
            (frame.registers[b].clone(), frame.registers[c].clone())
        };

        if self.values_equal(&left, &right) {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::boolean(true);
            return Ok(());
        }

        let left_mm = self.get_metamethod(&left, "__eq");
        let right_mm = self.get_metamethod(&right, "__eq");

        if let (Some(mm_left), Some(mm_right)) = (&left_mm, &right_mm) {
            if self.values_equal(mm_left, mm_right) {
                if self.call_binop_metamethod(&left, &right, "__eq", a)? {
                    return Ok(());
                }
            }
        }

        let frame = self.current_frame_mut();
        frame.registers[a] = LuaValue::boolean(false);
        Ok(())
    }

    #[inline]
    fn op_lt(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();

        // Fast path: numeric comparison without clone
        match (frame.registers[b].kind(), frame.registers[c].kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let l = frame.registers[b].as_integer().unwrap();
                let r = frame.registers[c].as_integer().unwrap();
                frame.registers[a] = LuaValue::boolean(l < r);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = frame.registers[b].as_float().unwrap();
                let r = frame.registers[c].as_float().unwrap();
                frame.registers[a] = LuaValue::boolean(l < r);
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = frame.registers[b].as_integer().unwrap();
                let f = frame.registers[c].as_float().unwrap();
                frame.registers[a] = LuaValue::boolean((i as f64) < f);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = frame.registers[b].as_float().unwrap();
                let i = frame.registers[c].as_integer().unwrap();
                frame.registers[a] = LuaValue::boolean(f < (i as f64));
                return Ok(());
            }
            _ => {}
        }

        // Slow path: strings and metamethods
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if let (Some(l), Some(r)) = (left.as_string(), right.as_string()) {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::boolean(l.as_str() < r.as_str());
            return Ok(());
        }

        if self.call_binop_metamethod(&left, &right, "__lt", a)? {
            return Ok(());
        }

        Err("attempt to compare incompatible values".to_string())
    }

    #[inline]
    fn op_le(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();

        // Fast path: numeric comparison without clone
        match (frame.registers[b].kind(), frame.registers[c].kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let l = frame.registers[b].as_integer().unwrap();
                let r = frame.registers[c].as_integer().unwrap();
                frame.registers[a] = LuaValue::boolean(l <= r);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = frame.registers[b].as_float().unwrap();
                let r = frame.registers[c].as_float().unwrap();
                frame.registers[a] = LuaValue::boolean(l <= r);
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = frame.registers[b].as_integer().unwrap();
                let f = frame.registers[c].as_float().unwrap();
                frame.registers[a] = LuaValue::boolean((i as f64) <= f);
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = frame.registers[b].as_float().unwrap();
                let i = frame.registers[c].as_integer().unwrap();
                frame.registers[a] = LuaValue::boolean(f <= (i as f64));
                return Ok(());
            }
            _ => {}
        }

        // Slow path
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if let (Some(l), Some(r)) = (left.as_string(), right.as_string()) {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::boolean(l.as_str() <= r.as_str());
            return Ok(());
        }

        if self.call_binop_metamethod(&left, &right, "__le", a)? {
            return Ok(());
        }

        if let Some(_) = self.get_metamethod(&left, "__lt") {
            if self.call_binop_metamethod(&right, &left, "__lt", a)? {
                let frame = self.current_frame_mut();
                let result = frame.registers[a].is_truthy();
                frame.registers[a] = LuaValue::boolean(!result);
                return Ok(());
            }
        }

        Err("attempt to compare incompatible values".to_string())
    }

    #[inline(always)]
    fn op_jmp(&mut self, instr: u32) -> Result<(), String> {
        let sbx = Instruction::get_sbx(instr);
        let frame = self.current_frame_mut();
        frame.pc = (frame.pc as i32 + sbx) as usize;
        Ok(())
    }

    // Numeric for loop opcodes for optimal performance
    #[inline]
    fn op_forprep(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let sbx = Instruction::get_sbx(instr);

        let frame = self.current_frame_mut();

        // R(A) should be init, R(A+1) should be limit, R(A+2) should be step
        // Subtract step from init: R(A) -= R(A+2)
        match (frame.registers[a].kind(), frame.registers[a + 2].kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let init = frame.registers[a].as_integer().unwrap();
                let step = frame.registers[a + 2].as_integer().unwrap();
                let new_init = init - step;
                frame.registers[a] = LuaValue::integer(new_init);
                // Pre-initialize R(A+3) so ForLoop can do in-place update
                frame.registers[a + 3] = LuaValue::integer(new_init);
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let init = frame.registers[a].as_float().unwrap();
                let step = frame.registers[a + 2].as_float().unwrap();
                let new_init = init - step;
                frame.registers[a] = LuaValue::float(new_init);
                frame.registers[a + 3] = LuaValue::float(new_init);
            }
            (_, _) if frame.registers[a].is_number() && frame.registers[a + 2].is_number() => {
                let init_f = frame.registers[a].as_number().unwrap();
                let step_f = frame.registers[a + 2].as_number().unwrap();
                let new_init = init_f - step_f;
                frame.registers[a] = LuaValue::float(new_init);
                frame.registers[a + 3] = LuaValue::float(new_init);
            }
            _ => {
                return Err("'for' initial value must be a number".to_string());
            }
        }

        // Jump to loop start
        frame.pc = (frame.pc as i32 + sbx) as usize;
        Ok(())
    }

    #[inline]
    fn op_forloop(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let sbx = Instruction::get_sbx(instr);

        let frame = self.current_frame_mut();

        // R(A) is index, R(A+1) is limit, R(A+2) is step
        // SAFETY: Compiler guarantees a+3 is within bounds
        unsafe {
            // Fast path: Pure integer loop (most common case)
            let idx_val = frame.registers.get_unchecked(a);
            let limit_val = frame.registers.get_unchecked(a + 1);
            let step_val = frame.registers.get_unchecked(a + 2);
            
            if let (LuaValueKind::Integer, LuaValueKind::Integer, LuaValueKind::Integer) = 
                (idx_val.kind(), limit_val.kind(), step_val.kind()) {
                let idx = idx_val.as_integer().unwrap();
                let limit = limit_val.as_integer().unwrap();
                let step = step_val.as_integer().unwrap();
                
                let new_idx = idx + step;
                let continue_loop = if step >= 0 {
                    new_idx <= limit
                } else {
                    new_idx >= limit
                };

                *frame.registers.get_unchecked_mut(a) = LuaValue::integer(new_idx);

                if continue_loop {
                    *frame.registers.get_unchecked_mut(a + 3) = LuaValue::integer(new_idx);
                    // Jump back to loop body
                    frame.pc = (frame.pc as i32 + sbx) as usize;
                }
                return Ok(());
            }

            // Slow path: Float or mixed types
            let (new_value, continue_loop) = match (
                frame.registers.get_unchecked(a).kind(),
                frame.registers.get_unchecked(a + 1).kind(),
                frame.registers.get_unchecked(a + 2).kind(),
            ) {
                (LuaValueKind::Float, LuaValueKind::Float, LuaValueKind::Float) => {
                    let idx = frame.registers.get_unchecked(a).as_float().unwrap();
                    let limit = frame.registers.get_unchecked(a + 1).as_float().unwrap();
                    let step = frame.registers.get_unchecked(a + 2).as_float().unwrap();
                    
                    let new_idx = idx + step;
                    let cont = if step >= 0.0 {
                        new_idx <= limit
                    } else {
                        new_idx >= limit
                    };
                    (LuaValue::float(new_idx), cont)
                }
                _ => {
                    // Mixed or other numeric types
                    let idx_val = frame.registers.get_unchecked(a);
                    let limit_val = frame.registers.get_unchecked(a + 1);
                    let step_val = frame.registers.get_unchecked(a + 2);
                    
                    if idx_val.is_number() && limit_val.is_number() && step_val.is_number() {
                        let idx_f = idx_val.as_number().unwrap();
                        let limit_f = limit_val.as_number().unwrap();
                        let step_f = step_val.as_number().unwrap();
                        let new_idx = idx_f + step_f;
                        let cont = if step_f >= 0.0 {
                            new_idx <= limit_f
                        } else {
                            new_idx >= limit_f
                        };
                        (LuaValue::float(new_idx), cont)
                    } else {
                        return Err("'for' step/limit must be a number".to_string());
                    }
                }
            };

            *frame.registers.get_unchecked_mut(a) = new_value.clone();

            if continue_loop {
                // Copy index to loop variable: R(A+3) = R(A)
                *frame.registers.get_unchecked_mut(a + 3) = new_value;
                // Jump back to loop body
                frame.pc = (frame.pc as i32 + sbx) as usize;
            }
        }

        Ok(())
    }

    fn op_test(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let c = Instruction::get_c(instr);
        let frame = self.current_frame_mut();

        let is_true = frame.registers[a].is_truthy();
        if (is_true as u32) != c {
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_testset(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr);
        let frame = self.current_frame_mut();

        let is_true = frame.registers[b].is_truthy();
        if (is_true as u32) == c {
            frame.registers[a] = frame.registers[b].clone();
        } else {
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_call(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let func = frame.registers[a].clone();

        // Check for __call metamethod on non-functions (tables, userdata)
        if !func.is_function() && !func.is_cfunction() {
            // Look for __call metamethod
            if let Some(metatable) = func.get_metatable() {
                let call_key = self.create_string("__call".to_string());
                if let Some(call_func) = metatable
                    .borrow()
                    .raw_get(&LuaValue::from_string_rc(call_key))
                {
                    // Replace func with the __call function
                    // But we need to pass the original value as the first argument
                    // This means shifting all arguments: (func, arg1, arg2) -> (call_func, func, arg1, arg2)

                    let current_frame = self.current_frame_mut();

                    // Shift arguments right by one position
                    // We need to be careful about register allocation
                    let original_func = func.clone();
                    let call_function = call_func.clone();

                    // Create new register layout: [call_func, original_func, arg1, arg2, ...]
                    current_frame.registers[a] = call_function;

                    // Shift existing arguments
                    for i in (1..b).rev() {
                        if a + i + 1 < current_frame.registers.len() {
                            current_frame.registers[a + i + 1] =
                                current_frame.registers[a + i].clone();
                        }
                    }

                    // Place original func as first argument
                    if a + 1 < current_frame.registers.len() {
                        current_frame.registers[a + 1] = original_func;
                    }

                    // Adjust b to include the extra argument
                    let new_b = b + 1;

                    // Recreate instruction with new b
                    let new_instr =
                        Instruction::encode_abc(OpCode::Call, a as u32, new_b as u32, c as u32);
                    return self.op_call(new_instr);
                }
            }

            return Err("Attempt to call a non-function value".to_string());
        }

        // Check for CFunction (native Rust function)
        if let Some(cfunc) = func.as_cfunction() {
            // Create a temporary call frame with arguments in registers
            let mut arg_registers = vec![func.clone()]; // Register 0 is the function itself

            // Copy arguments to registers
            for i in 1..b {
                if a + i < frame.registers.len() {
                    arg_registers.push(frame.registers[a + i].clone());
                } else {
                    arg_registers.push(LuaValue::nil());
                }
            }

            // Create temporary frame for CFunction
            let frame_id = self.next_frame_id;
            self.next_frame_id += 1;

            let temp_frame = CallFrame {
                frame_id,
                function: self.current_frame().function.clone(),
                pc: self.current_frame().pc,
                registers: arg_registers,
                base: self.frames.len(),
                result_reg: 0,
                num_results: 0,
            };

            self.frames.push(temp_frame);

            // Call the CFunction
            let multi_result = cfunc(self)?;

            // Pop the temporary frame
            self.frames.pop();

            // Store return values
            // C = 0: use all return values
            // C = 1: no return values expected
            // C = 2: 1 return value expected (at register a)
            // C = 3: 2 return values expected (at registers a, a+1)
            // etc.

            // Get all return values from MultiValue
            let all_returns = multi_result.all_values();
            let num_returns = all_returns.len();

            let num_expected = if c == 0 { num_returns } else { c - 1 };

            // Store them in registers
            let current_frame = self.current_frame_mut();
            for (i, value) in all_returns.into_iter().take(num_expected).enumerate() {
                if a + i < current_frame.registers.len() {
                    current_frame.registers[a + i] = value;
                }
            }

            // Fill remaining expected registers with nil
            for i in num_returns..num_expected {
                if a + i < current_frame.registers.len() {
                    current_frame.registers[a + i] = LuaValue::nil();
                }
            }

            return Ok(());
        }

        // Regular Lua function call
        if let Some(lua_func) = func.as_function() {
            let mut new_registers = vec![LuaValue::nil(); lua_func.chunk.max_stack_size];

            // Copy arguments
            for i in 1..b {
                if a + i < frame.registers.len() {
                    new_registers[i - 1] = frame.registers[a + i].clone();
                }
            }

            let frame_id = self.next_frame_id;
            self.next_frame_id += 1;

            let new_frame = CallFrame {
                frame_id,
                function: lua_func.clone(),
                pc: 0,
                registers: new_registers,
                base: self.frames.len(),
                result_reg: a,
                num_results: if c == 0 { usize::MAX } else { c - 1 },
            };

            self.frames.push(new_frame);
            Ok(())
        } else {
            Err("Attempt to call a non-function value".to_string())
        }
    }

    fn op_return(&mut self, instr: u32) -> Result<LuaValue, String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        // Collect return values
        // B = 0: return all values from A to top
        // B = 1: return 0 values
        // B = 2: return 1 value (registers[a])
        // B = 3: return 2 values (registers[a], registers[a+1])
        // etc.
        let num_returns = if b == 0 {
            // Return all values from A to top of stack
            let frame = self.current_frame();
            frame.registers.len().saturating_sub(a)
        } else {
            b - 1
        };

        let mut values = Vec::new();
        if num_returns > 0 {
            let frame = self.current_frame();
            for i in 0..num_returns {
                if a + i < frame.registers.len() {
                    values.push(frame.registers[a + i].clone());
                } else {
                    values.push(LuaValue::nil());
                }
            }
        }

        // Save caller info before popping
        let caller_result_reg = self.current_frame().result_reg;
        let caller_num_results = self.current_frame().num_results;
        let exiting_frame_id = self.current_frame().frame_id;

        // Close upvalues for the exiting frame
        self.close_upvalues(exiting_frame_id);

        self.frames.pop();

        // Store return values
        self.return_values = values.clone();

        // If there's a caller frame, copy return values to its registers
        if !self.frames.is_empty() {
            let frame = self.current_frame_mut();
            let num_to_copy = caller_num_results.min(values.len());

            for (i, val) in values.iter().take(num_to_copy).enumerate() {
                if caller_result_reg + i < frame.registers.len() {
                    frame.registers[caller_result_reg + i] = val.clone();
                }
            }

            // Fill remaining expected results with nil
            if caller_num_results != usize::MAX {
                for i in num_to_copy..caller_num_results {
                    if caller_result_reg + i < frame.registers.len() {
                        frame.registers[caller_result_reg + i] = LuaValue::nil();
                    }
                }
            }
        }

        // For backward compatibility, return first value or nil
        Ok(values.get(0).cloned().unwrap_or(LuaValue::nil()))
    }

    fn op_getupval(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let upvalue = {
            let frame = self.current_frame();
            if b >= frame.function.upvalues.len() {
                return Err(format!("Invalid upvalue index: {}", b));
            }
            frame.function.upvalues[b].clone()
        };

        // Get value from upvalue
        let value = upvalue.get_value(&self.frames);
        self.current_frame_mut().registers[a] = value;

        Ok(())
    }

    fn op_setupval(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let value = self.current_frame().registers[a].clone();

        let upvalue = {
            let frame = self.current_frame();
            if b >= frame.function.upvalues.len() {
                return Err(format!("Invalid upvalue index: {}", b));
            }
            frame.function.upvalues[b].clone()
        };

        // Set value to upvalue
        upvalue.set_value(&mut self.frames, value);

        Ok(())
    }

    fn op_closure(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;

        let (proto, parent_frame_id) = {
            let frame = self.current_frame();
            let parent_chunk = &frame.function.chunk;

            // Get the child chunk (prototype)
            if bx >= parent_chunk.child_protos.len() {
                return Err(format!("Invalid prototype index: {}", bx));
            }

            (parent_chunk.child_protos[bx].clone(), frame.frame_id)
        };

        // Capture upvalues according to the prototype's upvalue descriptors
        let mut upvalues = Vec::new();
        for desc in &proto.upvalue_descs {
            if desc.is_local {
                // Capture from parent's register - create or reuse open upvalue
                let register = desc.index as usize;

                // Check if an open upvalue already exists for this location
                let existing_upvalue = self
                    .open_upvalues
                    .iter()
                    .find(|uv| uv.points_to(parent_frame_id, register))
                    .cloned();

                let upvalue = if let Some(uv) = existing_upvalue {
                    // Reuse existing open upvalue
                    uv
                } else {
                    // Create new open upvalue
                    let uv = LuaUpvalue::new_open(parent_frame_id, register);
                    self.open_upvalues.push(uv.clone());
                    uv
                };

                upvalues.push(upvalue);
            } else {
                // Capture from parent's upvalue (share the same upvalue)
                let frame = self.current_frame();
                if (desc.index as usize) < frame.function.upvalues.len() {
                    upvalues.push(frame.function.upvalues[desc.index as usize].clone());
                } else {
                    // Fallback: create closed upvalue with nil
                    upvalues.push(LuaUpvalue::new_closed(LuaValue::nil()));
                }
            }
        }

        // Create new function (closure)
        let func = self.create_function(proto, upvalues);

        let frame = self.current_frame_mut();
        frame.registers[a] = LuaValue::from_function_rc(func);

        Ok(())
    }

    fn op_concat(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        // Binary concat with metamethod support
        let (left, right) = {
            let frame = self.current_frame();
            (frame.registers[b].clone(), frame.registers[c].clone())
        };

        // Try direct concatenation or __tostring
        let left_str = if let Some(s) = left.as_string() {
            Some(s.as_str().to_string())
        } else if let Some(n) = left.as_number() {
            Some(n.to_string())
        } else if let Some(s) = self.call_tostring_metamethod(&left)? {
            Some(s.as_str().to_string())
        } else {
            None
        };

        let right_str = if let Some(s) = right.as_string() {
            Some(s.as_str().to_string())
        } else if let Some(n) = right.as_number() {
            Some(n.to_string())
        } else if let Some(s) = self.call_tostring_metamethod(&right)? {
            Some(s.as_str().to_string())
        } else {
            None
        };

        if let (Some(l), Some(r)) = (left_str, right_str) {
            let result = l + &r;
            let string = self.create_string(result);
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::from_string_rc(string);
            return Ok(());
        }

        // Try __concat metamethod
        if self.call_binop_metamethod(&left, &right, "__concat", a)? {
            return Ok(());
        }

        Err("attempt to concatenate incompatible values".to_string())
    }

    fn op_getglobal(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;

        let frame = self.current_frame();
        let name_val = frame.function.chunk.constants[bx].clone();

        if let Some(name_str) = name_val.as_string() {
            let name = name_str.as_str();
            let value = self.get_global(name).unwrap_or(LuaValue::nil());

            let frame = self.current_frame_mut();
            frame.registers[a] = value;
            Ok(())
        } else {
            Err("Invalid global name".to_string())
        }
    }

    fn op_setglobal(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;

        let frame = self.current_frame();
        let name_val = frame.function.chunk.constants[bx].clone();
        let value = frame.registers[a].clone();

        if let Some(name_str) = name_val.as_string() {
            let name = name_str.as_str();
            self.set_global(name, value);
            Ok(())
        } else {
            Err("Invalid global name".to_string())
        }
    }

    // Helper methods
    #[inline(always)]
    fn current_frame(&self) -> &CallFrame {
        unsafe { self.frames.last().unwrap_unchecked() }
    }

    #[inline(always)]
    fn current_frame_mut(&mut self) -> &mut CallFrame {
        unsafe { self.frames.last_mut().unwrap_unchecked() }
    }

    pub fn values_equal(&self, left: &LuaValue, right: &LuaValue) -> bool {
        left == right
    }

    pub fn get_global(&self, name: &str) -> Option<LuaValue> {
        let key = LuaValue::from_string_rc(Rc::new(LuaString::new(name.to_string())));
        self.globals.borrow().raw_get(&key)
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        let key = LuaValue::from_string_rc(Rc::new(LuaString::new(name.to_string())));
        self.globals.borrow_mut().raw_set(key, value);
    }

    /// Get value from table with metatable support
    /// Handles __index metamethod
    pub fn table_get(
        &mut self,
        table_rc: Rc<RefCell<LuaTable>>,
        key: &LuaValue,
    ) -> Option<LuaValue> {
        // First try raw get
        let value = {
            let table = table_rc.borrow();
            table.raw_get(key).unwrap_or(LuaValue::nil())
        };

        if !value.is_nil() {
            return Some(value);
        }

        // If not found, check for __index metamethod
        let metatable = {
            let table = table_rc.borrow();
            table.get_metatable()
        };

        if let Some(mt) = metatable {
            let index_key =
                LuaValue::from_string_rc(Rc::new(LuaString::new("__index".to_string())));

            let index_value = {
                let mt_borrowed = mt.borrow();
                mt_borrowed.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table
                    LuaValueKind::Table => {
                        if let Some(t) = index_val.as_table() {
                            return self.table_get(t, key);
                        }
                    }
                    // __index is a function - call it with (table, key)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let self_value = LuaValue::from_table_rc(table_rc);
                        let args = vec![self_value, key.clone()];

                        match self.call_metamethod(&index_val, &args) {
                            Ok(result) => return result,
                            Err(_) => return None,
                        }
                    }
                    _ => {}
                }
            }
        }

        None
    }

    /// Get value from userdata with metatable support
    /// Handles __index metamethod
    pub fn userdata_get(&mut self, userdata: Rc<LuaUserdata>, key: &LuaValue) -> Option<LuaValue> {
        // Check for __index metamethod
        let metatable = userdata.get_metatable();

        if let Some(mt) = metatable {
            let index_key =
                LuaValue::from_string_rc(Rc::new(LuaString::new("__index".to_string())));

            let index_value = {
                let mt_borrowed = mt.borrow();
                mt_borrowed.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table
                    LuaValueKind::Table => {
                        if let Some(t) = index_val.as_table() {
                            return self.table_get(t, key);
                        }
                    }
                    // __index is a function - call it with (userdata, key)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let self_value = LuaValue::from_userdata_rc(userdata);
                        let args = vec![self_value, key.clone()];

                        match self.call_metamethod(&index_val, &args) {
                            Ok(result) => return result,
                            Err(_) => return None,
                        }
                    }
                    _ => {}
                }
            }
        }

        None
    }

    /// Set value in table with metatable support
    /// Handles __newindex metamethod
    pub fn table_set(
        &mut self,
        table_rc: Rc<RefCell<LuaTable>>,
        key: LuaValue,
        value: LuaValue,
    ) -> Result<(), String> {
        // Check if key already exists
        let has_key = {
            let table = table_rc.borrow();
            table.raw_get(&key).map(|v| !v.is_nil()).unwrap_or(false)
        };

        if has_key {
            // Key exists, use raw set
            table_rc.borrow_mut().raw_set(key, value);
            return Ok(());
        }

        // Key doesn't exist, check for __newindex metamethod
        let metatable = {
            let table = table_rc.borrow();
            table.get_metatable()
        };

        if let Some(mt) = metatable {
            let newindex_key =
                LuaValue::from_string_rc(Rc::new(LuaString::new("__newindex".to_string())));

            let newindex_value = {
                let mt_borrowed = mt.borrow();
                mt_borrowed.raw_get(&newindex_key)
            };

            if let Some(newindex_val) = newindex_value {
                match newindex_val.kind() {
                    // __newindex is a table - set in that table
                    LuaValueKind::Table => {
                        if let Some(t) = newindex_val.as_table() {
                            return self.table_set(t, key, value);
                        }
                    }
                    // __newindex is a function - call it with (table, key, value)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let self_value = LuaValue::from_table_rc(table_rc);
                        let args = vec![self_value, key, value];

                        match self.call_metamethod(&newindex_val, &args) {
                            Ok(_) => return Ok(()),
                            Err(e) => return Err(e),
                        }
                    }
                    _ => {}
                }
            }
        }

        // No metamethod or key doesn't exist, use raw set
        table_rc.borrow_mut().raw_set(key, value);
        Ok(())
    }

    /// Call a Lua value (function or CFunction) with the given arguments
    /// Returns the first return value, or None if the call fails
    pub fn call_metamethod(
        &mut self,
        func: &LuaValue,
        args: &[LuaValue],
    ) -> Result<Option<LuaValue>, String> {
        match func.kind() {
            LuaValueKind::CFunction => {
                let cfunc = func.as_cfunction().unwrap();
                // Create a temporary frame for the call
                let mut registers = vec![func.clone()];
                registers.extend_from_slice(args);
                registers.resize(16, LuaValue::nil());

                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // We need a dummy function for the frame - use an empty one
                let dummy_func = Rc::new(LuaFunction {
                    chunk: Rc::new(Chunk {
                        code: Vec::new(),
                        constants: Vec::new(),
                        locals: Vec::new(),
                        upvalue_count: 0,
                        param_count: 0,
                        max_stack_size: 16,
                        child_protos: Vec::new(),
                        upvalue_descs: Vec::new(),
                    }),
                    upvalues: Vec::new(),
                });

                let temp_frame = CallFrame {
                    frame_id,
                    function: dummy_func,
                    pc: 0,
                    registers,
                    base: self.frames.len(),
                    result_reg: 0,
                    num_results: 0,
                };

                self.frames.push(temp_frame);

                // Call the CFunction
                let result = cfunc(self);

                // Pop the temporary frame
                self.frames.pop();

                match result {
                    Ok(multi_val) => {
                        let values = multi_val.all_values();
                        Ok(values.get(0).cloned())
                    }
                    Err(e) => Err(e),
                }
            }
            LuaValueKind::Function => {
                let lua_func = func.as_function_rc().unwrap();
                // Call Lua function
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // Create a new call frame
                let mut registers = vec![LuaValue::nil(); lua_func.chunk.max_stack_size];

                // Copy arguments to registers (starting from register 0)
                for (i, arg) in args.iter().enumerate() {
                    if i < registers.len() {
                        registers[i] = arg.clone();
                    }
                }

                let new_frame = CallFrame {
                    frame_id,
                    function: lua_func.clone(),
                    pc: 0,
                    registers,
                    base: self.frames.len(),
                    result_reg: 0,
                    num_results: 0, // Don't write back to caller's registers
                };

                let initial_frame_count = self.frames.len();
                self.frames.push(new_frame);

                // Execute instructions in this frame until it returns
                let exec_result = loop {
                    if self.frames.len() <= initial_frame_count {
                        // Frame has been popped (function returned)
                        break Ok(());
                    }

                    let frame_idx = self.frames.len() - 1;
                    let pc = self.frames[frame_idx].pc;
                    let chunk = self.frames[frame_idx].function.chunk.clone();

                    if pc >= chunk.code.len() {
                        // End of code
                        self.frames.pop();
                        break Ok(());
                    }

                    let instr = chunk.code[pc];
                    self.frames[frame_idx].pc += 1;

                    // Decode and execute
                    let opcode = Instruction::get_opcode(instr);

                    // Special handling for Return opcode
                    if let OpCode::Return = opcode {
                        match self.op_return(instr) {
                            Ok(_val) => {
                                // Return values are now in self.return_values
                                break Ok(());
                            }
                            Err(e) => {
                                if self.frames.len() > initial_frame_count {
                                    self.frames.pop();
                                }
                                break Err(e);
                            }
                        }
                    }

                    // Execute the instruction
                    let step_result = match opcode {
                        OpCode::Move => self.op_move(instr),
                        OpCode::LoadK => self.op_loadk(instr),
                        OpCode::LoadBool => self.op_loadbool(instr),
                        OpCode::LoadNil => self.op_loadnil(instr),
                        OpCode::GetGlobal => self.op_getglobal(instr),
                        OpCode::SetGlobal => self.op_setglobal(instr),
                        OpCode::GetTable => self.op_gettable(instr),
                        OpCode::SetTable => self.op_settable(instr),
                        OpCode::NewTable => self.op_newtable(instr),
                        OpCode::Call => self.op_call(instr),
                        OpCode::Add => self.op_add(instr),
                        OpCode::Sub => self.op_sub(instr),
                        OpCode::Mul => self.op_mul(instr),
                        OpCode::Div => self.op_div(instr),
                        OpCode::Mod => self.op_mod(instr),
                        OpCode::Pow => self.op_pow(instr),
                        OpCode::Unm => self.op_unm(instr),
                        OpCode::Not => self.op_not(instr),
                        OpCode::Len => self.op_len(instr),
                        OpCode::Concat => self.op_concat(instr),
                        OpCode::Jmp => self.op_jmp(instr),
                        OpCode::Eq => self.op_eq(instr),
                        OpCode::Lt => self.op_lt(instr),
                        OpCode::Le => self.op_le(instr),
                        OpCode::Gt => self.op_gt(instr),
                        OpCode::Ge => self.op_ge(instr),
                        OpCode::Ne => self.op_ne(instr),
                        OpCode::And => self.op_and(instr),
                        OpCode::Or => self.op_or(instr),
                        OpCode::BAnd => self.op_band(instr),
                        OpCode::BOr => self.op_bor(instr),
                        OpCode::BXor => self.op_bxor(instr),
                        OpCode::Shl => self.op_shl(instr),
                        OpCode::Shr => self.op_shr(instr),
                        OpCode::BNot => self.op_bnot(instr),
                        OpCode::IDiv => self.op_idiv(instr),
                        OpCode::Test => self.op_test(instr),
                        OpCode::TestSet => self.op_testset(instr),
                        OpCode::Closure => self.op_closure(instr),
                        OpCode::GetUpval => self.op_getupval(instr),
                        OpCode::SetUpval => self.op_setupval(instr),
                        _ => Err(format!("Unimplemented opcode: {:?}", opcode)),
                    };

                    if let Err(e) = step_result {
                        // Pop the frame on error
                        if self.frames.len() > initial_frame_count {
                            self.frames.pop();
                        }
                        break Err(e);
                    }
                };

                match exec_result {
                    Ok(_) => {
                        // Get the return value from return_values buffer
                        let result = if !self.return_values.is_empty() {
                            Some(self.return_values[0].clone())
                        } else {
                            None
                        };
                        // Clear return values
                        self.return_values.clear();
                        Ok(result)
                    }
                    Err(e) => Err(e),
                }
            }
            _ => Err("Attempt to call a non-function value".to_string()),
        }
    }

    // Additional comparison operators
    fn op_ne(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let (left, right) = {
            let frame = self.current_frame();
            (frame.registers[b].clone(), frame.registers[c].clone())
        };

        let result = !self.values_equal(&left, &right);
        let frame = self.current_frame_mut();
        // Store boolean result in register A
        frame.registers[a] = LuaValue::boolean(result);
        Ok(())
    }

    fn op_gt(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = frame.registers[b]
            .as_number()
            .ok_or("Comparison on non-number")?;
        let right = frame.registers[c]
            .as_number()
            .ok_or("Comparison on non-number")?;

        // Store boolean result in register A
        frame.registers[a] = LuaValue::boolean(left > right);
        Ok(())
    }

    fn op_ge(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = frame.registers[b]
            .as_number()
            .ok_or("Comparison on non-number")?;
        let right = frame.registers[c]
            .as_number()
            .ok_or("Comparison on non-number")?;

        // Store boolean result in register A
        frame.registers[a] = LuaValue::boolean(left >= right);
        Ok(())
    }

    // Logical operators (short-circuit handled at compile time)
    fn op_and(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();

        // Lua's 'and' returns first false value or last value
        let left = frame.registers[b].clone();
        frame.registers[a] = if !left.is_truthy() {
            left
        } else {
            frame.registers[c].clone()
        };
        Ok(())
    }

    fn op_or(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();

        // Lua's 'or' returns first true value or last value
        let left = frame.registers[b].clone();
        frame.registers[a] = if left.is_truthy() {
            left
        } else {
            frame.registers[c].clone()
        };
        Ok(())
    }

    // Bitwise operators
    fn op_band(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::integer(l & r);
            Ok(())
        } else if self.call_binop_metamethod(&left, &right, "__band", a)? {
            Ok(())
        } else {
            Err("Bitwise operation requires integer".to_string())
        }
    }

    fn op_bor(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::integer(l | r);
            Ok(())
        } else if self.call_binop_metamethod(&left, &right, "__bor", a)? {
            Ok(())
        } else {
            Err("Bitwise operation requires integer".to_string())
        }
    }

    fn op_bxor(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::integer(l ^ r);
            Ok(())
        } else if self.call_binop_metamethod(&left, &right, "__bxor", a)? {
            Ok(())
        } else {
            Err("Bitwise operation requires integer".to_string())
        }
    }

    fn op_shl(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::integer(l << (r as u32));
            Ok(())
        } else if self.call_binop_metamethod(&left, &right, "__shl", a)? {
            Ok(())
        } else {
            Err("Bitwise operation requires integer".to_string())
        }
    }

    fn op_shr(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = frame.registers[b].clone();
        let right = frame.registers[c].clone();

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::integer(l >> (r as u32));
            Ok(())
        } else if self.call_binop_metamethod(&left, &right, "__shr", a)? {
            Ok(())
        } else {
            Err("Bitwise operation requires integer".to_string())
        }
    }

    fn op_bnot(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let frame = self.current_frame_mut();
        let value = frame.registers[b].clone();

        if let Some(i) = value.as_integer() {
            let frame = self.current_frame_mut();
            frame.registers[a] = LuaValue::integer(!i);
            Ok(())
        } else if self.call_unop_metamethod(&value, "__bnot", a)? {
            Ok(())
        } else {
            Err("Bitwise operation requires integer".to_string())
        }
    }

    // Integer division

    /// Close all open upvalues for a specific frame
    /// Called when a frame exits to move values from stack to heap
    fn close_upvalues(&mut self, frame_id: usize) {
        // Find all open upvalues pointing to this frame
        let upvalues_to_close: Vec<Rc<LuaUpvalue>> = self
            .open_upvalues
            .iter()
            .filter(|uv| {
                if let Some(frame) = self.frames.iter().find(|f| f.frame_id == frame_id) {
                    // Check if any open upvalue points to this frame
                    for reg_idx in 0..frame.registers.len() {
                        if uv.points_to(frame_id, reg_idx) {
                            return true;
                        }
                    }
                }
                false
            })
            .cloned()
            .collect();

        // Close each upvalue
        for upvalue in upvalues_to_close.iter() {
            // Get the value from the stack before closing
            let value = upvalue.get_value(&self.frames);
            upvalue.close(value);
        }

        // Remove closed upvalues from the open list
        self.open_upvalues.retain(|uv| uv.is_open());
    }

    /// Create a new table and register it with GC
    pub fn create_table(&mut self) -> Rc<RefCell<LuaTable>> {
        let table = Rc::new(RefCell::new(LuaTable::new()));
        let ptr = Rc::as_ptr(&table) as usize;
        self.gc.register_object(ptr, GcObjectType::Table);

        // Trigger GC if needed
        self.maybe_collect_garbage();

        table
    }

    /// Create a string and register it with GC
    pub fn create_string(&mut self, s: String) -> Rc<LuaString> {
        let string = Rc::new(LuaString::new(s));
        let ptr = Rc::as_ptr(&string) as usize;
        self.gc.register_object(ptr, GcObjectType::String);

        // Trigger GC if needed
        self.maybe_collect_garbage();

        string
    }

    /// Create a string for builtin function returns (lighter weight, no immediate GC check)
    /// Returns are short-lived and will be registered when stored in registers
    pub fn create_builtin_string(&mut self, s: String) -> Rc<LuaString> {
        let string = Rc::new(LuaString::new(s));
        let ptr = Rc::as_ptr(&string) as usize;
        self.gc.register_object(ptr, GcObjectType::String);
        string
    }

    /// Create a function and register it with GC
    pub fn create_function(
        &mut self,
        chunk: Rc<Chunk>,
        upvalues: Vec<Rc<LuaUpvalue>>,
    ) -> Rc<LuaFunction> {
        let func = Rc::new(LuaFunction { chunk, upvalues });
        let ptr = Rc::as_ptr(&func) as usize;
        self.gc.register_object(ptr, GcObjectType::Function);

        // Trigger GC if needed
        self.maybe_collect_garbage();

        func
    }

    /// Check if GC should run and collect garbage if needed
    fn maybe_collect_garbage(&mut self) {
        if self.gc.should_collect() {
            self.collect_garbage();
        }
    }

    /// Register all constants in a chunk with GC
    fn register_chunk_constants(&mut self, chunk: &Chunk) {
        for value in &chunk.constants {
            match value.kind() {
                LuaValueKind::String => {
                    let s = value.as_string_rc().unwrap();
                    let ptr = Rc::as_ptr(&s) as usize;
                    self.gc.register_object(ptr, GcObjectType::String);
                }
                LuaValueKind::Table => {
                    let t = value.as_table_rc().unwrap();
                    let ptr = Rc::as_ptr(&t) as usize;
                    self.gc.register_object(ptr, GcObjectType::Table);
                }
                LuaValueKind::Function => {
                    let f = value.as_function_rc().unwrap();
                    let ptr = Rc::as_ptr(&f) as usize;
                    self.gc.register_object(ptr, GcObjectType::Function);
                    // Recursively register nested function chunks
                    self.register_chunk_constants(&f.chunk);
                }
                _ => {}
            }
        }
    }

    /// Perform garbage collection
    pub fn collect_garbage(&mut self) {
        // Collect all roots
        let mut roots = Vec::new();

        // Add the global table itself as a root
        roots.push(LuaValue::from_table_rc(self.globals.clone()));

        // Add all frame registers as roots
        for frame in &self.frames {
            for value in &frame.registers {
                roots.push(value.clone());
            }
        }

        // Add return values as roots
        for value in &self.return_values {
            roots.push(value.clone());
        }

        // Add open upvalues as roots (only closed ones that have values)
        for upvalue in &self.open_upvalues {
            if let Some(value) = upvalue.get_closed_value() {
                roots.push(value);
            }
        }

        // Run GC
        self.gc.collect(&roots);
    }

    /// Get GC statistics
    pub fn gc_stats(&self) -> String {
        let stats = self.gc.stats();
        format!(
            "GC Stats:\n\
            - Bytes allocated: {}\n\
            - Threshold: {}\n\
            - Total collections: {}\n\
            - Minor collections: {}\n\
            - Major collections: {}\n\
            - Objects collected: {}\n\
            - Young generation size: {}\n\
            - Old generation size: {}\n\
            - Promoted objects: {}",
            stats.bytes_allocated,
            stats.threshold,
            stats.collection_count,
            stats.minor_collections,
            stats.major_collections,
            stats.objects_collected,
            stats.young_gen_size,
            stats.old_gen_size,
            stats.promoted_objects
        )
    }

    /// Try to get a metamethod from a value
    fn get_metamethod(&self, value: &LuaValue, event: &str) -> Option<LuaValue> {
        match value.kind() {
            LuaValueKind::Table => {
                let t = value.as_table_rc().unwrap();
                if let Some(mt) = t.borrow().get_metatable() {
                    let key = LuaValue::from_string_rc(Rc::new(LuaString::new(event.to_string())));
                    mt.borrow().raw_get(&key)
                } else {
                    None
                }
            }
            // TODO: Support metatables for other types (strings, userdata)
            _ => None,
        }
    }

    /// Call a binary metamethod (like __add, __sub, etc.)
    fn call_binop_metamethod(
        &mut self,
        left: &LuaValue,
        right: &LuaValue,
        event: &str,
        result_reg: usize,
    ) -> Result<bool, String> {
        // Try left operand's metamethod first
        let metamethod = self
            .get_metamethod(left, event)
            .or_else(|| self.get_metamethod(right, event));

        if let Some(mm) = metamethod {
            self.call_metamethod_with_args(mm, vec![left.clone(), right.clone()], result_reg)
        } else {
            Ok(false)
        }
    }

    /// Call a unary metamethod (like __unm, __bnot, etc.)
    fn call_unop_metamethod(
        &mut self,
        value: &LuaValue,
        event: &str,
        result_reg: usize,
    ) -> Result<bool, String> {
        if let Some(mm) = self.get_metamethod(value, event) {
            self.call_metamethod_with_args(mm, vec![value.clone()], result_reg)
        } else {
            Ok(false)
        }
    }

    /// Generic method to call a metamethod with given arguments
    fn call_metamethod_with_args(
        &mut self,
        metamethod: LuaValue,
        args: Vec<LuaValue>,
        result_reg: usize,
    ) -> Result<bool, String> {
        match metamethod.kind() {
            LuaValueKind::Function => {
                let f = metamethod.as_function_rc().unwrap();
                // Save current state
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // In Lua calling convention:
                // Register 0: the function being called
                // Register 1+: arguments
                let mut registers = vec![LuaValue::nil(); f.chunk.max_stack_size];
                // Don't put function in register[0], it's handled by the function itself
                // Parameters start at local variable positions
                // For a function with N parameters, they are in registers[0..N]
                for (i, arg) in args.iter().enumerate() {
                    if i < registers.len() {
                        registers[i] = arg.clone();
                    }
                }

                let temp_frame = CallFrame {
                    frame_id,
                    function: f.clone(),
                    pc: 0,
                    registers,
                    base: self.frames.len(),
                    result_reg,
                    num_results: 1,
                };

                self.frames.push(temp_frame);

                // Execute the metamethod
                let result = self.run()?;

                // Store result in the target register
                if !self.frames.is_empty() {
                    let frame = self.current_frame_mut();
                    frame.registers[result_reg] = result;
                }

                Ok(true)
            }
            LuaValueKind::CFunction => {
                let cf = metamethod.as_cfunction().unwrap();
                // Create temporary frame for CFunction
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let mut registers = vec![LuaValue::nil(); 10];
                registers[0] = LuaValue::cfunction(cf);
                for (i, arg) in args.iter().enumerate() {
                    registers[i + 1] = arg.clone();
                }

                let temp_frame = CallFrame {
                    frame_id,
                    function: self.current_frame().function.clone(),
                    pc: self.current_frame().pc,
                    registers,
                    base: self.frames.len(),
                    result_reg: 0,
                    num_results: 1,
                };

                self.frames.push(temp_frame);

                // Call the CFunction
                let multi_result = cf(self)?;

                // Pop temporary frame
                self.frames.pop();

                // Store result
                let values = multi_result.all_values();
                let result = values.first().cloned().unwrap_or(LuaValue::nil());
                let frame = self.current_frame_mut();
                frame.registers[result_reg] = result;

                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Call __tostring metamethod if it exists, return the string result
    pub fn call_tostring_metamethod(
        &mut self,
        value: &LuaValue,
    ) -> Result<Option<Rc<LuaString>>, String> {
        // Check for __tostring metamethod
        if let Some(metatable) = value.get_metatable() {
            let tostring_key = self.create_string("__tostring".to_string());
            if let Some(tostring_func) = metatable
                .borrow()
                .raw_get(&LuaValue::from_string_rc(tostring_key))
            {
                // Call the metamethod with the value as argument
                let result = self.call_metamethod(&tostring_func, &[value.clone()])?;

                // Extract string from result
                if let Some(result_val) = result {
                    if let Some(s) = result_val.as_string() {
                        return Ok(Some(s));
                    } else {
                        return Err("'__tostring' must return a string".to_string());
                    }
                }
            }
        }

        Ok(None)
    }

    /// Convert a value to string, calling __tostring metamethod if present
    pub fn value_to_string(&mut self, value: &LuaValue) -> Result<String, String> {
        if let Some(s) = self.call_tostring_metamethod(value)? {
            Ok(s.as_str().to_string())
        } else {
            Ok(value.to_string_repr())
        }
    }

    /// Generate a stack traceback string
    pub fn generate_traceback(&self, error_msg: &str) -> String {
        let mut trace = format!("Runtime error: {}\nStack traceback:", error_msg);

        // Iterate through call frames from top to bottom
        for (i, frame) in self.frames.iter().rev().enumerate() {
            let func_name = "?"; // Could extract from debug info if available
            let pc = frame.pc.saturating_sub(1); // Adjust to show failing instruction

            trace.push_str(&format!(
                "\n  [{}] function '{}' at PC {}",
                self.frames.len() - i,
                func_name,
                pc
            ));
        }

        trace
    }

    /// Execute a function with protected call (pcall semantics)
    pub fn protected_call(&mut self, func: LuaValue, args: Vec<LuaValue>) -> (bool, Vec<LuaValue>) {
        // Save current state
        let initial_frame_count = self.frames.len();

        // Try to call the function
        let result = self.call_function_internal(func, args);

        match result {
            Ok(return_values) => {
                // Success: return true and the return values
                (true, return_values)
            }
            Err(error_msg) => {
                // Error: clean up frames and return false with error message
                // Restore frame count (pop any frames created during the failed call)
                while self.frames.len() > initial_frame_count {
                    self.frames.pop();
                }

                // Return error without traceback for now (can add later)
                let error_str = self.create_string(error_msg);

                (false, vec![LuaValue::from_string_rc(error_str)])
            }
        }
    }

    /// Internal helper to call a function
    fn call_function_internal(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
    ) -> Result<Vec<LuaValue>, String> {
        match func.kind() {
            LuaValueKind::CFunction => {
                let cfunc = func.as_cfunction().unwrap();
                // For CFunction, create a temporary frame
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let mut registers = vec![func.clone()];
                registers.extend(args);
                registers.resize(16, LuaValue::nil());

                let dummy_func = Rc::new(LuaFunction {
                    chunk: Rc::new(Chunk {
                        code: Vec::new(),
                        constants: Vec::new(),
                        locals: Vec::new(),
                        upvalue_count: 0,
                        param_count: 0,
                        max_stack_size: 16,
                        child_protos: Vec::new(),
                        upvalue_descs: Vec::new(),
                    }),
                    upvalues: Vec::new(),
                });

                let temp_frame = CallFrame {
                    frame_id,
                    function: dummy_func,
                    pc: 0,
                    registers,
                    base: self.frames.len(),
                    result_reg: 0,
                    num_results: 0,
                };

                self.frames.push(temp_frame);
                let result = cfunc(self)?;
                self.frames.pop();

                Ok(result.all_values())
            }
            LuaValueKind::Function => {
                let lua_func = func.as_function_rc().unwrap();
                // For Lua function, use similar logic to call_metamethod
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let mut registers = vec![LuaValue::nil(); lua_func.chunk.max_stack_size];
                for (i, arg) in args.iter().enumerate() {
                    if i < registers.len() {
                        registers[i] = arg.clone();
                    }
                }

                let new_frame = CallFrame {
                    frame_id,
                    function: lua_func.clone(),
                    pc: 0,
                    registers,
                    base: self.frames.len(),
                    result_reg: 0,
                    num_results: 0,
                };

                let initial_frame_count = self.frames.len();
                self.frames.push(new_frame);

                // Execute instructions until frame returns
                let exec_result = loop {
                    if self.frames.len() <= initial_frame_count {
                        // Frame has been popped (function returned)
                        break Ok(());
                    }

                    let frame_idx = self.frames.len() - 1;
                    let pc = self.frames[frame_idx].pc;
                    let chunk = self.frames[frame_idx].function.chunk.clone();

                    if pc >= chunk.code.len() {
                        // End of code
                        self.frames.pop();
                        break Ok(());
                    }

                    let instr = chunk.code[pc];
                    self.frames[frame_idx].pc += 1;

                    // Decode and execute
                    let opcode = Instruction::get_opcode(instr);

                    // Special handling for Return opcode
                    if let OpCode::Return = opcode {
                        match self.op_return(instr) {
                            Ok(_val) => {
                                // Return values are now in self.return_values
                                break Ok(());
                            }
                            Err(e) => {
                                if self.frames.len() > initial_frame_count {
                                    self.frames.pop();
                                }
                                break Err(e);
                            }
                        }
                    }

                    // Execute the instruction
                    let step_result = match opcode {
                        OpCode::Move => self.op_move(instr),
                        OpCode::LoadK => self.op_loadk(instr),
                        OpCode::LoadBool => self.op_loadbool(instr),
                        OpCode::LoadNil => self.op_loadnil(instr),
                        OpCode::GetGlobal => self.op_getglobal(instr),
                        OpCode::SetGlobal => self.op_setglobal(instr),
                        OpCode::GetTable => self.op_gettable(instr),
                        OpCode::SetTable => self.op_settable(instr),
                        OpCode::NewTable => self.op_newtable(instr),
                        OpCode::Call => self.op_call(instr),
                        OpCode::Add => self.op_add(instr),
                        OpCode::Sub => self.op_sub(instr),
                        OpCode::Mul => self.op_mul(instr),
                        OpCode::Div => self.op_div(instr),
                        OpCode::Mod => self.op_mod(instr),
                        OpCode::Pow => self.op_pow(instr),
                        OpCode::Unm => self.op_unm(instr),
                        OpCode::Not => self.op_not(instr),
                        OpCode::Len => self.op_len(instr),
                        OpCode::Concat => self.op_concat(instr),
                        OpCode::Jmp => self.op_jmp(instr),
                        OpCode::Eq => self.op_eq(instr),
                        OpCode::Lt => self.op_lt(instr),
                        OpCode::Le => self.op_le(instr),
                        OpCode::Ne => self.op_ne(instr),
                        OpCode::Gt => self.op_gt(instr),
                        OpCode::Ge => self.op_ge(instr),
                        OpCode::And => self.op_and(instr),
                        OpCode::Or => self.op_or(instr),
                        OpCode::BAnd => self.op_band(instr),
                        OpCode::BOr => self.op_bor(instr),
                        OpCode::BXor => self.op_bxor(instr),
                        OpCode::Shl => self.op_shl(instr),
                        OpCode::Shr => self.op_shr(instr),
                        OpCode::BNot => self.op_bnot(instr),
                        OpCode::IDiv => self.op_idiv(instr),
                        OpCode::Test => self.op_test(instr),
                        OpCode::TestSet => self.op_testset(instr),
                        OpCode::Closure => self.op_closure(instr),
                        OpCode::GetUpval => self.op_getupval(instr),
                        OpCode::SetUpval => self.op_setupval(instr),
                        _ => Err(format!("Unimplemented opcode: {:?}", opcode)),
                    };

                    if let Err(e) = step_result {
                        // Pop the frame on error
                        if self.frames.len() > initial_frame_count {
                            self.frames.pop();
                        }
                        break Err(e);
                    }
                };

                match exec_result {
                    Ok(_) => {
                        // Get return values
                        let result = self.return_values.clone();
                        self.return_values.clear();
                        Ok(result)
                    }
                    Err(e) => Err(e),
                }
            }
            _ => Err("attempt to call a non-function value".to_string()),
        }
    }
}
