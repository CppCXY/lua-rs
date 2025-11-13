// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture

use crate::opcode::{Instruction, OpCode};
use crate::value::{Chunk, LuaFunction, LuaString, LuaTable, LuaValue};
use crate::builtin;
use std::collections::HashMap;
use std::rc::Rc;

pub struct VM {
    // Global environment
    globals: HashMap<String, LuaValue>,

    // Call stack
    pub frames: Vec<CallFrame>,

    // GC root set (for future use)
    #[allow(dead_code)]
    gc_roots: Vec<LuaValue>,
    
    // Multi-return value buffer (temporary storage for function returns)
    pub return_values: Vec<LuaValue>,
}

pub struct CallFrame {
    pub function: Rc<LuaFunction>,
    pub pc: usize,                // Program counter
    pub registers: Vec<LuaValue>, // Register file
    pub base: usize, // Stack base for this frame
    pub result_reg: usize,        // Register to store return value
    pub num_results: usize,       // Number of expected return values
}

impl VM {
    pub fn new() -> Self {
        let mut vm = VM {
            globals: HashMap::new(),
            frames: Vec::new(),
            gc_roots: Vec::new(),
            return_values: Vec::new(),
        };

        // Register built-in functions
        vm.register_builtins();

        vm
    }

    fn register_builtins(&mut self) {
        // Basic library
        self.globals.insert("print".to_string(), LuaValue::cfunction(builtin::lua_print));
        self.globals.insert("type".to_string(), LuaValue::cfunction(builtin::lua_type));
        self.globals.insert("assert".to_string(), LuaValue::cfunction(builtin::lua_assert));
        self.globals.insert("tostring".to_string(), LuaValue::cfunction(builtin::lua_tostring));
        self.globals.insert("tonumber".to_string(), LuaValue::cfunction(builtin::lua_tonumber));
        
        // Iterator functions
        self.globals.insert("next".to_string(), LuaValue::cfunction(builtin::lua_next));
        self.globals.insert("pairs".to_string(), LuaValue::cfunction(builtin::lua_pairs));
        self.globals.insert("ipairs".to_string(), LuaValue::cfunction(builtin::lua_ipairs));
        
        // Table library (as a table)
        let mut table_lib = LuaTable::new();
        table_lib.set(
            LuaValue::string(LuaString::new("insert".to_string())),
            LuaValue::cfunction(builtin::table_insert),
        );
        table_lib.set(
            LuaValue::string(LuaString::new("remove".to_string())),
            LuaValue::cfunction(builtin::table_remove),
        );
        self.globals.insert("table".to_string(), LuaValue::table(table_lib));
    }

