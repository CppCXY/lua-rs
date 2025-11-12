// Lua Virtual Machine
// Executes compiled bytecode with register-based architecture

use crate::opcode::{OpCode, Instruction};
use crate::value::{Chunk, LuaValue, LuaTable, LuaFunction, LuaString};
use std::collections::HashMap;
use std::rc::Rc;

pub struct VM {
    // Global environment
    globals: HashMap<String, LuaValue>,
    
    // Call stack
    frames: Vec<CallFrame>,
    
    // GC root set (for future use)
    #[allow(dead_code)]
    gc_roots: Vec<LuaValue>,
}

struct CallFrame {
    function: Rc<LuaFunction>,
    pc: usize,              // Program counter
    registers: Vec<LuaValue>, // Register file
    #[allow(dead_code)]
    base: usize,            // Stack base for this frame (for future use)
}

impl VM {
    pub fn new() -> Self {
        let mut vm = VM {
            globals: HashMap::new(),
            frames: Vec::new(),
            gc_roots: Vec::new(),
        };
        
        // Register built-in functions
        vm.register_builtins();
        
        vm
    }

    fn register_builtins(&mut self) {
        // print function
        self.globals.insert("print".to_string(), LuaValue::nil());
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
        self.binary_arith(instr, |a, b| a + b)
    }

    fn op_sub(&mut self, instr: u32) -> Result<(), String> {
        self.binary_arith(instr, |a, b| a - b)
    }

    fn op_mul(&mut self, instr: u32) -> Result<(), String> {
        self.binary_arith(instr, |a, b| a * b)
    }

    fn op_div(&mut self, instr: u32) -> Result<(), String> {
        self.binary_arith(instr, |a, b| a / b)
    }

    fn op_mod(&mut self, instr: u32) -> Result<(), String> {
        self.binary_arith(instr, |a, b| a % b)
    }

    fn op_pow(&mut self, instr: u32) -> Result<(), String> {
        self.binary_arith(instr, |a, b| a.powf(b))
    }

    fn binary_arith<F>(&mut self, instr: u32, op: F) -> Result<(), String>
    where
        F: Fn(f64, f64) -> f64,
    {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        
        let frame = self.current_frame_mut();
        let left = frame.registers[b].as_number()
            .ok_or("Arithmetic on non-number")?;
        let right = frame.registers[c].as_number()
            .ok_or("Arithmetic on non-number")?;
        
        frame.registers[a] = LuaValue::number(op(left, right));
        Ok(())
    }

    fn op_unm(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let frame = self.current_frame_mut();
        let value = frame.registers[b].as_number()
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
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        
        let (left, right) = {
            let frame = self.current_frame();
            (frame.registers[b].clone(), frame.registers[c].clone())
        };
        
        let result = self.values_equal(&left, &right);
        let frame = self.current_frame_mut();
        if (result as u32) != a {
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_lt(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        
        let frame = self.current_frame_mut();
        let left = frame.registers[b].as_number()
            .ok_or("Comparison on non-number")?;
        let right = frame.registers[c].as_number()
            .ok_or("Comparison on non-number")?;
        
        if (left < right) as u32 != a {
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_le(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        
        let frame = self.current_frame_mut();
        let left = frame.registers[b].as_number()
            .ok_or("Comparison on non-number")?;
        let right = frame.registers[c].as_number()
            .ok_or("Comparison on non-number")?;
        
        if (left <= right) as u32 != a {
            frame.pc += 1;
        }
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
        let _c = Instruction::get_c(instr) as usize;
        
        let frame = self.current_frame();
        let func = frame.registers[a].clone();
        
        // Check for built-in functions
        if let Some(name) = self.get_builtin_name(&func) {
            return self.call_builtin(&name, a, b);
        }
        
        // Regular function call
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
        
        let return_value = if b > 1 {
            self.current_frame().registers[a].clone()
        } else {
            LuaValue::nil()
        };
        
        self.frames.pop();
        
        Ok(return_value)
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

    fn op_closure(&mut self, _instr: u32) -> Result<(), String> {
        // Simplified: closures not fully implemented
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
            _ => Err(format!("Unknown builtin: {}", name))
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
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        
        let (left, right) = {
            let frame = self.current_frame();
            (frame.registers[b].clone(), frame.registers[c].clone())
        };
        
        let result = !self.values_equal(&left, &right);
        let frame = self.current_frame_mut();
        if (result as u32) != a {
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_gt(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        
        let frame = self.current_frame_mut();
        let left = frame.registers[b].as_number()
            .ok_or("Comparison on non-number")?;
        let right = frame.registers[c].as_number()
            .ok_or("Comparison on non-number")?;
        
        if (left > right) as u32 != a {
            frame.pc += 1;
        }
        Ok(())
    }

    fn op_ge(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        
        let frame = self.current_frame_mut();
        let left = frame.registers[b].as_number()
            .ok_or("Comparison on non-number")?;
        let right = frame.registers[c].as_number()
            .ok_or("Comparison on non-number")?;
        
        if (left >= right) as u32 != a {
            frame.pc += 1;
        }
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
        
        let left = frame.registers[b].as_number().ok_or("Bitwise operation on non-number")? as i64;
        let right = frame.registers[c].as_number().ok_or("Bitwise operation on non-number")? as i64;
        frame.registers[a] = LuaValue::number((left & right) as f64);
        Ok(())
    }

    fn op_bor(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();
        
        let left = frame.registers[b].as_number().ok_or("Bitwise operation on non-number")? as i64;
        let right = frame.registers[c].as_number().ok_or("Bitwise operation on non-number")? as i64;
        frame.registers[a] = LuaValue::number((left | right) as f64);
        Ok(())
    }

    fn op_bxor(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();
        
        let left = frame.registers[b].as_number().ok_or("Bitwise operation on non-number")? as i64;
        let right = frame.registers[c].as_number().ok_or("Bitwise operation on non-number")? as i64;
        frame.registers[a] = LuaValue::number((left ^ right) as f64);
        Ok(())
    }

    fn op_shl(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();
        
        let left = frame.registers[b].as_number().ok_or("Bitwise operation on non-number")? as i64;
        let right = frame.registers[c].as_number().ok_or("Bitwise operation on non-number")? as u32;
        frame.registers[a] = LuaValue::number((left << right) as f64);
        Ok(())
    }

    fn op_shr(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();
        
        let left = frame.registers[b].as_number().ok_or("Bitwise operation on non-number")? as i64;
        let right = frame.registers[c].as_number().ok_or("Bitwise operation on non-number")? as u32;
        frame.registers[a] = LuaValue::number((left >> right) as f64);
        Ok(())
    }

    fn op_bnot(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let frame = self.current_frame_mut();
        
        let value = frame.registers[b].as_number().ok_or("Bitwise operation on non-number")? as i64;
        frame.registers[a] = LuaValue::number((!value) as f64);
        Ok(())
    }

    // Integer division
    fn op_idiv(&mut self, instr: u32) -> Result<(), String> {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        let frame = self.current_frame_mut();
        
        let left = frame.registers[b].as_number().ok_or("Division on non-number")?;
        let right = frame.registers[c].as_number().ok_or("Division on non-number")?;
        if right == 0.0 {
            return Err("Division by zero".to_string());
        }
        frame.registers[a] = LuaValue::number((left / right).floor());
        Ok(())
    }
}
