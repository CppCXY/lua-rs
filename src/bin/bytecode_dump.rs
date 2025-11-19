use lua_rs::Chunk;
use lua_rs::LuaVM;
use lua_rs::lua_vm::{Instruction, OpCode};
use std::env;
use std::fs;

// Import OpMode if it's re-exported, otherwise we'll use the method directly

fn main() {
    let args: Vec<String> = env::args().collect();

    let source = if args.len() > 1 {
        // Read from file
        let filename = &args[1];
        match fs::read_to_string(filename) {
            Ok(content) => {
                println!("=== File: {} ===\n", filename);
                content
            }
            Err(e) => {
                eprintln!("Error reading file '{}': {}", filename, e);
                std::process::exit(1);
            }
        }
    } else {
        println!("Usage: bytecode_dump <source_file.lua>");
        std::process::exit(0);
    };

    let mut vm = LuaVM::new();
    match vm.compile(&source) {
        Ok(chunk) => {
            println!("=== Bytecode Analysis ===\n");
            println!("Total instructions: {}", chunk.code.len());
            println!("Constants: {}", chunk.constants.len());
            println!("Registers used: ~{}", estimate_registers(&chunk));
            println!("\n{:<4} {:<14} {:<6} {:<10} {:<10} {:<10} {}", "PC", "OpCode", "A", "B/Bx", "C", "Raw Hex", "Details");
            println!("{}", "-".repeat(130));

            for (pc, &instr) in chunk.code.iter().enumerate() {
                let opcode = Instruction::get_opcode(instr);
                let a = Instruction::get_a(instr);
                let b = Instruction::get_b(instr);
                let c = Instruction::get_c(instr);
                let bx = Instruction::get_bx(instr);
                let sbx = Instruction::get_sbx(instr);
                let k = Instruction::get_k(instr);
                
                // Determine display format based on opcode
                let (a_display, b_display, c_display) = match opcode {
                    // ABx format opcodes
                    OpCode::LoadK | OpCode::Closure => {
                        (format!("{}", a), format!("{}", bx), String::new())
                    }
                    // AsBx format opcodes  
                    OpCode::LoadI | OpCode::LoadF | OpCode::ForLoop | 
                    OpCode::ForPrep | OpCode::TForPrep | OpCode::TForLoop => {
                        let c_str = if matches!(opcode, OpCode::LoadF) { format!("{}", c) } else { String::new() };
                        (format!("{}", a), format!("{}", sbx), c_str)
                    }
                    // Ax format
                    OpCode::ExtraArg => {
                        (format!("{}", Instruction::get_ax(instr)), String::new(), String::new())
                    }
                    // sJ format - JMP has no A parameter, sJ is the jump offset
                    OpCode::Jmp => {
                        (String::new(), format!("{}", Instruction::get_sj(instr)), String::new())
                    }
                    // RETURN and CALL show k as third column
                    OpCode::Return | OpCode::TailCall => {
                        let k_str = if k { "1".to_string() } else { "0".to_string() };
                        (format!("{}", a), format!("{}", b), k_str)
                    }
                    // ABC format (default)
                    _ => {
                        (format!("{}", a), format!("{}", b), format!("{}{}", c, if k { " k" } else { "" }))
                    }
                };

                let details = match opcode {
                    // Load/Move operations
                    OpCode::Move => format!("R({}) := R({})", a, b),
                    OpCode::LoadI => format!("R({}) := {}", a, sbx),
                    OpCode::LoadF => format!("R({}) := float({},{})", a, sbx, c),
                    OpCode::LoadK => format!("R({}) := K({})", a, bx),
                    OpCode::LoadKX => format!("R({}) := K[extra]", a),
                    OpCode::LoadFalse => format!("R({}) := false", a),
                    OpCode::LFalseSkip => format!("R({}) := false; PC++", a),
                    OpCode::LoadTrue => format!("R({}) := true", a),
                    OpCode::LoadNil => format!("R({})..R({}) := nil", a, a + b),
                    
                    // Upvalue operations
                    OpCode::GetUpval => format!("R({}) := UpValue[{}]", a, b),
                    OpCode::SetUpval => format!("UpValue[{}] := R({})", b, a),
                    
                    // Table get operations
                    OpCode::GetTabUp => format!("R({}) := UpValue[{}][K({})]", a, b, c),
                    OpCode::GetTable => format!("R({}) := R({})[R({})]", a, b, c),
                    OpCode::GetI => format!("R({}) := R({})[{}]", a, b, c),
                    OpCode::GetField => format!("R({}) := R({})[K({})]", a, b, c),
                    
                    // Table set operations
                    OpCode::SetTabUp => {
                        let k = Instruction::get_k(instr);
                        if k {
                            format!("UpValue[{}][K({})] := K({})", a, b, c)
                        } else {
                            format!("UpValue[{}][K({})] := R({})", a, b, c)
                        }
                    }
                    OpCode::SetTable => format!("R({})[R({})] := R({})", a, b, c),
                    OpCode::SetI => {
                        let k = Instruction::get_k(instr);
                        if k {
                            format!("R({})[{}] := K({})", a, c, b)
                        } else {
                            format!("R({})[{}] := R({})", a, c, b)
                        }
                    }
                    OpCode::SetField => {
                        let k = Instruction::get_k(instr);
                        if k {
                            format!("R({})[K({})] := K({})", a, b, c)
                        } else {
                            format!("R({})[K({})] := R({})", a, b, c)
                        }
                    }
                    
                    // Table creation
                    OpCode::NewTable => format!("R({}) := {{}} (array={}, hash={})", a, b, c),
                    
                    // Self call
                    OpCode::Self_ => format!("R({}+1) := R({}); R({}) := R({})[RK({})]", a, b, a, b, c),
                    
                    // Arithmetic with immediate/constant
                    OpCode::AddI => format!("R({}) := R({}) + {}", a, b, c as i32),
                    OpCode::AddK => format!("R({}) := R({}) + K({})", a, b, c),
                    OpCode::SubK => format!("R({}) := R({}) - K({})", a, b, c),
                    OpCode::MulK => format!("R({}) := R({}) * K({})", a, b, c),
                    OpCode::ModK => format!("R({}) := R({}) % K({})", a, b, c),
                    OpCode::PowK => format!("R({}) := R({}) ^ K({})", a, b, c),
                    OpCode::DivK => format!("R({}) := R({}) / K({})", a, b, c),
                    OpCode::IDivK => format!("R({}) := R({}) // K({})", a, b, c),
                    
                    // Bitwise with constant
                    OpCode::BAndK => format!("R({}) := R({}) & K({})", a, b, c),
                    OpCode::BOrK => format!("R({}) := R({}) | K({})", a, b, c),
                    OpCode::BXorK => format!("R({}) := R({}) ~ K({})", a, b, c),
                    
                    // Shift operations
                    OpCode::ShrI => format!("R({}) := R({}) >> {}", a, b, c),
                    OpCode::ShlI => format!("R({}) := {} << R({})", a, c, b),
                    
                    // Arithmetic operations (register-register)
                    OpCode::Add => format!("R({}) := R({}) + R({})", a, b, c),
                    OpCode::Sub => format!("R({}) := R({}) - R({})", a, b, c),
                    OpCode::Mul => format!("R({}) := R({}) * R({})", a, b, c),
                    OpCode::Mod => format!("R({}) := R({}) % R({})", a, b, c),
                    OpCode::Pow => format!("R({}) := R({}) ^ R({})", a, b, c),
                    OpCode::Div => format!("R({}) := R({}) / R({})", a, b, c),
                    OpCode::IDiv => format!("R({}) := R({}) // R({})", a, b, c),
                    
                    // Bitwise operations (register-register)
                    OpCode::BAnd => format!("R({}) := R({}) & R({})", a, b, c),
                    OpCode::BOr => format!("R({}) := R({}) | R({})", a, b, c),
                    OpCode::BXor => format!("R({}) := R({}) ~ R({})", a, b, c),
                    OpCode::Shl => format!("R({}) := R({}) << R({})", a, b, c),
                    OpCode::Shr => format!("R({}) := R({}) >> R({})", a, b, c),
                    
                    // Metamethod binary operations
                    OpCode::MmBin => format!("call MM[{}] over R({}) and R({})", c, a, b),
                    OpCode::MmBinI => format!("call MM[{}] over R({}) and {}", c, a, b as i32),
                    OpCode::MmBinK => format!("call MM[{}] over R({}) and K({})", c, a, b),
                    
                    // Unary operations
                    OpCode::BNot => format!("R({}) := ~R({})", a, b),
                    OpCode::Not => format!("R({}) := not R({})", a, b),
                    OpCode::Unm => format!("R({}) := -R({})", a, b),
                    OpCode::Len => format!("R({}) := #R({})", a, b),
                    
                    // Concatenation
                    OpCode::Concat => format!("R({}) := R({})..R({})", a, b, c),
                    
                    // Closure and close
                    OpCode::Close => format!("close upvalues >= R({})", a),
                    OpCode::Tbc => format!("mark R({}) as to-be-closed", a),
                    OpCode::Jmp => format!("PC += {} (-> {})", sbx, (pc as i32 + 1 + sbx)),
                    
                    // Comparison operations
                    OpCode::Eq => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) == R({})) ~= {} then PC++", a, b, k as i32)
                    }
                    OpCode::Lt => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) < R({})) ~= {} then PC++", a, b, k as i32)
                    }
                    OpCode::Le => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) <= R({})) ~= {} then PC++", a, b, k as i32)
                    }
                    
                    OpCode::EqK => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) == K({})) ~= {} then PC++", a, b, k as i32)
                    }
                    OpCode::EqI => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) == {}) ~= {} then PC++", a, b as i32, k as i32)
                    }
                    OpCode::LtI => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) < {}) ~= {} then PC++", a, b as i32, k as i32)
                    }
                    OpCode::LeI => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) <= {}) ~= {} then PC++", a, b as i32, k as i32)
                    }
                    OpCode::GtI => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) > {}) ~= {} then PC++", a, b as i32, k as i32)
                    }
                    OpCode::GeI => {
                        let k = Instruction::get_k(instr);
                        format!("if (R({}) >= {}) ~= {} then PC++", a, b as i32, k as i32)
                    }
                    
                    // Test operations
                    OpCode::Test => {
                        let k = Instruction::get_k(instr);
                        format!("if not R({}) ~= {} then PC++", a, k as i32)
                    }
                    OpCode::TestSet => format!("if R({}) then R({}) := R({}) else PC++", b, a, b),
                    
                    // Function call and return
                    OpCode::Call => {
                        if b == 0 {
                            format!("R({})..R(top)(... args)", a)
                        } else {
                            format!("R({})(R({})..R({}))", a, a + 1, a + b - 1)
                        }
                    }
                    OpCode::TailCall => format!("return R({})(R({})..R({}))", a, a + 1, a + b - 1),
                    OpCode::Return => {
                        if b == 0 {
                            format!("return R({})..R(top)", a)
                        } else if b == 1 {
                            format!("return")
                        } else {
                            format!("return R({})..R({})", a, a + b - 2)
                        }
                    }
                    OpCode::Return0 => format!("return"),
                    OpCode::Return1 => format!("return R({})", a),
                    
                    // Loop control
                    OpCode::ForLoop => format!(
                        "R({}) += R({}+2); if R({}) <= R({}+1) then PC += {} (-> {}), R({}+3) := R({})",
                        a, a, a, a, sbx, (pc as i32 + 1 + sbx), a, a
                    ),
                    OpCode::ForPrep => format!(
                        "R({}) -= R({}+2); PC += {} (-> {})",
                        a, a, sbx, (pc as i32 + 1 + sbx)
                    ),
                    OpCode::TForPrep => format!("PC += {} (-> {})", bx, pc + 1 + bx as usize),
                    OpCode::TForCall => format!("R({})..R({}+2) := R({})(R({}+1), R({}+2))", a + 4, a, a, a, a),
                    OpCode::TForLoop => format!("if R({}+4) ~= nil then R({}+2) := R({}+4); PC += {}", a, a, a, sbx),
                    
                    // Varargs
                    OpCode::Vararg => {
                        if c == 0 {
                            format!("R({})..R(top) := ...", a)
                        } else {
                            format!("R({})..R({}) := ...", a, a + c - 2)
                        }
                    }
                    OpCode::VarargPrep => format!("(adjust varargs for {} parameters)", a),
                    
                    // Extra argument
                    OpCode::ExtraArg => format!("extra arg: {}", bx),
                    
                    // Default for any missing opcodes
                    OpCode::SetList => format!("R({})[{}..{}] := R({})..R({})", a, c, c + b - 1, a + 1, a + b),
                    OpCode::Closure => format!("R({}) := closure(PROTO[{}], ...)", a, bx),
                };

                println!(
                    "{:<4} {:<14} {:<6} {:<10} {:<10} 0x{:08X} {}",
                    pc, 
                    format!("{:?}", opcode),
                    a_display,
                    b_display,
                    c_display,
                    instr,
                    details
                );
            }

            if !chunk.constants.is_empty() {
                println!("\nConstants:");
                for (i, val) in chunk.constants.iter().enumerate() {
                    println!("  K({:3}) = {:?}", i, val);
                }
            }

            // Performance hints
            println!("\n=== Performance Analysis ===");
            analyze_performance(&chunk);
        }
        Err(e) => {
            eprintln!("Compilation error: {}", e);
            std::process::exit(1);
        }
    }
}

