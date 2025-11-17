// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture
mod lua_call_frame;

use crate::gc::{GC, GcObjectType};
use crate::lib_registry;
use crate::lua_value::{
    Chunk, LuaFunction, LuaString, LuaTable, LuaUpvalue, LuaValue, LuaValueKind,
};
pub use crate::lua_vm::lua_call_frame::LuaCallFrame;
use crate::opcode::{Instruction, OpCode};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Coroutine status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoroutineStatus {
    Suspended, // Created or yielded
    Running,   // Currently executing
    Normal,    // Resumed another coroutine
    Dead,      // Finished or error
}

/// Lua Thread (coroutine)
pub struct LuaThread {
    pub status: CoroutineStatus,
    pub frames: Vec<LuaCallFrame>,
    pub register_stack: Vec<LuaValue>,
    pub return_values: Vec<LuaValue>,
    pub open_upvalues: Vec<Rc<LuaUpvalue>>,
    pub next_frame_id: usize,
    pub error_handler: Option<LuaValue>,
    pub yield_values: Vec<LuaValue>,  // Values yielded from coroutine
    pub resume_values: Vec<LuaValue>, // Values passed to resume() that yield should return
    // For yield: store CALL instruction info to properly restore return values on resume
    pub yield_call_reg: Option<usize>, // Register where return values should be stored (A param of CALL)
    pub yield_call_nret: Option<usize>, // Number of expected return values (C-1 param of CALL)
}

pub struct LuaVM {
    // Global environment table (_G and _ENV point to this)
    globals: LuaValue,

    // Call stack
    pub frames: Vec<LuaCallFrame>,

    // Global register stack (unified stack architecture, like Lua 5.4)
    pub register_stack: Vec<LuaValue>,

    // Garbage collector
    gc: GC,

    // Multi-return value buffer (temporary storage for function returns)
    pub return_values: Vec<LuaValue>,

    // Open upvalues list (for closing when frames exit)
    open_upvalues: Vec<Rc<LuaUpvalue>>,

    // Next frame ID (for tracking frames)
    next_frame_id: usize,

    // Error handling state
    pub error_handler: Option<LuaValue>, // Current error handler for xpcall

    // FFI state
    ffi_state: crate::ffi::FFIState,

    // Current running thread (for coroutine.running())
    pub current_thread: Option<Rc<RefCell<LuaThread>>>,

    // Current thread as LuaValue (for comparison in coroutine.running())
    pub current_thread_value: Option<LuaValue>,

    // Main thread representation (for coroutine.running() in main thread)
    pub main_thread_value: Option<LuaValue>,

    // Yield flag: set when a coroutine yields
    yielding: bool,

    // String metatable (shared by all strings)
    string_metatable: Option<Rc<RefCell<LuaTable>>>,

    // Object pool for unified object management (new architecture)
    pub(crate) object_pool: crate::object_pool::ObjectPool,

    // Legacy string interning table (for compatibility during migration)
    // TODO: Remove after full migration to object pool
    string_table: HashMap<u64, *const LuaString>,

    // Maximum length for interned strings (like Lua's LUAI_MAXSHORTLEN)
    max_short_string_len: usize,
}

impl LuaVM {
    pub fn new() -> Self {
        let mut vm = LuaVM {
            globals: LuaValue::nil(),
            frames: Vec::new(),
            register_stack: Vec::with_capacity(1024), // Pre-allocate for initial stack
            gc: GC::new(),
            return_values: Vec::new(),
            open_upvalues: Vec::new(),
            next_frame_id: 0,
            error_handler: None,
            ffi_state: crate::ffi::FFIState::new(),
            current_thread: None,
            current_thread_value: None,
            main_thread_value: None, // Will be initialized lazily
            yielding: false,
            string_metatable: None,
            object_pool: crate::object_pool::ObjectPool::new(),
            string_table: HashMap::with_capacity(2048),
            max_short_string_len: 64, // Like Lua's LUAI_MAXSHORTLEN
        };

        // Set _G to point to the global table itself
        let globals_ref = vm.create_table();
        vm.globals = globals_ref;
        vm.set_global("_G", globals_ref);
        vm.set_global("_ENV", globals_ref);

        vm
    }

    // Register access helpers for unified stack architecture
    #[inline(always)]
    fn get_register(&self, base_ptr: usize, reg: usize) -> LuaValue {
        self.register_stack[base_ptr + reg]
    }

    #[inline(always)]
    fn set_register(&mut self, base_ptr: usize, reg: usize, value: LuaValue) {
        self.register_stack[base_ptr + reg] = value;
    }

    #[inline(always)]
    fn ensure_stack_capacity(&mut self, required: usize) {
        if self.register_stack.len() < required {
            self.register_stack.resize(required, LuaValue::nil());
        }
    }

    pub fn open_libs(&mut self) {
        let _ = lib_registry::create_standard_registry().load_all(self);

        // Override coroutine.wrap with a Lua implementation
        // because it needs to return a closure that captures the thread
        let wrap_impl = r#"
            local _create = coroutine.create
            local _resume = coroutine.resume
            coroutine.wrap = function(f)
                local co = _create(f)
                return function(...)
                    local success, result = _resume(co, ...)
                    if not success then
                        error(result, 2)
                    end
                    return result
                end
            end
        "#;
        if let Err(e) = self.execute_string(wrap_impl) {
            eprintln!("Warning: Failed to override coroutine.wrap: {}", e);
        }
    }

    pub fn execute(&mut self, chunk: Rc<Chunk>) -> Result<LuaValue, String> {
        // Register all constants in the chunk with GC
        self.register_chunk_constants(&chunk);

        // Create main function in object pool
        let main_func_value = self.create_function(chunk.clone(), Vec::new());

        // Create initial call frame using unified stack
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let base_ptr = self.register_stack.len();
        let required_size = base_ptr + chunk.max_stack_size;
        self.ensure_stack_capacity(required_size);

        let frame = LuaCallFrame::new_lua_function(
            frame_id,
            main_func_value,
            base_ptr,
            chunk.max_stack_size,
            0,
            0,
        );

        self.frames.push(frame);

        // Execute
        let result = self.run()?;

        // Clean up - clear stack used by this execution
        self.register_stack.clear();
        self.open_upvalues.clear();

        Ok(result)
    }

    pub fn execute_string(&mut self, source: &str) -> Result<LuaValue, String> {
        let chunk = self.compile(source)?;
        self.execute(Rc::new(chunk))
    }

    /// Compile source code using VM's string pool
    pub fn compile(&mut self, source: &str) -> Result<Chunk, String> {
        use crate::compiler::Compiler;

        let chunk = Compiler::compile(self, source)?;

        Ok(chunk)
    }