    pub fn execute(&mut self, chunk: Rc<Chunk>) -> Result<LuaValue, String> {
        // Create main function
        let main_func = LuaFunction {
            chunk: chunk.clone(),
            upvalues: Vec::new(),
        };

        // Create initial call frame
        let frame = CallFrame {
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
            let chunk = &self.frames[frame_idx].function.chunk;

            if pc >= chunk.code.len() {
                // End of code
                self.frames.pop();
                continue;
            }

            let instr = chunk.code[pc];
            self.frames[frame_idx].pc += 1;

            // Decode and execute
            let opcode = Instruction::get_opcode(instr);

            match opcode {
                OpCode::Move => self.op_move(instr)?,
                OpCode::LoadK => self.op_loadk(instr)?,
                OpCode::LoadNil => self.op_loadnil(instr)?,
                OpCode::LoadBool => self.op_loadbool(instr)?,
                OpCode::NewTable => self.op_newtable(instr)?,
                OpCode::GetTable => self.op_gettable(instr)?,
                OpCode::SetTable => self.op_settable(instr)?,
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
    fn op_move(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let frame = self.current_frame_mut();
        frame.registers[a] = frame.registers[b].clone();
        Ok(())
    }

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
        let frame = self.current_frame_mut();
        frame.registers[a] = LuaValue::table(LuaTable::new());
        Ok(())
    }

    fn op_gettable(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let table = frame.registers[b].clone();
        let key = frame.registers[c].clone();

        if let Some(tbl) = table.as_table() {
            let value = tbl.borrow().get(&key).unwrap_or(LuaValue::nil());
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

        let frame = self.current_frame_mut();
        let table = frame.registers[a].clone();
        let key = frame.registers[b].clone();
        let value = frame.registers[c].clone();

        if let Some(tbl) = table.as_table() {
            tbl.borrow_mut().set(key, value);
            Ok(())
        } else {
            Err("Attempt to index a non-table value".to_string())
        }
    }

    fn op_add(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = &frame.registers[b];
        let right = &frame.registers[c];
        match (left, right) {
            (LuaValue::Float(l), LuaValue::Float(r)) => {
                frame.registers[a] = LuaValue::number(l + r);
                Ok(())
            }
            (LuaValue::Integer(i), LuaValue::Integer(j)) => {
                frame.registers[a] = LuaValue::integer(i + j);
                Ok(())
            }
            (left, right) if left.is_number() && right.is_number() => {
                let l = left.as_number().unwrap();
                let r = right.as_number().unwrap();
                frame.registers[a] = LuaValue::number(l + r);
                Ok(())
            }
            _ => Err("Addition on non-number values".to_string()),
        }
    }

    fn op_sub(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = &frame.registers[b];
        let right = &frame.registers[c];
        match (left, right) {
            (LuaValue::Float(l), LuaValue::Float(r)) => {
                frame.registers[a] = LuaValue::number(l - r);
                Ok(())
            }
            (LuaValue::Integer(i), LuaValue::Integer(j)) => {
                frame.registers[a] = LuaValue::integer(i - j);
                Ok(())
            }
            (left, right) if left.is_number() && right.is_number() => {
                let l = left.as_number().unwrap();
                let r = right.as_number().unwrap();
                frame.registers[a] = LuaValue::number(l - r);
                Ok(())
            }
            _ => Err("Subtraction on non-number values".to_string()),
        }
    }

    fn op_mul(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = &frame.registers[b];
        let right = &frame.registers[c];
        match (left, right) {
            (LuaValue::Float(l), LuaValue::Float(r)) => {
                frame.registers[a] = LuaValue::number(l * r);
                Ok(())
            }
            (LuaValue::Integer(i), LuaValue::Integer(j)) => {
                frame.registers[a] = LuaValue::integer(i * j);
                Ok(())
            }
            (left, right) if left.is_number() && right.is_number() => {
                let l = left.as_number().unwrap();
                let r = right.as_number().unwrap();
                frame.registers[a] = LuaValue::number(l * r);
                Ok(())
            }
            _ => Err("Multiplication on non-number values".to_string()),
        }
    }

    fn op_div(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let left = &frame.registers[b];
        let right = &frame.registers[c];
        match (left, right) {
            (LuaValue::Float(l), LuaValue::Float(r)) => {
                frame.registers[a] = LuaValue::number(l / r);
                Ok(())
            }
            (left, right) if left.is_number() && right.is_number() => {
                let l = left.as_number().unwrap();
                let r = right.as_number().unwrap();
                let value = l / r;
                if value.fract() == 0.0 {
                    frame.registers[a] = LuaValue::integer(value as i64);
                } else {
                    frame.registers[a] = LuaValue::number(value);
                }
                Ok(())
            }
            _ => Err("Division on non-number values".to_string()),
        }
    }

    fn op_mod(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();
        let left = &frame.registers[b];
        let right = &frame.registers[c];
        match (left, right) {
            (LuaValue::Integer(i), LuaValue::Integer(j)) => {
                frame.registers[a] = LuaValue::integer(i % j);
                Ok(())
            }
            (left, right) if left.is_integer() && right.is_integer() => {
                let l = left.as_integer().unwrap();
                let r = right.as_integer().unwrap();
                frame.registers[a] = LuaValue::integer(l % r);
                Ok(())
            }
            _ => Err("Modulo on non-number values".to_string()),
        }
    }

    fn op_pow(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();
        let left = &frame.registers[b];
        let right = &frame.registers[c];
        match (left, right) {
            (LuaValue::Float(l), LuaValue::Float(r)) => {
                frame.registers[a] = LuaValue::number(l.powf(*r));
                Ok(())
            }
            (LuaValue::Integer(i), LuaValue::Integer(j)) => {
                frame.registers[a] = LuaValue::number((*i as f64).powf(*j as f64));
                Ok(())
            }
            (left, right) if left.is_number() && right.is_number() => {
                let l = left.as_number().unwrap();
                let r = right.as_number().unwrap();
                frame.registers[a] = LuaValue::number(l.powf(r));
                Ok(())
            }
            _ => Err("Exponentiation on non-number values".to_string()),
        }
    }

    fn op_unm(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let frame = self.current_frame_mut();
        let value = frame.registers[b]
            .as_number()
            .ok_or("Unary minus on non-number")?;
        frame.registers[a] = LuaValue::number(-value);
        Ok(())
    }

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
        let frame = self.current_frame_mut();

        let len = if let Some(s) = frame.registers[b].as_string() {
            s.as_str().len() as f64
        } else if let Some(t) = frame.registers[b].as_table() {
            t.borrow().len() as f64
        } else {
            return Err("Length of non-sequence value".to_string());
        };

        frame.registers[a] = LuaValue::number(len);
        Ok(())
    }

    fn op_eq(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let (left, right) = {
            let frame = self.current_frame();
            (frame.registers[b].clone(), frame.registers[c].clone())
        };

        let result = self.values_equal(&left, &right);
        let frame = self.current_frame_mut();
        // Store boolean result in register A
        frame.registers[a] = LuaValue::boolean(result);
        Ok(())
    }

    fn op_lt(&mut self, instr: u32) -> Result<(), String> {
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
        frame.registers[a] = LuaValue::boolean(left < right);
        Ok(())
    }

    fn op_le(&mut self, instr: u32) -> Result<(), String> {
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
        frame.registers[a] = LuaValue::boolean(left <= right);
        Ok(())
    }

    fn op_jmp(&mut self, instr: u32) -> Result<(), String> {
        let sbx = Instruction::get_sbx(instr);
        let frame = self.current_frame_mut();
        frame.pc = (frame.pc as i32 + sbx) as usize;
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
            let temp_frame = CallFrame {
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
            
            let num_expected = if c == 0 {
                num_returns
            } else {
                c - 1
            };
            
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

        // Check for built-in functions (old system, may remove later)
        if let Some(name) = self.get_builtin_name(&func) {
            return self.call_builtin(&name, a, b);
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

            let new_frame = CallFrame {
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
        let frame = self.current_frame_mut();

        if b < frame.function.upvalues.len() {
            frame.registers[a] = frame.function.upvalues[b].clone();
        }
        Ok(())
    }

    fn op_setupval(&mut self, instr: u32) -> Result<(), String> {
        let _a = Instruction::get_a(instr) as usize;
        let _b = Instruction::get_b(instr) as usize;

        // Note: Upvalues not fully implemented yet
        // let value = self.current_frame().registers[a].clone();
        // let frame = self.current_frame_mut();

        // if b < frame.function.upvalues.len() {
        //     frame.function.upvalues[b] = value;
        // }
        Ok(())
    }

    fn op_closure(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;
        
        let frame = self.current_frame();
        let parent_chunk = &frame.function.chunk;
        
        // Get the child chunk (prototype)
        if bx >= parent_chunk.child_protos.len() {
            return Err(format!("Invalid prototype index: {}", bx));
        }
        
        let proto = parent_chunk.child_protos[bx].clone();
        
        // Create new function (closure)
        // TODO: Capture upvalues properly
        let func = LuaFunction {
            chunk: proto,
            upvalues: Vec::new(),
        };
        
        let frame = self.current_frame_mut();
        frame.registers[a] = LuaValue::Function(Rc::new(func));
        
        Ok(())
    }

    fn op_concat(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;

        let frame = self.current_frame_mut();
        let mut result = String::new();

        for i in b..=c {
            if let Some(s) = frame.registers[i].as_string() {
                result.push_str(s.as_str());
            } else if let Some(n) = frame.registers[i].as_number() {
                result.push_str(&n.to_string());
            }
        }

        frame.registers[a] = LuaValue::string(LuaString::new(result));
        Ok(())
    }

    fn op_getglobal(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let bx = Instruction::get_bx(instr) as usize;

        let frame = self.current_frame();
        let name_val = frame.function.chunk.constants[bx].clone();

        if let Some(name_str) = name_val.as_string() {
            let name = name_str.as_str().to_string();
            let value = self.globals.get(&name).cloned().unwrap_or(LuaValue::nil());

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
            let name = name_str.as_str().to_string();
            self.globals.insert(name, value);
            Ok(())
        } else {
            Err("Invalid global name".to_string())
        }
    }

    // Helper methods
    fn current_frame(&self) -> &CallFrame {
        self.frames.last().expect("No active frame")
    }

    fn current_frame_mut(&mut self) -> &mut CallFrame {
        self.frames.last_mut().expect("No active frame")
    }

    fn values_equal(&self, left: &LuaValue, right: &LuaValue) -> bool {
        if left.is_nil() && right.is_nil() {
            true
        } else if let (Some(l), Some(r)) = (left.as_boolean(), right.as_boolean()) {
            l == r
        } else if let (Some(l), Some(r)) = (left.as_number(), right.as_number()) {
            l == r
        } else if let (Some(l), Some(r)) = (left.as_string(), right.as_string()) {
            l.as_str() == r.as_str()
        } else {
            false
        }
    }

    fn get_builtin_name(&self, _func: &LuaValue) -> Option<String> {
        // Simplified: check if it's a known builtin
        None
    }

    fn call_builtin(&mut self, name: &str, a: usize, b: usize) -> Result<(), String> {
        match name {
            "print" => {
                let frame = self.current_frame();
                for i in 1..b {
                    if a + i < frame.registers.len() {
                        print!("{:?} ", frame.registers[a + i]);
                    }
                }
                println!();
                Ok(())
            }
            _ => Err(format!("Unknown builtin: {}", name)),
        }
    }

    pub fn set_global(&mut self, name: String, value: LuaValue) {
        self.globals.insert(name, value);
    }

    pub fn get_global(&self, name: &str) -> Option<LuaValue> {
        self.globals.get(name).cloned()
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

        let left = frame.registers[b]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        let right = frame.registers[c]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        frame.registers[a] = LuaValue::integer(left & right);
        Ok(())
    }

    fn op_bor(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();

        let left = frame.registers[b]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        let right = frame.registers[c]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        frame.registers[a] = LuaValue::integer(left | right);
        Ok(())
    }

    fn op_bxor(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();

        let left = frame.registers[b]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        let right = frame.registers[c]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        frame.registers[a] = LuaValue::integer(left ^ right);
        Ok(())
    }

    fn op_shl(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();

        let left = frame.registers[b]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        let right = frame.registers[c]
            .as_integer()
            .ok_or("Bitwise operation requires integer")? as u32;
        frame.registers[a] = LuaValue::integer(left << right);
        Ok(())
    }

    fn op_shr(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();

        let left = frame.registers[b]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        let right = frame.registers[c]
            .as_integer()
            .ok_or("Bitwise operation requires integer")? as u32;
        frame.registers[a] = LuaValue::integer(left >> right);
        Ok(())
    }

    fn op_bnot(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let frame = self.current_frame_mut();

        let value = frame.registers[b]
            .as_integer()
            .ok_or("Bitwise operation requires integer")?;
        frame.registers[a] = LuaValue::integer(!value);
        Ok(())
    }

    // Integer division
    fn op_idiv(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();

        let left = frame.registers[b]
            .as_number()
            .ok_or("Division on non-number")?;
        let right = frame.registers[c]
            .as_number()
            .ok_or("Division on non-number")?;
        if right == 0.0 {
            return Err("Division by zero".to_string());
        }
        // Integer division returns integer if both operands can be integers
        let result = (left / right).floor();
        if let (Some(l), Some(r)) = (
            frame.registers[b].as_integer(),
            frame.registers[c].as_integer(),
        ) {
            if r != 0 {
                frame.registers[a] = LuaValue::integer(l / r);
                return Ok(());
            }
        }
        frame.registers[a] = LuaValue::number(result);
        Ok(())
    }
}