fn estimate_registers(chunk: &Chunk) -> usize {
    let mut max_reg = 0;
    for &instr in &chunk.code {
        let a = Instruction::get_a(instr) as usize;
        let b = Instruction::get_b(instr) as usize;
        let c = Instruction::get_c(instr) as usize;
        max_reg = max_reg.max(a).max(b).max(c);
    }
    max_reg + 1
}

fn analyze_performance(chunk: &Chunk) {
    let mut move_count = 0;
    let mut load_counts = std::collections::HashMap::new();
    let mut arithmetic_counts = std::collections::HashMap::new();
    let mut table_access_count = 0;
    let mut loop_count = 0;
    let mut jump_count = 0;
    let mut call_count = 0;
    let mut comparison_count = 0;
    let mut bitwise_count = 0;
    let mut upvalue_count = 0;
    let mut closure_count = 0;
    let mut constant_ops = 0;
    let mut immediate_ops = 0;

    for &instr in &chunk.code {
        let opcode = Instruction::get_opcode(instr);
        match opcode {
            // Move operations
            OpCode::Move => move_count += 1,
            
            // Load operations
            OpCode::LoadI => {
                *load_counts.entry("LoadI").or_insert(0) += 1;
                immediate_ops += 1;
            }
            OpCode::LoadF => *load_counts.entry("LoadF").or_insert(0) += 1,
            OpCode::LoadK | OpCode::LoadKX => *load_counts.entry("LoadK").or_insert(0) += 1,
            OpCode::LoadNil => *load_counts.entry("LoadNil").or_insert(0) += 1,
            OpCode::LoadTrue | OpCode::LoadFalse | OpCode::LFalseSkip => {
                *load_counts.entry("LoadBool").or_insert(0) += 1
            }
            
            // Arithmetic operations
            OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div 
            | OpCode::Mod | OpCode::IDiv | OpCode::Pow => {
                *arithmetic_counts.entry("basic").or_insert(0) += 1;
            }
            OpCode::AddI | OpCode::AddK | OpCode::SubK | OpCode::MulK 
            | OpCode::ModK | OpCode::PowK | OpCode::DivK | OpCode::IDivK => {
                *arithmetic_counts.entry("optimized").or_insert(0) += 1;
                if matches!(opcode, OpCode::AddI) {
                    immediate_ops += 1;
                } else {
                    constant_ops += 1;
                }
            }
            OpCode::Unm | OpCode::Len | OpCode::Not => {
                *arithmetic_counts.entry("unary").or_insert(0) += 1;
            }
            
            // Bitwise operations
            OpCode::BAnd | OpCode::BOr | OpCode::BXor | OpCode::Shl | OpCode::Shr | OpCode::BNot
            | OpCode::BAndK | OpCode::BOrK | OpCode::BXorK | OpCode::ShlI | OpCode::ShrI => {
                bitwise_count += 1;
                if matches!(opcode, OpCode::BAndK | OpCode::BOrK | OpCode::BXorK) {
                    constant_ops += 1;
                } else if matches!(opcode, OpCode::ShlI | OpCode::ShrI) {
                    immediate_ops += 1;
                }
            }
            
            // Table operations
            OpCode::GetTable | OpCode::SetTable | OpCode::GetI | OpCode::SetI 
            | OpCode::GetField | OpCode::SetField | OpCode::NewTable | OpCode::SetList
            | OpCode::GetTabUp | OpCode::SetTabUp | OpCode::Self_ => {
                table_access_count += 1;
            }
            
            // Comparison operations
            OpCode::Eq | OpCode::Lt | OpCode::Le | OpCode::EqK | OpCode::EqI 
            | OpCode::LtI | OpCode::LeI | OpCode::GtI | OpCode::GeI => {
                comparison_count += 1;
                if matches!(opcode, OpCode::EqK) {
                    constant_ops += 1;
                } else if matches!(opcode, OpCode::EqI | OpCode::LtI | OpCode::LeI | OpCode::GtI | OpCode::GeI) {
                    immediate_ops += 1;
                }
            }
            
            // Loop control
            OpCode::ForPrep | OpCode::ForLoop | OpCode::TForPrep | OpCode::TForCall | OpCode::TForLoop => {
                loop_count += 1;
            }
            
            // Jumps and tests
            OpCode::Jmp | OpCode::Test | OpCode::TestSet => jump_count += 1,
            
            // Function calls
            OpCode::Call | OpCode::TailCall => call_count += 1,
            
            // Upvalues and closures
            OpCode::GetUpval | OpCode::SetUpval => upvalue_count += 1,
            OpCode::Closure => closure_count += 1,
            
            // Metamethods
            OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK => {
                *arithmetic_counts.entry("metamethod").or_insert(0) += 1;
            }
            
            _ => {}
        }
    }

    let total = chunk.code.len() as f64;
    
    println!("=== Instruction Distribution ===");
    println!("  Total instructions:      {}", chunk.code.len());
    println!();
    
    // Load operations
    println!("  Load Operations:         {} ({:.1}%)", 
        load_counts.values().sum::<i32>(),
        load_counts.values().sum::<i32>() as f64 / total * 100.0
    );
    for (name, count) in load_counts.iter() {
        println!("    - {:<12}       {} ({:.1}%)", name, count, *count as f64 / total * 100.0);
    }
    
    // Move operations
    if move_count > 0 {
        println!("\n  Move instructions:       {} ({:.1}%)", move_count, move_count as f64 / total * 100.0);
    }
    
    // Arithmetic operations
    let arith_total: i32 = arithmetic_counts.values().sum();
    if arith_total > 0 {
        println!("\n  Arithmetic Operations:   {} ({:.1}%)", arith_total, arith_total as f64 / total * 100.0);
        for (name, count) in arithmetic_counts.iter() {
            println!("    - {:<12}       {} ({:.1}%)", name, count, *count as f64 / total * 100.0);
        }
    }
    
    // Table operations
    if table_access_count > 0 {
        println!("\n  Table Access:            {} ({:.1}%)", 
            table_access_count, table_access_count as f64 / total * 100.0);
    }
    
    // Comparisons
    if comparison_count > 0 {
        println!("  Comparison Operations:   {} ({:.1}%)", 
            comparison_count, comparison_count as f64 / total * 100.0);
    }
    
    // Bitwise
    if bitwise_count > 0 {
        println!("  Bitwise Operations:      {} ({:.1}%)", 
            bitwise_count, bitwise_count as f64 / total * 100.0);
    }
    
    // Control flow
    if loop_count > 0 {
        println!("  Loop Control:            {} ({:.1}%)", 
            loop_count, loop_count as f64 / total * 100.0);
    }
    if jump_count > 0 {
        println!("  Jumps/Tests:             {} ({:.1}%)", 
            jump_count, jump_count as f64 / total * 100.0);
    }
    
    // Function calls
    if call_count > 0 {
        println!("  Function Calls:          {} ({:.1}%)", 
            call_count, call_count as f64 / total * 100.0);
    }
    
    // Closures and upvalues
    if upvalue_count > 0 || closure_count > 0 {
        println!("\n  Upvalue Operations:      {}", upvalue_count);
        println!("  Closures:                {}", closure_count);
    }
    
    // Optimization metrics
    println!();
    println!("=== Optimization Metrics ===");
    
    // Constant and immediate operations
    if constant_ops > 0 || immediate_ops > 0 {
        println!("  [+] Constant operations:   {} (K-suffix instructions)", constant_ops);
        println!("  [+] Immediate operations:  {} (I-suffix instructions)", immediate_ops);
        let opt_ratio = (constant_ops + immediate_ops) as f64 / total * 100.0;
        println!("    Optimization ratio:    {:.1}%", opt_ratio);
    }
    
    // Register efficiency
    let max_reg = estimate_registers(chunk);
    println!("  Registers used:          {}", max_reg);
    
    // Move instruction analysis
    let move_ratio = move_count as f64 / total * 100.0;
    if move_ratio > 30.0 {
        println!("\n  [!] High Move ratio ({:.1}%)", move_ratio);
        println!("      -> Consider improving register allocation");
    } else if move_ratio > 15.0 {
        println!("  [!] Moderate Move ratio ({:.1}%)", move_ratio);
    } else if move_count == 0 {
        println!("  [+] Zero Move instructions - excellent register allocation!");
    } else {
        println!("  [+] Low Move ratio ({:.1}%) - good register allocation", move_ratio);
    }
    
    // Loop optimization
    if loop_count > 0 {
        println!("  [+] Optimized loops detected ({} loop instructions)", loop_count);
    }
    
    // Constant table efficiency
    if chunk.constants.len() > 0 {
        println!("\n=== Constant Table ===");
        println!("  Total constants:         {}", chunk.constants.len());
        
        // Check for potential duplicates (naive check)
        if chunk.constants.len() > 50 {
            println!("  [!] Large constant table - ensure deduplication is enabled");
        }
    }
    
    // Code complexity estimate
    println!();
    println!("=== Code Complexity ===");
    let complexity_score = (jump_count as f64 * 2.0 + loop_count as f64 * 3.0 + call_count as f64 * 1.5) / total * 100.0;
    if complexity_score > 50.0 {
        println!("  High complexity:         {:.1} (many branches/calls)", complexity_score);
    } else if complexity_score > 20.0 {
        println!("  Medium complexity:       {:.1}", complexity_score);
    } else {
        println!("  Low complexity:          {:.1} (linear code)", complexity_score);
    }
    
    // Estimated performance characteristics
    println!();
    println!("=== Performance Characteristics ===");
    if constant_ops + immediate_ops > chunk.code.len() / 4 {
        println!("  [+] Good use of constant/immediate optimizations");
    }
    if table_access_count > chunk.code.len() / 3 {
        println!("  [!] Table-heavy code (consider caching)");
    }
    if call_count > chunk.code.len() / 5 {
        println!("  [!] Call-heavy code (function call overhead)");
    }
    if loop_count > 0 && arith_total > (chunk.code.len() / 4) as i32 {
        println!("  [!] Compute-intensive (loops + arithmetic)");
    }
    if upvalue_count > 0 {
        println!("  [i] Uses closures (upvalue access overhead)");
    }
}