    fn run(&mut self) -> Result<LuaValue, String> {
        let mut instruction_count = 0;
        loop {
            // Check if coroutine is yielding
            if self.yielding {
                return Ok(LuaValue::nil());
            }

            if self.frames.is_empty() {
                return Ok(LuaValue::nil());
            }

            // Resolve function and get instruction (optimized: use cached function_id)
            let frame = self.current_frame();
            let pc = frame.pc;
            let cached_func_id = frame.cached_function_id;

            // Get instruction directly without cloning chunk, using cached function ID
            let instr = if let Some(func_id) = cached_func_id {
                let func_ref = self
                    .object_pool
                    .get_function(func_id)
                    .ok_or("Invalid function ID")?;
                let func_borrowed = func_ref.borrow();
                
                // Check bounds and get instruction
                if pc >= func_borrowed.chunk.code.len() {
                    drop(func_borrowed);
                    self.frames.pop();
                    continue;
                }
                
                func_borrowed.chunk.code[pc]
            } else {
                return Err("Frame function is not a Lua function".to_string());
            };

            self.frames.last_mut().unwrap().pc += 1;

            let opcode = Instruction::get_opcode(instr);

            // Periodic GC check (every 10000 instructions for better performance)
            instruction_count += 1;
            if instruction_count >= 10000 {
                instruction_count = 0;
                if self.gc.should_collect() {
                    self.collect_garbage();
                }
            }

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
                OpCode::VarArg => self.op_vararg(instr)?,
                OpCode::SetList => self.op_setlist(instr)?,
            }

            // Check if yielding after executing instruction
            if self.yielding {
                return Ok(LuaValue::nil());
            }
        }
    }

    // Opcode implementations
    #[inline(always)]
    fn op_move(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let base_ptr = self.current_frame().base_ptr;
        let value = self.get_register(base_ptr, b);
        self.set_register(base_ptr, a, value);
        Ok(())
    }

    #[inline(always)]
    fn op_loadk(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;
        let chunk = self.get_current_chunk()?;
        let constant = chunk.constants[bx].clone();
        let base_ptr = self.current_frame().base_ptr;
        self.set_register(base_ptr, a, constant);
        Ok(())
    }

    fn op_loadnil(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let base_ptr = self.current_frame().base_ptr;
        self.set_register(base_ptr, a, LuaValue::nil());
        Ok(())
    }

    fn op_loadbool(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr);
        let c = Instruction::get_c(instr);
        let base_ptr = self.current_frame().base_ptr;
        self.set_register(base_ptr, a, LuaValue::boolean(b != 0));
        if c != 0 {
            self.current_frame_mut().pc += 1;
        }
        Ok(())
    }

    fn op_newtable(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let table = self.create_table();
        let base_ptr = self.current_frame().base_ptr;
        self.set_register(base_ptr, a, table);
        Ok(())
    }

    fn op_gettable(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // Check types first to determine fast path
        let table = self.get_register(base_ptr, b);
        let key = self.get_register(base_ptr, c);

        if table.is_table() {
            // Use VM's table_get which handles metamethods
            let value = self.table_get(&table, &key).unwrap_or(LuaValue::nil());
            self.set_register(base_ptr, a, value);
            return Ok(());
        } else if table.is_userdata() {
            // Handle userdata __index metamethod
            let value = self.userdata_get(&table, &key).unwrap_or(LuaValue::nil());
            self.set_register(base_ptr, a, value);
            return Ok(());
        } else if table.is_string() {
            // Handle string indexing (uses string metatable)
            let value = self.string_get(&table, &key).unwrap_or(LuaValue::nil());
            self.set_register(base_ptr, a, value);
            return Ok(());
        }

        Err("Attempt to index a non-table value".to_string())
    }

    fn op_settable(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // Check types and get values
        let val_a = self.get_register(base_ptr, a);
        let val_b = self.get_register(base_ptr, b);
        let val_c = self.get_register(base_ptr, c);

        // Slow path: metamethods or non-table
        if val_a.is_table() {
            self.table_set(val_a, val_b, val_c)?;
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let table = self.get_register(base_ptr, b);

        // Optimized: use direct reference, no Rc clone
        if table.is_table() {
            // Fast path: direct integer access, no atomic ops
            let value = self
                .table_get(&table, &LuaValue::integer(c))
                .unwrap_or(LuaValue::nil());
            self.set_register(base_ptr, a, value);
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let table = self.get_register(base_ptr, a);
        let value = self.get_register(base_ptr, c);

        if table.is_table() {
            self.table_set(table, LuaValue::integer(b), value)?;
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

        let chunk = self.get_current_chunk()?;
        let key = chunk.constants[c].clone();
        let base_ptr = self.current_frame().base_ptr;
        let table = self.get_register(base_ptr, b);

        if table.is_table() {
            // Not found or not a string key: use generic get with metamethods
            let value = self.table_get(&table, &key).unwrap_or(LuaValue::nil());
            self.set_register(base_ptr, a, value);
            Ok(())
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

        let chunk = self.get_current_chunk()?;
        let key = chunk.constants[b].clone();
        let base_ptr = self.current_frame().base_ptr;
        let table = self.get_register(base_ptr, a);
        let value = self.get_register(base_ptr, c);

        if table.is_table() {
            // Fallback: use generic set with metamethods
            self.table_set(table, key, value)?;
            Ok(())
        } else {
            Err("Attempt to index a non-table value".to_string())
        }
    }

    #[inline]
    fn op_add(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // SAFETY: Compiler guarantees indices are within bounds
        // Ultra-fast path: direct tag comparison (no kind() overhead)
        unsafe {
            let left = self.register_stack.get_unchecked(base_ptr + b);
            let right = self.register_stack.get_unchecked(base_ptr + c);

            let left_tag = left.primary();
            let right_tag = right.primary();

            // Fast path: both integers (most common case)
            if left_tag == crate::lua_value::TAG_INTEGER
                && right_tag == crate::lua_value::TAG_INTEGER
            {
                let i = left.secondary() as i64;
                let j = right.secondary() as i64;
                *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::integer(i + j);
                return Ok(());
            }

            // Both floats
            if left_tag < crate::lua_value::NAN_BASE && right_tag < crate::lua_value::NAN_BASE {
                let l = f64::from_bits(left_tag);
                let r = f64::from_bits(right_tag);
                *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::float(l + r);
                return Ok(());
            }

            // Mixed int/float - convert to float
            if (left_tag == crate::lua_value::TAG_INTEGER || left_tag < crate::lua_value::NAN_BASE)
                && (right_tag == crate::lua_value::TAG_INTEGER
                    || right_tag < crate::lua_value::NAN_BASE)
            {
                let l = if left_tag == crate::lua_value::TAG_INTEGER {
                    left.secondary() as i64 as f64
                } else {
                    f64::from_bits(left_tag)
                };
                let r = if right_tag == crate::lua_value::TAG_INTEGER {
                    right.secondary() as i64 as f64
                } else {
                    f64::from_bits(right_tag)
                };
                *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::float(l + r);
                return Ok(());
            }
        }

        // Slow path: metamethods
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // SAFETY: Compiler guarantees indices are within bounds
        unsafe {
            let left = self.register_stack.get_unchecked(base_ptr + b);
            let right = self.register_stack.get_unchecked(base_ptr + c);

            match (left.kind(), right.kind()) {
                (LuaValueKind::Integer, LuaValueKind::Integer) => {
                    let i = left.as_integer().unwrap();
                    let j = right.as_integer().unwrap();
                    *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::integer(i - j);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Float) => {
                    let l = left.as_float().unwrap();
                    let r = right.as_float().unwrap();
                    *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::float(l - r);
                    return Ok(());
                }
                (LuaValueKind::Integer, LuaValueKind::Float) => {
                    let i = left.as_integer().unwrap();
                    let f = right.as_float().unwrap();
                    *self.register_stack.get_unchecked_mut(base_ptr + a) =
                        LuaValue::float(i as f64 - f);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Integer) => {
                    let f = left.as_float().unwrap();
                    let i = right.as_integer().unwrap();
                    *self.register_stack.get_unchecked_mut(base_ptr + a) =
                        LuaValue::float(f - i as f64);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Slow path
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // SAFETY: Compiler guarantees indices are within bounds
        unsafe {
            let left = self.register_stack.get_unchecked(base_ptr + b);
            let right = self.register_stack.get_unchecked(base_ptr + c);

            match (left.kind(), right.kind()) {
                (LuaValueKind::Integer, LuaValueKind::Integer) => {
                    let i = left.as_integer().unwrap();
                    let j = right.as_integer().unwrap();
                    *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::integer(i * j);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Float) => {
                    let l = left.as_float().unwrap();
                    let r = right.as_float().unwrap();
                    *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::float(l * r);
                    return Ok(());
                }
                (LuaValueKind::Integer, LuaValueKind::Float) => {
                    let i = left.as_integer().unwrap();
                    let f = right.as_float().unwrap();
                    *self.register_stack.get_unchecked_mut(base_ptr + a) =
                        LuaValue::float(i as f64 * f);
                    return Ok(());
                }
                (LuaValueKind::Float, LuaValueKind::Integer) => {
                    let f = left.as_float().unwrap();
                    let i = right.as_integer().unwrap();
                    *self.register_stack.get_unchecked_mut(base_ptr + a) =
                        LuaValue::float(f * i as f64);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Slow path
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let val_b = self.get_register(base_ptr, b);
        let val_c = self.get_register(base_ptr, c);

        // Fast path: division always returns float in Lua
        match (val_b.kind(), val_c.kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let i = val_b.as_integer().unwrap();
                let j = val_c.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::float(i as f64 / j as f64));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = val_b.as_float().unwrap();
                let r = val_c.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::float(l / r));
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = val_b.as_integer().unwrap();
                let f = val_c.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::float(i as f64 / f));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = val_b.as_float().unwrap();
                let i = val_c.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::float(f / i as f64));
                return Ok(());
            }
            _ => {}
        }

        // Slow path
        let left = val_b;
        let right = val_c;

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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let val_b = self.get_register(base_ptr, b);
        let val_c = self.get_register(base_ptr, c);

        // Fast path
        match (val_b.kind(), val_c.kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let i = val_b.as_integer().unwrap();
                let j = val_c.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::integer(i % j));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = val_b.as_float().unwrap();
                let r = val_c.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::float(l % r));
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = val_b.as_integer().unwrap();
                let f = val_c.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::float((i as f64) % f));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = val_b.as_float().unwrap();
                let i = val_c.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::float(f % (i as f64)));
                return Ok(());
            }
            _ => {}
        }

        // Slow path
        let left = val_b;
        let right = val_c;

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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

        match (&left, &right) {
            (l, r) if l.is_number() && r.is_number() => {
                let l_num = l.as_number().unwrap();
                let r_num = r.as_number().unwrap();
                self.set_register(base_ptr, a, LuaValue::number(l_num.powf(r_num)));
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let val = self.get_register(base_ptr, b);

        // Fast path: avoid clone
        match val.kind() {
            LuaValueKind::Integer => {
                let i = val.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::integer(-i));
                return Ok(());
            }
            LuaValueKind::Float => {
                let f = val.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::float(-f));
                return Ok(());
            }
            _ => {}
        }

        // Slow path: need to clone for metamethod
        let value = val;
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let val_b = self.get_register(base_ptr, b);
        let val_c = self.get_register(base_ptr, c);

        // Fast path
        match (val_b.kind(), val_c.kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let i = val_b.as_integer().unwrap();
                let j = val_c.as_integer().unwrap();
                if j == 0 {
                    return Err("attempt to divide by zero".to_string());
                }
                self.set_register(base_ptr, a, LuaValue::integer(i / j));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = val_b.as_float().unwrap();
                let r = val_c.as_float().unwrap();
                let result = (l / r).floor();
                self.set_register(base_ptr, a, LuaValue::integer(result as i64));
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = val_b.as_integer().unwrap();
                let f = val_c.as_float().unwrap();
                let result = (i as f64 / f).floor();
                self.set_register(base_ptr, a, LuaValue::integer(result as i64));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = val_b.as_float().unwrap();
                let i = val_c.as_integer().unwrap();
                let result = (f / i as f64).floor();
                self.set_register(base_ptr, a, LuaValue::integer(result as i64));
                return Ok(());
            }
            _ => {}
        }

        // Slow path
        let left = val_b;
        let right = val_c;

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
        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let value = self.get_register(base_ptr, b).is_truthy();
        self.set_register(base_ptr, a, LuaValue::boolean(!value));
        Ok(())
    }

    fn op_len(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let value = self.get_register(base_ptr, b);

        // Strings have raw length
        if let Some(s) = self.get_string(&value) {
            self.set_register(base_ptr, a, LuaValue::integer(s.as_str().len() as i64));
            return Ok(());
        }

        // Try __len metamethod for tables

        if value.is_table() {
            if self.call_unop_metamethod(&value, "__len", a)? {
                return Ok(());
            }

            // No __len metamethod, use raw length
            if let Some(table_id) = value.as_table_id() {
                let table = self
                    .object_pool
                    .get_table(table_id)
                    .ok_or("Missing table")?;
                let len = table.borrow().len() as i64;
                self.set_register(base_ptr, a, LuaValue::integer(len));
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
            let base_ptr = frame.base_ptr;
            let left = self.get_register(base_ptr, b);
            let right = self.get_register(base_ptr, c);

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
            let base_ptr = frame.base_ptr;
            self.set_register(base_ptr, a, LuaValue::boolean(result));
            return Ok(());
        }

        // Slow path: need to handle tables/strings/metamethods
        let (left, right) = {
            let frame = self.current_frame();
            let base_ptr = frame.base_ptr;
            (
                self.get_register(base_ptr, b),
                self.get_register(base_ptr, c),
            )
        };

        if self.values_equal(&left, &right) {
            let frame = self.current_frame_mut();
            let base_ptr = frame.base_ptr;
            self.set_register(base_ptr, a, LuaValue::boolean(true));
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
        let base_ptr = frame.base_ptr;
        self.set_register(base_ptr, a, LuaValue::boolean(false));
        Ok(())
    }

    #[inline]
    fn op_lt(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // Fast path: numeric comparison without clone
        let val_b = self.get_register(base_ptr, b);
        let val_c = self.get_register(base_ptr, c);

        match (val_b.kind(), val_c.kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let l = val_b.as_integer().unwrap();
                let r = val_c.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::boolean(l < r));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = val_b.as_float().unwrap();
                let r = val_c.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::boolean(l < r));
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = val_b.as_integer().unwrap();
                let f = val_c.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::boolean((i as f64) < f));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = val_b.as_float().unwrap();
                let i = val_c.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::boolean(f < (i as f64)));
                return Ok(());
            }
            _ => {}
        }

        // Slow path: strings and metamethods
        let left = val_b;
        let right = val_c;

        if let (Some(l), Some(r)) = (self.get_string(&left), self.get_string(&right)) {
            let frame = self.current_frame();
            let base_ptr = frame.base_ptr;
            self.set_register(base_ptr, a, LuaValue::boolean(l.as_str() < r.as_str()));
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // Fast path: numeric comparison without clone
        let val_b = self.get_register(base_ptr, b);
        let val_c = self.get_register(base_ptr, c);

        match (val_b.kind(), val_c.kind()) {
            (LuaValueKind::Integer, LuaValueKind::Integer) => {
                let l = val_b.as_integer().unwrap();
                let r = val_c.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::boolean(l <= r));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Float) => {
                let l = val_b.as_float().unwrap();
                let r = val_c.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::boolean(l <= r));
                return Ok(());
            }
            (LuaValueKind::Integer, LuaValueKind::Float) => {
                let i = val_b.as_integer().unwrap();
                let f = val_c.as_float().unwrap();
                self.set_register(base_ptr, a, LuaValue::boolean((i as f64) <= f));
                return Ok(());
            }
            (LuaValueKind::Float, LuaValueKind::Integer) => {
                let f = val_b.as_float().unwrap();
                let i = val_c.as_integer().unwrap();
                self.set_register(base_ptr, a, LuaValue::boolean(f <= (i as f64)));
                return Ok(());
            }
            _ => {}
        }

        // Slow path
        let left = val_b;
        let right = val_c;

        if let (Some(l), Some(r)) = (self.get_string(&left), self.get_string(&right)) {
            let frame = self.current_frame();
            let base_ptr = frame.base_ptr;
            self.set_register(base_ptr, a, LuaValue::boolean(l.as_str() <= r.as_str()));
            return Ok(());
        }

        if self.call_binop_metamethod(&left, &right, "__le", a)? {
            return Ok(());
        }

        if let Some(_) = self.get_metamethod(&left, "__lt") {
            if self.call_binop_metamethod(&right, &left, "__lt", a)? {
                let frame = self.current_frame();
                let base_ptr = frame.base_ptr;
                let result = self.get_register(base_ptr, a).is_truthy();
                self.set_register(base_ptr, a, LuaValue::boolean(!result));
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
    // Lua 5.4 semantics: for i = init, limit, step do ... end
    // FORPREP: Initialize loop by subtracting step from init (so first FORLOOP adds it back)
    #[inline]
    fn op_forprep(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let sbx = Instruction::get_sbx(instr);

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // R(A) = init, R(A+1) = limit, R(A+2) = step
        // Validate that all are numbers
        let val_a = self.get_register(base_ptr, a);
        let val_a1 = self.get_register(base_ptr, a + 1);
        let val_a2 = self.get_register(base_ptr, a + 2);

        if !val_a.is_number() {
            return Err("'for' initial value must be a number".to_string());
        }
        if !val_a1.is_number() {
            return Err("'for' limit must be a number".to_string());
        }
        if !val_a2.is_number() {
            return Err("'for' step must be a number".to_string());
        }

        // Fast path: Pure integer arithmetic (most common)
        if let (LuaValueKind::Integer, LuaValueKind::Integer) = (val_a.kind(), val_a2.kind()) {
            let init = val_a.as_integer().unwrap();
            let step = val_a2.as_integer().unwrap();

            // Subtract step so first FORLOOP iteration adds it back to get init
            let new_init = init.wrapping_sub(step);
            self.set_register(base_ptr, a, LuaValue::integer(new_init));
            self.set_register(base_ptr, a + 3, LuaValue::integer(new_init));
        } else {
            // Float path: Any operand is float or needs float precision
            let init = val_a.as_number().unwrap();
            let step = val_a2.as_number().unwrap();
            let new_init = init - step;
            self.set_register(base_ptr, a, LuaValue::float(new_init));
            self.set_register(base_ptr, a + 3, LuaValue::float(new_init));
        }

        // Jump forward to FORLOOP
        let frame = self.current_frame_mut();
        frame.pc = (frame.pc as i32 + sbx) as usize;
        Ok(())
    }

    // FORLOOP: Increment index and test loop condition
    // R(A) = index, R(A+1) = limit, R(A+2) = step, R(A+3) = loop variable
    // Lua 5.4 semantics: continue if (step > 0 and idx <= limit) or (step <= 0 and idx >= limit)
    fn op_forloop(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let sbx = Instruction::get_sbx(instr);

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // SAFETY: Compiler guarantees a, a+1, a+2, a+3 are within register bounds
        // Ultra-fast path: direct tag checking (no kind() overhead)
        unsafe {
            let idx_val = self.register_stack.get_unchecked(base_ptr + a);
            let limit_val = self.register_stack.get_unchecked(base_ptr + a + 1);
            let step_val = self.register_stack.get_unchecked(base_ptr + a + 2);

            let idx_tag = idx_val.primary();
            let limit_tag = limit_val.primary();
            let step_tag = step_val.primary();

            // Fast path: All integers (most common - > 95% of loops)
            if idx_tag == crate::lua_value::TAG_INTEGER
                && limit_tag == crate::lua_value::TAG_INTEGER
                && step_tag == crate::lua_value::TAG_INTEGER
            {
                let idx = idx_val.secondary() as i64;
                let limit = limit_val.secondary() as i64;
                let step = step_val.secondary() as i64;

                // Add step with wrapping (Lua 5.4 allows overflow)
                let new_idx = idx.wrapping_add(step);

                // Lua 5.4 loop condition: (step >= 0) ? (idx <= limit) : (idx >= limit)
                let continue_loop = if step >= 0 {
                    new_idx <= limit
                } else {
                    new_idx >= limit
                };

                // Update index register
                *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::integer(new_idx);

                if continue_loop {
                    // Update loop variable: R(A+3) = new_idx
                    *self.register_stack.get_unchecked_mut(base_ptr + a + 3) =
                        LuaValue::integer(new_idx);
                    // Jump back to loop body
                    let frame = self.current_frame_mut();
                    frame.pc = (frame.pc as i32 + sbx) as usize;
                }
                return Ok(());
            }

            // Slow path: Float or mixed types
            if idx_tag < crate::lua_value::NAN_BASE
                && limit_tag < crate::lua_value::NAN_BASE
                && step_tag < crate::lua_value::NAN_BASE
            {
                // All floats
                let idx = f64::from_bits(idx_tag);
                let limit = f64::from_bits(limit_tag);
                let step = f64::from_bits(step_tag);

                let new_idx = idx + step;
                let continue_loop = if step >= 0.0 {
                    new_idx <= limit
                } else {
                    new_idx >= limit
                };

                *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::float(new_idx);

                if continue_loop {
                    *self.register_stack.get_unchecked_mut(base_ptr + a + 3) =
                        LuaValue::float(new_idx);
                    let frame = self.current_frame_mut();
                    frame.pc = (frame.pc as i32 + sbx) as usize;
                }
                return Ok(());
            }

            // Mixed int/float - convert to float
            let is_num =
                |tag: u64| tag == crate::lua_value::TAG_INTEGER || tag < crate::lua_value::NAN_BASE;
            if is_num(idx_tag) && is_num(limit_tag) && is_num(step_tag) {
                let to_float = |val: &LuaValue, tag: u64| {
                    if tag == crate::lua_value::TAG_INTEGER {
                        val.secondary() as i64 as f64
                    } else {
                        f64::from_bits(tag)
                    }
                };

                let idx = to_float(idx_val, idx_tag);
                let limit = to_float(limit_val, limit_tag);
                let step = to_float(step_val, step_tag);

                let new_idx = idx + step;
                let continue_loop = if step >= 0.0 {
                    new_idx <= limit
                } else {
                    new_idx >= limit
                };

                *self.register_stack.get_unchecked_mut(base_ptr + a) = LuaValue::float(new_idx);

                if continue_loop {
                    *self.register_stack.get_unchecked_mut(base_ptr + a + 3) =
                        LuaValue::float(new_idx);
                    let frame = self.current_frame_mut();
                    frame.pc = (frame.pc as i32 + sbx) as usize;
                }
                return Ok(());
            }
        }

        Err("'for' loop variables must be numbers".to_string())
    }

    fn op_test(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let c = Instruction::get_c(instr);
        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        let is_true = self.get_register(base_ptr, a).is_truthy();
        let frame = self.current_frame_mut();
        if (is_true as u32) != c {
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_testset(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr);
        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        let val_b = self.get_register(base_ptr, b);
        let is_true = val_b.is_truthy();
        if (is_true as u32) == c {
            self.set_register(base_ptr, a, val_b);
        } else {
            let frame = self.current_frame_mut();
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_call(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let func = self.register_stack[base_ptr + a];

        // Fast path: Lua function (most common case)
        if func.is_function() {
            if let Some(lua_func_id) = func.as_function_id() {
                let (max_stack_size, param_count, is_vararg) = {
                    let func_ref = self
                        .object_pool
                        .get_function(lua_func_id)
                        .ok_or("Missing function")?;
                    let lua_func = func_ref.borrow();
                    (
                        lua_func.chunk.max_stack_size,
                        lua_func.chunk.param_count,
                        lua_func.chunk.is_vararg,
                    )
                };

                let frame = self.current_frame();
                let src_base = frame.base_ptr + a;
                let top = frame.top;
                let frame_base = frame.base_ptr;
                let arg_count = if b == 0 { top - a - 1 } else { b - 1 };

                let new_base = self.register_stack.len();

                // For vararg functions, we need extra space for varargs after max_stack_size
                let vararg_count = if arg_count > param_count {
                    arg_count - param_count
                } else {
                    0
                };
                let total_size = max_stack_size + vararg_count;
                self.ensure_stack_capacity(new_base + total_size);

                // Copy only the regular parameters (not varargs)
                let params_to_copy = arg_count.min(param_count);
                for i in 0..params_to_copy {
                    let src_idx = src_base + 1 + i;
                    if src_idx < frame_base + top {
                        let val = self.register_stack[src_idx];
                        self.register_stack[new_base + i] = val;
                    }
                }

                // If vararg function, copy extra args to the end of the frame
                let vararg_base = if is_vararg && vararg_count > 0 {
                    let vararg_start_in_stack = new_base + max_stack_size;
                    for i in 0..vararg_count {
                        let src_idx = src_base + 1 + param_count + i;
                        if src_idx < frame_base + top {
                            let val = self.register_stack[src_idx];
                            self.register_stack[vararg_start_in_stack + i] = val;
                        }
                    }
                    Some(vararg_start_in_stack)
                } else {
                    None
                };

                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let mut new_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    func,
                    new_base,
                    max_stack_size,
                    a,
                    if c == 0 { usize::MAX } else { c - 1 },
                );

                // Set up vararg information if function uses ...
                if is_vararg && vararg_count > 0 {
                    // vararg_start is now an ABSOLUTE index into register_stack
                    new_frame.vararg_start = vararg_base.unwrap();
                    new_frame.vararg_count = vararg_count;
                }

                self.frames.push(new_frame);
                return Ok(());
            }
        }

        // Fast path: C function
        if func.is_cfunction() {
            if let Some(cfunc) = func.as_cfunction() {
                let frame = self.current_frame();
                let base_ptr = frame.base_ptr;
                let top = frame.top;

                let arg_count = if b == 0 { top - a } else { b };
                let mut arg_registers = Vec::with_capacity(arg_count);
                arg_registers.push(func);

                // Copy arguments to registers
                for i in 1..b {
                    if a + i < top {
                        let val = self.register_stack[base_ptr + a + i];
                        arg_registers.push(val);
                    } else {
                        arg_registers.push(LuaValue::nil());
                    }
                }

                // Create temporary frame for CFunction
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let cfunc_base = self.register_stack.len();
                self.ensure_stack_capacity(cfunc_base + arg_registers.len());
                for (i, val) in arg_registers.iter().enumerate() {
                    self.register_stack[cfunc_base + i] = *val;
                }

                let temp_frame = LuaCallFrame::new_c_function(
                    frame_id,
                    func,
                    self.current_frame().pc,
                    cfunc_base,
                    arg_registers.len(),
                );

                self.frames.push(temp_frame);

                // Call the CFunction
                let multi_result = match cfunc(self) {
                    Ok(result) => result,
                    Err(e) => {
                        self.frames.pop();
                        return Err(e);
                    }
                };

                // Pop CFunction frame
                self.frames.pop();

                // Check if yielding - if so, don't store return values yet
                // They will be stored when resume() is called with new values
                // Save CALL instruction info for later
                if self.yielding {
                    if let Some(thread_rc) = &self.current_thread {
                        let mut thread = thread_rc.borrow_mut();
                        thread.yield_call_reg = Some(a);
                        thread.yield_call_nret = Some(if c == 0 { usize::MAX } else { c - 1 });
                    }
                    return Ok(());
                }

                // Store return values
                let all_returns = multi_result.all_values();
                let num_returns = all_returns.len();
                let num_expected = if c == 0 { num_returns } else { c - 1 };

                let frame = self.current_frame();
                let base_ptr = frame.base_ptr;
                let top = frame.top;

                for (i, value) in all_returns.into_iter().take(num_expected).enumerate() {
                    if a + i < top {
                        self.register_stack[base_ptr + a + i] = value;
                    }
                }

                // Fill remaining expected registers with nil
                for i in num_returns..num_expected {
                    if a + i < top {
                        self.register_stack[base_ptr + a + i] = LuaValue::nil();
                    }
                }

                return Ok(());
            }
        }

        // TODO: Slow path: Check for __call metamethod on non-functions (tables, userdata)
        // Requires implementing get_metamethod for non-table types
        // Currently disabled as the get_metamethod method needs to be updated for ID-based system

        Err("Attempt to call a non-function value".to_string())
    }

    fn op_return(&mut self, instr: u32) -> Result<LuaValue, String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        // Collect return values
        let num_returns = if b == 0 {
            let frame = self.current_frame();
            frame.top.saturating_sub(a)
        } else {
            b - 1
        };

        // Fast path: single return value (most common case)
        let return_value = if num_returns == 1 {
            let frame = self.current_frame();
            let base_ptr = frame.base_ptr;
            if a < frame.top {
                self.register_stack[base_ptr + a]
            } else {
                LuaValue::nil()
            }
        } else {
            LuaValue::nil()
        };

        // Save caller info before popping
        let caller_result_reg = self.current_frame().result_reg;
        let caller_num_results = self.current_frame().num_results;
        let exiting_frame_id = self.current_frame().frame_id;
        let exiting_base_ptr = self.current_frame().base_ptr;

        // Close upvalues for the exiting frame
        self.close_upvalues(exiting_frame_id);

        // Pop frame
        self.frames.pop();

        // Always save return values to self.return_values for callers that need them
        // (like call_function_internal)
        self.return_values.clear();
        let stack_top = self.register_stack.len().min(exiting_base_ptr + 100);
        for i in 0..num_returns {
            let idx = exiting_base_ptr + a + i;
            if idx < stack_top {
                self.return_values.push(self.register_stack[idx]);
            } else {
                self.return_values.push(LuaValue::nil());
            }
        }

        // If this is the top-level return (no caller frame), we're done
        if self.frames.is_empty() {
            return Ok(return_value);
        }

        // Fast path: no return values or caller doesn't care
        if num_returns == 0 {
            return Ok(return_value);
        }

        // Store return values directly to caller's registers
        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let top = frame.top;
        let num_to_copy = caller_num_results.min(num_returns);

        // Direct copy without intermediate Vec
        for i in 0..num_to_copy {
            if caller_result_reg + i < top && a + i < exiting_base_ptr + top {
                let val = self.register_stack[exiting_base_ptr + a + i];
                self.register_stack[base_ptr + caller_result_reg + i] = val;
            }
        }

        // Fill remaining expected results with nil
        if caller_num_results != usize::MAX {
            for i in num_to_copy..caller_num_results {
                if caller_result_reg + i < top {
                    self.register_stack[base_ptr + caller_result_reg + i] = LuaValue::nil();
                }
            }
        }

        Ok(return_value)
    }

    fn op_getupval(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let upvalue = self.get_current_upvalue(b)?;

        // Get value from upvalue with access to register_stack
        let value = upvalue.get_value(&self.frames, &self.register_stack);
        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        self.set_register(base_ptr, a, value);

        Ok(())
    }

    fn op_setupval(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let value = self.get_register(base_ptr, a);

        let upvalue = self.get_current_upvalue(b)?;

        // Set value to upvalue with access to register_stack
        upvalue.set_value(&mut self.frames, &mut self.register_stack, value);

        Ok(())
    }

    fn op_closure(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;

        // Dynamically resolve parent function chunk and frame info
        let (proto, parent_frame_id) = {
            let frame = self.current_frame();
            let parent_chunk = self.get_current_chunk()?;

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
                let upvalue = self.get_current_upvalue(desc.index as usize)?;
                upvalues.push(upvalue);
            }
        }

        // Create new function (closure)
        let func = self.create_function(proto, upvalues);

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        self.set_register(base_ptr, a, func);

        Ok(())
    }

    fn op_concat(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        // Binary concat with metamethod support
        let (left, right) = {
            let frame = self.current_frame();
            let base_ptr = frame.base_ptr;
            (
                self.get_register(base_ptr, b),
                self.get_register(base_ptr, c),
            )
        };

        // Try direct concatenation - use String capacity for efficiency
        let mut result = String::new();
        let mut success = false;

        if let Some(s) = self.get_string(&left) {
            result.push_str(s.as_str());
            success = true;
        } else if let Some(n) = left.as_number() {
            result.push_str(&n.to_string());
            success = true;
        } else if let Some(val) = self.call_tostring_metamethod(&left)? {
            if let Some(s) = self.get_string(&val) {
                result.push_str(s.as_str());
                success = true;
            }
        }

        if success {
            if let Some(s) = self.get_string(&right) {
                result.push_str(s.as_str());
            } else if let Some(n) = right.as_number() {
                result.push_str(&n.to_string());
            } else if let Some(val) = self.call_tostring_metamethod(&right)? {
                if let Some(s) = self.get_string(&val) {
                    result.push_str(s.as_str());
                } else {
                    success = false;
                }
            } else {
                success = false;
            }
        }

        if success {
            let string = self.create_string(&result);
            let frame = self.current_frame();
            let base_ptr = frame.base_ptr;
            self.set_register(base_ptr, a, string);
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

        let chunk = self.get_current_chunk()?;
        let name_val = &chunk.constants[bx];
        let value = self.get_global_by_lua_value(&name_val);
        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        match value {
            Some(v) => {
                self.set_register(base_ptr, a, v);
                Ok(())
            }
            None => {
                self.set_register(base_ptr, a, LuaValue::nil());
                Ok(())
            }
        }
    }

    fn op_setglobal(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;

        let chunk = self.get_current_chunk()?;
        let name_val = chunk.constants[bx].clone();
        let base_ptr = self.current_frame().base_ptr;
        let value = self.get_register(base_ptr, a);

        if let Some(name_str) = self.get_string(&name_val) {
            let name = name_str.as_str().to_string(); // Clone the string to avoid borrow issues
            self.set_global(&name, value);
            Ok(())
        } else {
            Err("Invalid global name".to_string())
        }
    }

    fn op_vararg(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let vararg_start = frame.vararg_start; // This is an ABSOLUTE index into register_stack
        let vararg_count = frame.vararg_count;

        if b == 0 {
            // Load all varargs
            for i in 0..vararg_count {
                let vararg_value = self.register_stack[vararg_start + i];
                self.set_register(base_ptr, a + i, vararg_value);
            }
            // Update frame top to reflect loaded varargs (for SetList with B=0)
            // Only increase top if needed, never decrease it
            let frame = self.current_frame_mut();
            let new_top = a + vararg_count;
            if new_top > frame.top {
                frame.top = new_top;
            }
        } else {
            // Load (b-1) varargs
            let count = b - 1;
            for i in 0..count {
                if i < vararg_count {
                    let vararg_value = self.register_stack[vararg_start + i];
                    self.set_register(base_ptr, a + i, vararg_value);
                } else {
                    // Pad with nil if not enough varargs
                    self.set_register(base_ptr, a + i, LuaValue::nil());
                }
            }
        }

        Ok(())
    }

    fn op_setlist(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize; // Table register
        let b = Instruction::get_b(instr) as usize; // Number of elements (0 = to stack top)
        let c = Instruction::get_c(instr) as usize; // Starting index in table (base)

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // Get the table
        let table_val = self.get_register(base_ptr, a);

        if !table_val.is_table() {
            return Err(format!("SetList: not a table, got {:?}", table_val));
        }

        // Optimized: use direct reference, no Rc clone needed for raw_set
        let table_id = table_val.as_table_id().unwrap();
        let table_ref = self
            .object_pool
            .get_table(table_id)
            .ok_or("SetList: missing table")?;

        // Determine how many elements to set
        let count = if b == 0 {
            // Use all remaining registers in the frame
            let top = self.current_frame().top;
            top - a - 1
        } else {
            b
        };

        // Set table elements: table[c + i] = R(a + i) for i = 1..count
        for i in 0..count {
            let value_reg = a + 1 + i;
            let value = self.get_register(base_ptr, value_reg);
            let key = LuaValue::integer((c + i + 1) as i64); // Lua arrays are 1-indexed
            table_ref.borrow_mut().raw_set(key, value);
        }

        Ok(())
    }

    // Helper methods
    #[inline(always)]
    fn current_frame(&self) -> &LuaCallFrame {
        unsafe { self.frames.last().unwrap_unchecked() }
    }

    #[inline(always)]
    fn current_frame_mut(&mut self) -> &mut LuaCallFrame {
        unsafe { self.frames.last_mut().unwrap_unchecked() }
    }

    pub fn values_equal(&self, left: &LuaValue, right: &LuaValue) -> bool {
        left == right
    }

    pub fn get_global(&mut self, name: &str) -> Option<LuaValue> {
        let key = self.create_string(name);
        if let Some(global_id) = self.globals.as_table_id() {
            let global = self.object_pool.get_table(global_id).unwrap();
            global.borrow().raw_get(&key)
        } else {
            None
        }
    }

    pub fn get_global_by_lua_value(&self, key: &LuaValue) -> Option<LuaValue> {
        if let Some(global_id) = self.globals.as_table_id() {
            let global = self.object_pool.get_table(global_id).unwrap();
            global.borrow().raw_get(key)
        } else {
            None
        }
    }

    pub fn set_global(&mut self, name: &str, value: LuaValue) {
        let key = self.create_string(name);

        if let Some(global_id) = self.globals.as_table_id() {
            let global = self.object_pool.get_table(global_id).unwrap();
            global.borrow_mut().raw_set(key, value);
        }
    }

    pub fn set_global_by_lua_value(&self, key: &LuaValue, value: LuaValue) {
        if let Some(global_id) = self.globals.as_table_id() {
            let global = self.object_pool.get_table(global_id).unwrap();
            global.borrow_mut().raw_set(key.clone(), value);
        }
    }

    /// Set the metatable for all strings
    /// In Lua, all strings share a metatable with __index pointing to the string library
    pub fn set_string_metatable(&mut self, _string_lib: LuaValue) {
        // Create the metatable
        // let metatable = self.create_table();

        // // Set __index to the string library table
        // let index_key = self.create_string("__index");
        // metatable.borrow_mut().raw_set(index_key, string_lib);

        // self.string_metatable = Some(metatable);
    }

    /// Get the shared string metatable
    pub fn get_string_metatable(&self) -> Option<Rc<RefCell<LuaTable>>> {
        self.string_metatable.clone()
    }

    /// Get FFI state (immutable)
    pub fn get_ffi_state(&self) -> &crate::ffi::FFIState {
        &self.ffi_state
    }

    /// Get FFI state (mutable)
    pub fn get_ffi_state_mut(&mut self) -> &mut crate::ffi::FFIState {
        &mut self.ffi_state
    }

    // ============ Coroutine Support ============

    /// Create a new thread (coroutine)
    pub fn create_thread(&mut self, func: LuaValue) -> Rc<RefCell<LuaThread>> {
        let thread = LuaThread {
            status: CoroutineStatus::Suspended,
            frames: Vec::new(),
            register_stack: Vec::with_capacity(256),
            return_values: Vec::new(),
            open_upvalues: Vec::new(),
            next_frame_id: 0,
            error_handler: None,
            yield_values: Vec::new(),
            resume_values: Vec::new(),
            yield_call_reg: None,
            yield_call_nret: None,
        };

        let thread_rc = Rc::new(RefCell::new(thread));

        // Store the function in the thread's first register
        thread_rc.borrow_mut().register_stack.push(func);

        thread_rc
    }

    /// Resume a coroutine
    pub fn resume_thread(
        &mut self,
        thread_val: LuaValue,
        args: Vec<LuaValue>,
    ) -> Result<(bool, Vec<LuaValue>), String> {
        // Extract Rc from LuaValue
        let thread_rc = unsafe {
            let ptr = thread_val.as_thread_ptr().ok_or("invalid thread")?;
            if ptr.is_null() {
                return Err("invalid thread".to_string());
            }
            let rc = Rc::from_raw(ptr);
            let cloned = rc.clone();
            std::mem::forget(rc); // Don't drop
            cloned
        };

        let status = thread_rc.borrow().status;

        match status {
            CoroutineStatus::Dead => {
                return Ok((
                    false,
                    vec![self.create_string("cannot resume dead coroutine")],
                ));
            }
            CoroutineStatus::Running => {
                return Ok((
                    false,
                    vec![self.create_string("cannot resume running coroutine")],
                ));
            }
            _ => {}
        }

        // Save current VM state
        let saved_frames = std::mem::take(&mut self.frames);
        let saved_stack = std::mem::take(&mut self.register_stack);
        let saved_returns = std::mem::take(&mut self.return_values);
        let saved_upvalues = std::mem::take(&mut self.open_upvalues);
        let saved_frame_id = self.next_frame_id;
        let saved_thread = self.current_thread.take();
        let saved_yielding = self.yielding;

        // Reset yielding flag
        self.yielding = false;

        let is_first_resume = {
            let thread = thread_rc.borrow();
            thread.frames.is_empty()
        };

        // Load thread state
        {
            let mut thread = thread_rc.borrow_mut();
            thread.status = CoroutineStatus::Running;
            self.frames = std::mem::take(&mut thread.frames);
            self.register_stack = std::mem::take(&mut thread.register_stack);
            self.return_values = std::mem::take(&mut thread.return_values);
            self.open_upvalues = std::mem::take(&mut thread.open_upvalues);
            self.next_frame_id = thread.next_frame_id;
        }

        self.current_thread = Some(thread_rc.clone());
        self.current_thread_value = Some(thread_val.clone());

        // Execute
        let result = if is_first_resume {
            // First resume: call the function
            let func = self
                .register_stack
                .get(0)
                .cloned()
                .unwrap_or(LuaValue::nil());
            self.call_function_internal(func, args)
        } else {
            // Resumed from yield:
            // Use saved CALL instruction info to properly store return values
            let (call_reg, call_nret) = {
                let thread = thread_rc.borrow();
                (thread.yield_call_reg, thread.yield_call_nret)
            };

            if let (Some(a), Some(num_expected)) = (call_reg, call_nret) {
                let frame = &self.frames[self.frames.len() - 1];
                let base_ptr = frame.base_ptr;
                let top = frame.top;

                // Store resume args as return values of the yield call
                let num_returns = args.len();
                let n = if num_expected == usize::MAX {
                    num_returns
                } else {
                    num_expected.min(num_returns)
                };

                for (i, value) in args.iter().take(n).enumerate() {
                    if base_ptr + a + i < self.register_stack.len() && a + i < top {
                        self.register_stack[base_ptr + a + i] = value.clone();
                    }
                } // Fill remaining expected registers with nil
                for i in num_returns..num_expected.min(top - a) {
                    if base_ptr + a + i < self.register_stack.len() {
                        self.register_stack[base_ptr + a + i] = LuaValue::nil();
                    }
                }

                // Clear the saved info
                thread_rc.borrow_mut().yield_call_reg = None;
                thread_rc.borrow_mut().yield_call_nret = None;
            }

            self.return_values = args;

            // Continue execution from where it yielded
            self.run().map(|v| vec![v])
        };

        // Check if thread yielded (use yielding flag, not yield_values)
        let did_yield = self.yielding;
        // let yield_values = {
        //     let thread = thread_rc.borrow();
        //     thread.yield_values.clone()
        // };

        // Save thread state back
        let final_result = if did_yield {
            // Thread yielded - save state and return yield values
            let mut thread = thread_rc.borrow_mut();
            thread.frames = std::mem::take(&mut self.frames);
            thread.register_stack = std::mem::take(&mut self.register_stack);
            thread.return_values = std::mem::take(&mut self.return_values);
            thread.open_upvalues = std::mem::take(&mut self.open_upvalues);
            thread.next_frame_id = self.next_frame_id;
            thread.status = CoroutineStatus::Suspended;

            let values = thread.yield_values.clone();
            thread.yield_values.clear();

            Ok((true, values))
        } else {
            // Thread completed or error
            let mut thread = thread_rc.borrow_mut();
            thread.frames = std::mem::take(&mut self.frames);
            thread.register_stack = std::mem::take(&mut self.register_stack);
            thread.return_values = std::mem::take(&mut self.return_values);
            thread.open_upvalues = std::mem::take(&mut self.open_upvalues);
            thread.next_frame_id = self.next_frame_id;

            match result {
                Ok(values) => {
                    thread.status = CoroutineStatus::Dead;
                    Ok((true, values))
                }
                Err(e) => {
                    thread.status = CoroutineStatus::Dead;
                    Ok((false, vec![self.create_string(&e)]))
                }
            }
        };

        // Restore VM state
        self.frames = saved_frames;
        self.register_stack = saved_stack;
        self.return_values = saved_returns;
        self.open_upvalues = saved_upvalues;
        self.next_frame_id = saved_frame_id;
        self.current_thread = saved_thread;
        self.current_thread_value = None; // Clear after resume completes
        self.yielding = saved_yielding;

        final_result
    }

    /// Yield from current coroutine
    pub fn yield_thread(&mut self, values: Vec<LuaValue>) -> Result<(), String> {
        if let Some(thread_rc) = &self.current_thread {
            // Store yield values in the thread
            thread_rc.borrow_mut().yield_values = values;
            thread_rc.borrow_mut().status = CoroutineStatus::Suspended;
            // Set yielding flag to interrupt execution
            self.yielding = true;
            Ok(())
        } else {
            Err("attempt to yield from outside a coroutine".to_string())
        }
    }

    /// Get value from table with metatable support
    /// Handles __index metamethod
    pub fn table_get(&mut self, lua_table_value: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        let Some(table_id) = lua_table_value.as_table_id() else {
            return None;
        };

        let lua_table = self.object_pool.get_table(table_id)?;

        // First try raw get
        let value = {
            let table = lua_table.borrow();
            table.raw_get(key).unwrap_or(LuaValue::nil())
        };

        if !value.is_nil() {
            return Some(value);
        }

        // If not found, check for __index metamethod
        let meta_value = {
            let table = lua_table.borrow();
            table.get_metatable()
        };

        if let Some(mt) = meta_value
            && let Some(meta_id) = mt.as_table_id()
        {
            let index_key = self.create_string("__index");
            let metatable = self.object_pool.get_table(meta_id)?;

            let index_value = {
                let mt_borrowed = metatable.borrow();
                mt_borrowed.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table
                    LuaValueKind::Table => return self.table_get(&index_val, key),

                    // __index is a function - call it with (table, key)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let args = vec![lua_table_value.clone(), key.clone()];
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
    pub fn userdata_get(
        &mut self,
        lua_userdata_value: &LuaValue,
        key: &LuaValue,
    ) -> Option<LuaValue> {
        let Some(userdata_id) = lua_userdata_value.as_userdata_id() else {
            return None;
        };

        let userdata = self.object_pool.get_userdata(userdata_id)?;
        // Check for __index metamethod
        let metatable = userdata.get_metatable();

        if let Some(mt_id) = metatable.as_table_id() {
            let index_key = self.create_string("__index");

            let index_value = {
                let mt_borrowed = self.object_pool.get_table(mt_id)?.borrow();
                mt_borrowed.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table
                    LuaValueKind::Table => return self.table_get(&index_val, key),

                    // __index is a function - call it with (userdata, key)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let args = vec![lua_userdata_value.clone(), key.clone()];
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

    /// Get value from string with metatable support
    /// Handles __index metamethod for strings
    pub fn string_get(&mut self, string_val: &LuaValue, key: &LuaValue) -> Option<LuaValue> {
        let index_key = self.create_string("__index");
        // Check for __index metamethod in string metatable
        if let Some(mt) = &self.string_metatable {
            let index_value = {
                let mt_borrowed = mt.borrow();
                mt_borrowed.raw_get(&index_key)
            };

            if let Some(index_val) = index_value {
                match index_val.kind() {
                    // __index is a table - look up in that table (this is the common case for strings)
                    LuaValueKind::Table => return self.table_get(&index_val, key),
                    // __index is a function - call it with (string, key)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let args = vec![string_val.clone(), key.clone()];
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
        lua_table_val: LuaValue,
        key: LuaValue,
        value: LuaValue,
    ) -> Result<(), String> {
        let Some(table_id) = lua_table_val.as_table_id() else {
            return Err("table_set: not a table".to_string());
        };

        let lua_table = self
            .object_pool
            .get_table(table_id)
            .ok_or("invalid table")?;

        // Check if key already exists
        let has_key = {
            let table = lua_table.borrow();
            table.raw_get(&key).map(|v| !v.is_nil()).unwrap_or(false)
        };

        if has_key {
            // Key exists, use raw set
            lua_table.borrow_mut().raw_set(key, value);
            return Ok(());
        }

        // Key doesn't exist, check for __newindex metamethod
        let meta_value = {
            let table = lua_table.borrow();
            table.get_metatable()
        };

        drop(lua_table);

        if let Some(mt) = meta_value
            && let Some(table_id) = mt.as_table_id()
        {
            let newindex_key = self.create_string("__newindex");
            let metatable = self
                .object_pool
                .get_table(table_id)
                .ok_or("missing metatable")?;

            let newindex_value = {
                let mt_borrowed = metatable.borrow();
                mt_borrowed.raw_get(&newindex_key)
            };

            if let Some(newindex_val) = newindex_value {
                match newindex_val.kind() {
                    // __newindex is a table - set in that table
                    LuaValueKind::Table => {
                        return self.table_set(newindex_val, key, value);
                    }
                    // __newindex is a function - call it with (table, key, value)
                    LuaValueKind::CFunction | LuaValueKind::Function => {
                        let args = vec![lua_table_val, key, value];
                        match self.call_metamethod(&newindex_val, &args) {
                            Ok(_) => return Ok(()),
                            Err(e) => return Err(e),
                        }
                    }
                    _ => {}
                }
            }
        }

        let lua_table = self
            .object_pool
            .get_table(table_id)
            .ok_or("invalid table")?;
        // No metamethod or key doesn't exist, use raw set
        lua_table.borrow_mut().raw_set(key, value);
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
                let dummy_func = LuaFunction {
                    chunk: Rc::new(Chunk {
                        code: Vec::new(),
                        constants: Vec::new(),
                        locals: Vec::new(),
                        upvalue_count: 0,
                        param_count: 0,
                        is_vararg: false,
                        max_stack_size: 16,
                        child_protos: Vec::new(),
                        upvalue_descs: Vec::new(),
                        source_name: Some("[C]".to_string()),
                        line_info: Vec::new(),
                    }),
                    upvalues: Vec::new(),
                };

                let dummy_func_id = self.object_pool.create_function(dummy_func);
                let dummy_func_value = LuaValue::function_id(dummy_func_id);

                let base_ptr = self.register_stack.len();
                let num_args = registers.len();
                self.ensure_stack_capacity(base_ptr + num_args);
                for (i, val) in registers.into_iter().enumerate() {
                    self.register_stack[base_ptr + i] = val;
                }

                let temp_frame =
                    LuaCallFrame::new_c_function(frame_id, dummy_func_value, 0, base_ptr, num_args);

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
                let lua_func_id = func.as_function_id().unwrap();

                // Get max_stack_size before mutable operations
                let max_stack_size = {
                    let lua_func_ref = self
                        .object_pool
                        .get_function(lua_func_id)
                        .ok_or("invalid function")?;
                    lua_func_ref.borrow().chunk.max_stack_size
                };

                // Call Lua function
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // Create a new call frame
                let base_ptr = self.register_stack.len();
                self.ensure_stack_capacity(base_ptr + max_stack_size);

                // Initialize with nil
                for i in 0..max_stack_size {
                    self.register_stack[base_ptr + i] = LuaValue::nil();
                }

                // Copy arguments to registers (starting from register 0)
                for (i, arg) in args.iter().enumerate() {
                    if i < max_stack_size {
                        self.register_stack[base_ptr + i] = arg.clone();
                    }
                }

                let new_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    func.clone(), // Clone the LuaValue to pass ownership
                    base_ptr,
                    max_stack_size,
                    0,
                    0, // Don't write back to caller's registers
                );

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
                    let function_value = self.frames[frame_idx].function_value;

                    // Dynamically resolve chunk
                    let chunk = if let Some(func_ref) = self.get_function(&function_value) {
                        func_ref.borrow().chunk.clone()
                    } else {
                        break Err("Invalid function in frame".to_string());
                    };

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
                        OpCode::GetTableI => self.op_gettable_i(instr),
                        OpCode::SetTableI => self.op_settable_i(instr),
                        OpCode::GetTableK => self.op_gettable_k(instr),
                        OpCode::SetTableK => self.op_settable_k(instr),
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
                        OpCode::VarArg => self.op_vararg(instr),
                        OpCode::SetList => self.op_setlist(instr),
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

        let result = !self.values_equal(&left, &right);
        self.set_register(base_ptr, a, LuaValue::boolean(result));
        Ok(())
    }

    fn op_gt(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self
            .get_register(base_ptr, b)
            .as_number()
            .ok_or("Comparison on non-number")?;
        let right = self
            .get_register(base_ptr, c)
            .as_number()
            .ok_or("Comparison on non-number")?;

        // Store boolean result in register A
        self.set_register(base_ptr, a, LuaValue::boolean(left > right));
        Ok(())
    }

    fn op_ge(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self
            .get_register(base_ptr, b)
            .as_number()
            .ok_or("Comparison on non-number")?;
        let right = self
            .get_register(base_ptr, c)
            .as_number()
            .ok_or("Comparison on non-number")?;

        // Store boolean result in register A
        self.set_register(base_ptr, a, LuaValue::boolean(left >= right));
        Ok(())
    }

    // Logical operators (short-circuit handled at compile time)
    fn op_and(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // Lua's 'and' returns first false value or last value
        let left = self.get_register(base_ptr, b);
        let result = if !left.is_truthy() {
            left
        } else {
            self.get_register(base_ptr, c)
        };
        self.set_register(base_ptr, a, result);
        Ok(())
    }

    fn op_or(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;

        // Lua's 'or' returns first true value or last value
        let left = self.get_register(base_ptr, b);
        let result = if left.is_truthy() {
            left
        } else {
            self.get_register(base_ptr, c)
        };
        self.set_register(base_ptr, a, result);
        Ok(())
    }

    // Bitwise operators
    fn op_band(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            self.set_register(base_ptr, a, LuaValue::integer(l & r));
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            self.set_register(base_ptr, a, LuaValue::integer(l | r));
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            self.set_register(base_ptr, a, LuaValue::integer(l ^ r));
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            self.set_register(base_ptr, a, LuaValue::integer(l << (r as u32)));
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let left = self.get_register(base_ptr, b);
        let right = self.get_register(base_ptr, c);

        if let (Some(l), Some(r)) = (left.as_integer(), right.as_integer()) {
            self.set_register(base_ptr, a, LuaValue::integer(l >> (r as u32)));
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

        let frame = self.current_frame();
        let base_ptr = frame.base_ptr;
        let value = self.get_register(base_ptr, b);

        if let Some(i) = value.as_integer() {
            self.set_register(base_ptr, a, LuaValue::integer(!i));
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
                    for reg_idx in 0..frame.top {
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
            let value = upvalue.get_value(&self.frames, &self.register_stack);
            upvalue.close(value);
        }

        // Remove closed upvalues from the open list
        self.open_upvalues.retain(|uv| uv.is_open());
    }

    /// Create a new table and register it with GC
    /// Create a string and register it with GC
    /// For short strings (≤64 bytes), use interning (global deduplication)
    /// Create a string value with automatic interning for short strings
    /// Returns LuaValue directly with ZERO allocation overhead for interned strings
    ///
    /// Performance characteristics:
    /// - Cache hit (interned): O(1) hash lookup, 0 allocations, 0 atomic ops
    /// - Cache miss (new): 1 Box allocation, GC registration, pool insertion
    /// - Long string: 1 Box allocation, GC registration, no pooling
    pub fn create_string(&mut self, s: &str) -> LuaValue {
        let id = self.object_pool.create_string(s);
        LuaValue::string_id(id)
    }

    /// Get string by LuaValue (resolves ID from object pool)
    pub fn get_string(&self, value: &LuaValue) -> Option<&LuaString> {
        if let Some(id) = value.as_string_id() {
            self.object_pool.get_string(id)
        } else {
            None
        }
    }

    /// Create a new table in object pool
    pub fn create_table(&mut self) -> LuaValue {
        let id = self.object_pool.create_table();
        LuaValue::table_id(id)
    }

    /// Get table by LuaValue (resolves ID from object pool)
    pub fn get_table(&self, value: &LuaValue) -> Option<&std::cell::RefCell<LuaTable>> {
        if let Some(id) = value.as_table_id() {
            self.object_pool.get_table(id)
        } else {
            None
        }
    }

    /// Helper: Set table field via raw_set
    pub fn table_set_raw(&mut self, table: &LuaValue, key: LuaValue, value: LuaValue) {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.borrow_mut().raw_set(key, value);
        }
    }

    /// Helper: Get table field via raw_get
    pub fn table_get_raw(&self, table: &LuaValue, key: &LuaValue) -> LuaValue {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.borrow().raw_get(key).unwrap_or(LuaValue::nil())
        } else {
            LuaValue::nil()
        }
    }

    /// Helper: Set table metatable
    pub fn table_set_metatable(&mut self, table: &LuaValue, metatable: Option<LuaValue>) {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.borrow_mut().set_metatable(metatable);
        }
    }

    /// Helper: Get table metatable
    pub fn table_get_metatable(&self, table: &LuaValue) -> Option<LuaValue> {
        if let Some(table_ref) = self.get_table(table) {
            table_ref.borrow().get_metatable()
        } else {
            None
        }
    }

    /// Create new userdata in object pool
    pub fn create_userdata(&mut self, data: crate::lua_value::LuaUserdata) -> LuaValue {
        let id = self.object_pool.create_userdata(data);
        LuaValue::userdata_id(id)
    }

    /// Get userdata by LuaValue (resolves ID from object pool)
    pub fn get_userdata(&self, value: &LuaValue) -> Option<&crate::lua_value::LuaUserdata> {
        if let Some(id) = value.as_userdata_id() {
            self.object_pool.get_userdata(id)
        } else {
            None
        }
    }

    /// Create a function in object pool
    pub fn create_function(&mut self, chunk: Rc<Chunk>, upvalues: Vec<Rc<LuaUpvalue>>) -> LuaValue {
        let func = LuaFunction { chunk, upvalues };
        let id = self.object_pool.create_function(func);
        LuaValue::function_id(id)
    }

    /// Get function by LuaValue (resolves ID from object pool)
    pub fn get_function(&self, value: &LuaValue) -> Option<&std::cell::RefCell<LuaFunction>> {
        if let Some(id) = value.as_function_id() {
            self.object_pool.get_function(id)
        } else {
            None
        }
    }

    /// Helper: Get chunk from current frame's function (for hot path)
    #[inline]
    fn get_current_chunk(&self) -> Result<std::rc::Rc<Chunk>, String> {
        let frame = self.current_frame();
        if let Some(func_ref) = self.get_function(&frame.function_value) {
            Ok(func_ref.borrow().chunk.clone())
        } else {
            Err("Invalid function in current frame".to_string())
        }
    }

    /// Helper: Get upvalue from current frame's function
    #[inline]
    fn get_current_upvalue(&self, index: usize) -> Result<std::rc::Rc<LuaUpvalue>, String> {
        let frame = self.current_frame();
        if let Some(func_ref) = self.get_function(&frame.function_value) {
            let func = func_ref.borrow();
            if index < func.upvalues.len() {
                Ok(func.upvalues[index].clone())
            } else {
                Err(format!("Invalid upvalue index: {}", index))
            }
        } else {
            Err("Invalid function in current frame".to_string())
        }
    }

    /// Check if GC should run and collect garbage if needed
    #[allow(unused)]
    fn maybe_collect_garbage(&mut self) {
        if self.gc.should_collect() {
            self.collect_garbage();
        }
    }

    // ============ GC Management ============

    fn register_chunk_constants(&mut self, chunk: &Chunk) {
        for value in &chunk.constants {
            match value.kind() {
                LuaValueKind::String | LuaValueKind::Table => {
                    // Table IDs are managed by object pool, no direct GC registration needed
                    // The object pool will handle lifetime management
                }
                LuaValueKind::Function => {
                    // Function IDs are managed by object pool, no direct GC registration needed
                    // Recursively register nested function chunks if needed
                    if let Some(func_id) = value.as_function_id() {
                        // Extract child chunk before recursion to avoid borrow conflicts
                        let child_chunk =
                            if let Some(func_ref) = self.object_pool.get_function(func_id) {
                                Some(func_ref.borrow().chunk.clone())
                            } else {
                                None
                            };

                        if let Some(child_chunk) = child_chunk {
                            self.register_chunk_constants(&child_chunk);
                        }
                    }
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
        roots.push(self.globals);

        // Add all frame registers as roots
        for frame in &self.frames {
            let base_ptr = frame.base_ptr;
            let top = frame.top;
            for i in 0..top {
                roots.push(self.register_stack[base_ptr + i]);
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

        // Run GC with mutable object pool reference
        self.gc.collect(&roots, &mut self.object_pool);
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
    fn get_metamethod(&mut self, value: &LuaValue, event: &str) -> Option<LuaValue> {
        match value.kind() {
            LuaValueKind::Table => {
                if let Some(table_id) = value.as_table_id() {
                    let metatable = {
                        let table = self.object_pool.get_table(table_id).expect("invalid table");
                        table.borrow().get_metatable()
                    };
                    if let Some(metatable) = metatable {
                        let key = self.create_string(event);
                        self.table_get(&metatable, &key)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            LuaValueKind::String => {
                let key = self.create_string(event);
                // All strings share a metatable
                if let Some(mt) = &self.string_metatable {
                    mt.borrow().raw_get(&key)
                } else {
                    None
                }
            }
            // TODO: Support metatables for userdata
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
                let func_id = metamethod.as_function_id().ok_or("Invalid function ID")?;
                let max_stack_size = {
                    let func_ref = self
                        .object_pool
                        .get_function(func_id)
                        .ok_or("Invalid function reference")?;
                    func_ref.borrow().chunk.max_stack_size
                };

                // Save current state
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                // Allocate registers in global stack
                let new_base = self.register_stack.len();
                self.ensure_stack_capacity(new_base + max_stack_size);

                // Copy arguments to new frame's registers
                for (i, arg) in args.iter().enumerate() {
                    if i < max_stack_size {
                        self.register_stack[new_base + i] = *arg;
                    }
                }

                let temp_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    metamethod,
                    new_base,
                    max_stack_size,
                    result_reg,
                    1, // expect 1 result
                );

                self.frames.push(temp_frame);

                // Execute the metamethod
                let result = self.run()?;

                // Store result in the target register
                if !self.frames.is_empty() {
                    let frame = self.current_frame();
                    let base_ptr = frame.base_ptr;
                    self.set_register(base_ptr, result_reg, result);
                }

                Ok(true)
            }
            LuaValueKind::CFunction => {
                let cf = metamethod.as_cfunction().unwrap();
                // Create temporary frame for CFunction
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let arg_count = args.len() + 1; // +1 for function itself
                let new_base = self.register_stack.len();
                self.ensure_stack_capacity(new_base + arg_count);

                self.register_stack[new_base] = LuaValue::cfunction(cf);
                for (i, arg) in args.iter().enumerate() {
                    self.register_stack[new_base + i + 1] = *arg;
                }

                let parent_pc = self.current_frame().pc;
                let temp_frame = LuaCallFrame::new_c_function(
                    frame_id, metamethod, parent_pc, new_base, arg_count,
                );

                self.frames.push(temp_frame);

                // Call the CFunction
                let multi_result = cf(self)?;

                // Pop temporary frame
                self.frames.pop();

                // Store result
                let values = multi_result.all_values();
                let result = values.first().cloned().unwrap_or(LuaValue::nil());
                let frame = self.current_frame();
                let base_ptr = frame.base_ptr;
                self.set_register(base_ptr, result_reg, result);

                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Call __tostring metamethod if it exists, return the string result
    pub fn call_tostring_metamethod(
        &mut self,
        lua_table_value: &LuaValue,
    ) -> Result<Option<LuaValue>, String> {
        // Check for __tostring metamethod
        if let Some(tostring_func) = self.get_metamethod(lua_table_value, "__tostring") {
            // Call the metamethod with the value as argument
            return self.call_metamethod(&tostring_func, &[lua_table_value.clone()]);
        }

        Ok(None)
    }

    /// Convert a value to string, calling __tostring metamethod if present
    pub fn value_to_string(&mut self, value: &LuaValue) -> Result<String, String> {
        // Handle string values directly
        if value.is_string() {
            if let Some(s) = self.get_string(value) {
                return Ok(s.as_str().to_string());
            }
        }
        
        if let Some(s) = self.call_tostring_metamethod(value)? {
            if let Some(str) = self.get_string(&s) {
                Ok(str.as_str().to_string())
            } else {
                Err("`__tostring` metamethod did not return a string".to_string())
            }
        } else {
            Ok(value.to_string_repr())
        }
    }

    /// Generate a stack traceback string
    pub fn generate_traceback(&self, error_msg: &str) -> String {
        let mut trace = format!("{}\nstack traceback:", error_msg);

        // Iterate through call frames from top to bottom (most recent first)
        for frame in self.frames.iter().rev() {
            // Dynamically resolve chunk for debug info
            let (source, line) = if let Some(func_ref) = self.get_function(&frame.function_value) {
                let func = func_ref.borrow();
                let chunk = &func.chunk;

                let source_str = frame
                    .source
                    .or_else(|| chunk.source_name.as_deref())
                    .unwrap_or("[?]");

                let pc = frame.pc.saturating_sub(1);
                let line_str = if !chunk.line_info.is_empty() && pc < chunk.line_info.len() {
                    chunk.line_info[pc].to_string()
                } else {
                    "?".to_string()
                };

                (source_str.to_string(), line_str)
            } else {
                ("[?]".to_string(), "?".to_string())
            };

            let func_name = frame.func_name.unwrap_or("?");

            trace.push_str(&format!(
                "\n\t{}:{}: in function '{}'",
                source, line, func_name
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
                // Simply clear all open upvalues to avoid dangling references
                self.open_upvalues.clear();

                // Now pop the frames
                while self.frames.len() > initial_frame_count {
                    self.frames.pop();
                }

                // Return error without traceback for now (can add later)
                let error_str = self.create_string(&error_msg);

                (false, vec![error_str])
            }
        }
    }

    /// Protected call with error handler
    pub fn protected_call_with_handler(
        &mut self,
        func: LuaValue,
        args: Vec<LuaValue>,
        err_handler: LuaValue,
    ) -> (bool, Vec<LuaValue>) {
        let old_handler = self.error_handler.clone();
        self.error_handler = Some(err_handler.clone());

        let initial_frame_count = self.frames.len();

        let result = self.call_function_internal(func, args);

        self.error_handler = old_handler;

        match result {
            Ok(values) => (true, values),
            Err(err_msg) => {
                self.open_upvalues.clear();

                while self.frames.len() > initial_frame_count {
                    self.frames.pop();
                }
                let err_str = self.create_string(&err_msg);
                let handler_result = self.call_function_internal(err_handler, vec![err_str]);

                match handler_result {
                    Ok(handler_values) => (false, handler_values),
                    Err(_) => {
                        let err_str = self.create_string(&err_msg);
                        (false, vec![err_str])
                    }
                }
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

                // Allocate registers in global stack
                let new_base = self.register_stack.len();
                let stack_size = 16; // enough for most cfunc calls
                self.ensure_stack_capacity(new_base + stack_size);

                self.register_stack[new_base] = func;
                for (i, arg) in args.iter().enumerate() {
                    if i + 1 < stack_size {
                        self.register_stack[new_base + i + 1] = arg.clone();
                    }
                }

                // Create dummy function and add to object pool
                let dummy_func = LuaFunction {
                    chunk: Rc::new(Chunk {
                        code: Vec::new(),
                        constants: Vec::new(),
                        locals: Vec::new(),
                        upvalue_count: 0,
                        param_count: 0,
                        is_vararg: false,
                        max_stack_size: stack_size,
                        child_protos: Vec::new(),
                        upvalue_descs: Vec::new(),
                        source_name: Some("[direct_call]".to_string()),
                        line_info: Vec::new(),
                    }),
                    upvalues: Vec::new(),
                };
                let dummy_func_id = self.object_pool.create_function(dummy_func);
                let dummy_func_value = LuaValue::function_id(dummy_func_id);

                let temp_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    dummy_func_value,
                    new_base,
                    stack_size,
                    0,
                    0,
                );

                self.frames.push(temp_frame);

                // Call CFunction - ensure frame is always popped even on error
                let result = match cfunc(self) {
                    Ok(r) => r,
                    Err(e) => {
                        self.frames.pop();
                        return Err(e);
                    }
                };

                self.frames.pop();

                Ok(result.all_values())
            }
            LuaValueKind::Function => {
                let lua_func_id = func.as_function_id().unwrap();

                // Get max_stack_size before entering the execution loop
                let max_stack_size = {
                    let lua_func_ref = self
                        .object_pool
                        .get_function(lua_func_id)
                        .ok_or("Invalid function reference")?;
                    lua_func_ref.borrow().chunk.max_stack_size
                };

                // For Lua function, use similar logic to call_metamethod
                let frame_id = self.next_frame_id;
                self.next_frame_id += 1;

                let new_base = self.register_stack.len();
                self.ensure_stack_capacity(new_base + max_stack_size);

                for (i, arg) in args.iter().enumerate() {
                    if i < max_stack_size {
                        self.register_stack[new_base + i] = arg.clone();
                    }
                }

                let new_frame = LuaCallFrame::new_lua_function(
                    frame_id,
                    func,
                    new_base,
                    max_stack_size,
                    0,
                    usize::MAX, // Want all return values
                );

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
                    let function_value = self.frames[frame_idx].function_value;

                    // Dynamically resolve chunk
                    let chunk = if let Some(func_ref) = self.get_function(&function_value) {
                        func_ref.borrow().chunk.clone()
                    } else {
                        break Err("Invalid function in frame".to_string());
                    };

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
                        OpCode::GetTableI => self.op_gettable_i(instr),
                        OpCode::SetTableI => self.op_settable_i(instr),
                        OpCode::GetTableK => self.op_gettable_k(instr),
                        OpCode::SetTableK => self.op_settable_k(instr),
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
                        OpCode::ForPrep => self.op_forprep(instr),
                        OpCode::ForLoop => self.op_forloop(instr),
                        OpCode::Test => self.op_test(instr),
                        OpCode::TestSet => self.op_testset(instr),
                        OpCode::Closure => self.op_closure(instr),
                        OpCode::GetUpval => self.op_getupval(instr),
                        OpCode::SetUpval => self.op_setupval(instr),
                        OpCode::VarArg => self.op_vararg(instr),
                        OpCode::SetList => self.op_setlist(instr),
                        _ => Err(format!("Unimplemented opcode: {:?}", opcode)),
                    };

                    if let Err(e) = step_result {
                        // Pop the frame on error
                        if self.frames.len() > initial_frame_count {
                            self.frames.pop();
                        }
                        break Err(e);
                    }

                    // Check if yielding
                    if self.yielding {
                        break Ok(());
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

// Cleanup: VM owns all strings in the string pool and must free them
impl Drop for LuaVM {
    fn drop(&mut self) {
        // Free all interned strings in the string pool
        // Note: GC will handle strings registered with it, but we need to handle
        // strings that are only in the pool (though in practice they should all be registered)
        for (_hash, ptr) in self.string_table.drain() {
            // Check if this pointer is still valid (GC might have freed it)
            // For safety, we just let GC handle all cleanup during final collection
            // The string_table is just for lookup, GC owns the actual objects
            let _ = ptr; // Just drop the reference
        }
    }
}
