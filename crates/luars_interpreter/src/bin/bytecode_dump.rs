use luars::lua_vm::{Instruction, OpCode};
use luars::{Chunk, LuaVM};
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();

    let (source, filename) = if args.len() > 1 {
        let filename = args[1].clone();
        match fs::read_to_string(&filename) {
            Ok(content) => (content, filename),
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
    
    // Create chunk name with @ prefix like Lua
    let chunk_name = if filename.starts_with('@') {
        filename.clone()
    } else {
        format!("@{}", filename)
    };
    
    match vm.compile_with_name(&source, &chunk_name) {
        Ok(chunk) => {
            dump_chunk(&chunk, &filename, 0, 0, true, &vm);
        }
        Err(_) => {
            let err_msg = vm.get_error_message();
            eprintln!("{}", err_msg);
            std::process::exit(1);
        }
    }
}

/// 格式化常量值为luac格式的字符串（对齐luac的PrintConstant）
fn format_constant(chunk: &Chunk, idx: u32, vm: &LuaVM) -> String {
    if let Some(val) = chunk.constants.get(idx as usize) {
        // 根据值类型格式化
        if val.is_nil() {
            "nil".to_string()
        } else if val.is_boolean() {
            if let Some(b) = val.as_boolean() {
                if b { "true" } else { "false" }.to_string()
            } else {
                "?bool".to_string()
            }
        } else if val.is_integer() {
            if let Some(i) = val.as_integer() {
                i.to_string()
            } else {
                "?int".to_string()
            }
        } else if val.is_float() {
            if let Some(f) = val.as_float() {
                f.to_string()
            } else {
                "?float".to_string()
            }
        } else if val.is_string() {
            // 获取实际字符串内容（对齐luac）
            if let Some(s) = vm.get_string(val) {
                format!("\"{}\"", s.as_str())
            } else {
                format!("string({})", idx)
            }
        } else {
            format!("{:?}", val)
        }
    } else {
        format!("?({})", idx)
    }
}

fn dump_chunk(chunk: &Chunk, filename: &str, linedefined: usize, lastlinedefined: usize, is_main: bool, vm: &LuaVM) {
    // Format: main <file:line,line> or function <file:line,line>
    let func_name = if is_main {
        format!("main <{}:0,0>", filename)
    } else {
        format!("function <{}:{},{}>", filename, linedefined, lastlinedefined)
    };
    
    // Calculate instruction count
    let ninstr = chunk.code.len();
    
    // Format param info (0+ for vararg, or just number)
    let param_str = if chunk.is_vararg {
        format!("{}+", chunk.param_count)
    } else {
        format!("{}", chunk.param_count)
    };
    
    // Print header like luac: name (ninstr instructions)
    println!("\n{} ({} instructions)", func_name, ninstr);
    
    // Print meta info
    println!(
        "{} params, {} slots, {} upvalue{}, {} local{}, {} constant{}, {} function{}",
        param_str,
        chunk.max_stack_size,
        chunk.upvalue_count,
        if chunk.upvalue_count != 1 { "s" } else { "" },
        chunk.locals.len(),
        if chunk.locals.len() != 1 { "s" } else { "" },
        chunk.constants.len(),
        if chunk.constants.len() != 1 { "s" } else { "" },
        chunk.child_protos.len(),
        if chunk.child_protos.len() != 1 { "s" } else { "" }
    );

    for (pc, &instr) in chunk.code.iter().enumerate() {
        let opcode = Instruction::get_opcode(instr);
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr);
        let c = Instruction::get_c(instr);
        let bx = Instruction::get_bx(instr);
        let sbx = Instruction::get_sbx(instr);
        let k = Instruction::get_k(instr);
        
        // Get line number for this instruction (luac format)
        let line = if pc < chunk.line_info.len() {
            chunk.line_info[pc]
        } else {
            0
        };
        
        let detail = match opcode {
            OpCode::VarargPrep => format!("VARARGPREP {}", a),
            OpCode::Vararg => format!("VARARG {} {}", a, b),
            OpCode::Move => format!("MOVE {} {}", a, b),
            OpCode::LoadI => format!("LOADI {} {}", a, sbx),
            OpCode::LoadK => format!("LOADK {} {}", a, bx),
            OpCode::LoadNil => format!("LOADNIL {} {}", a, b),
            OpCode::GetUpval => format!("GETUPVAL {} {}", a, b),
            OpCode::SetUpval => format!("SETUPVAL {} {}", a, b),
            OpCode::GetTabUp => format!("GETTABUP {} {} {}", a, b, c),
            OpCode::SetTabUp => {
                let k_str = if k { "k" } else { "" };
                format!("SETTABUP {} {} {}{}", a, b, c, k_str)
            }
            OpCode::GetField => {
                // GETFIELD never shows k suffix (field name is always from constant table)
                format!("GETFIELD {} {} {}", a, b, c)
            }
            OpCode::SetField => {
                let k_str = if k { "k" } else { "" };
                format!("SETFIELD {} {} {}{}", a, b, c, k_str)
            }
            OpCode::GetTable => format!("GETTABLE {} {} {}", a, b, c),
            OpCode::SetTable => format!("SETTABLE {} {} {}", a, b, c),
            OpCode::NewTable => {
                let k_str = if k { "k" } else { "" };
                format!("NEWTABLE {} {} {}{}", a, b, c, k_str)
            }
            OpCode::Self_ => {
                let k_str = if k { "k" } else { "" };
                format!("SELF {} {} {}{}", a, b, c, k_str)
            }
            OpCode::Add => format!("ADD {} {} {}", a, b, c),
            OpCode::AddI => {
                // ADDI uses signed 8-bit immediate in sC field
                let sc = Instruction::get_sc(instr);
                format!("ADDI {} {} {}", a, b, sc)
            }
            OpCode::AddK => format!("ADDK {} {} {}", a, b, c),
            OpCode::Sub => format!("SUB {} {} {}", a, b, c),
            OpCode::SubK => format!("SUBK {} {} {}", a, b, c),
            OpCode::Mul => format!("MUL {} {} {}", a, b, c),
            OpCode::MulK => format!("MULK {} {} {}", a, b, c),
            OpCode::Div => format!("DIV {} {} {}", a, b, c),
            OpCode::Concat => format!("CONCAT {} {}", a, b),
            OpCode::Call => format!("CALL {} {} {}", a, b, c),
            OpCode::TailCall => {
                // TAILCALL A B C: function at A, B args, C=0 (always 0 for tailcall)
                format!("TAILCALL {} {} {}", a, b, c)
            }
            OpCode::Return => {
                // k=1: show "1k", k=0: show "1" (no k suffix)
                let k_suffix = if k { "k" } else { "" };
                format!("RETURN {} {} {}{}", a, b, c, k_suffix)
            }
            // Return0/Return1 are shown with their own opcode names like official luac
            OpCode::Return0 => format!("RETURN0  \t{}", a),
            OpCode::Return1 => format!("RETURN1  \t{}", a),
            OpCode::Closure => format!("CLOSURE {} {}", a, bx),
            OpCode::Jmp => format!("JMP {}", Instruction::get_sj(instr)),
            OpCode::Eq => format!("EQ {} {} {}", a, b, k as u32),
            OpCode::Lt => format!("LT {} {} {}", a, b, k as u32),
            OpCode::Le => format!("LE {} {} {}", a, b, k as u32),
            OpCode::EqI => {
                // sB field is signed 8-bit integer
                let sb = b as i32 - Instruction::OFFSET_SB;
                format!("EQI {} {} {}", a, sb, k as u32)
            }
            OpCode::LtI => {
                let sb = b as i32 - Instruction::OFFSET_SB;
                format!("LTI {} {} {}", a, sb, k as u32)
            }
            OpCode::LeI => {
                let sb = b as i32 - Instruction::OFFSET_SB;
                format!("LEI {} {} {}", a, sb, k as u32)
            }
            OpCode::GtI => {
                let sb = b as i32 - Instruction::OFFSET_SB;
                format!("GTI {} {} {}", a, sb, k as u32)
            }
            OpCode::GeI => {
                let sb = b as i32 - Instruction::OFFSET_SB;
                format!("GEI {} {} {}", a, sb, k as u32)
            }
            OpCode::ForLoop => {
                // FORLOOP uses unsigned Bx (backward jump distance)
                format!("FORLOOP {} {}", a, bx)
            }
            OpCode::ForPrep => {
                // FORPREP uses unsigned Bx (forward jump distance to skip loop)
                format!("FORPREP {} {}", a, bx)
            }
            OpCode::TForPrep => format!("TFORPREP {} {}", a, bx),
            OpCode::TForLoop => format!("TFORLOOP {} {}", a, bx),
            OpCode::TForCall => {
                // TFORCALL A C: R[A+4], ... ,R[A+3+C] := R[A](R[A+1], R[A+2])
                // Lua 5.4 displays it as "TFORCALL A C" (no B parameter shown)
                format!("TFORCALL {} {}", a, c)
            }
            OpCode::MmBin => {
                // MMBIN only shows 3 parameters (a, b, c) - k flag is not displayed
                format!("MMBIN {} {} {}", a, b, c)
            }
            OpCode::MmBinI => {
                // MMBINI shows 4 parameters, B is signed
                let sb = b as i32 - Instruction::OFFSET_SB;
                format!("MMBINI {} {} {} {}", a, sb, c, k as u32)
            }
            OpCode::MmBinK => {
                // MMBINK shows k flag as 4th parameter
                format!("MMBINK {} {} {} {}", a, b, c, k as u32)
            }
            OpCode::Len => format!("LEN {} {}", a, b),
            OpCode::GetI => {
                // GETI A B C: R[A] := R[B][C] - C is unsigned integer index
                format!("GETI {} {} {}", a, b, c)
            }
            OpCode::SetI => {
                // SETI A B C/k: R[A][B] := RK(C) - B is unsigned integer index
                let k_str = if k { "k" } else { "" };
                format!("SETI {} {} {}{}", a, b, c, k_str)
            }
            OpCode::EqK => {
                // EQK A B k: if ((R[A] == K[B]) ~= k) then pc++
                let k_str = if k { "k" } else { "" };
                format!("EQK {} {} {}{}", a, b, k as u32, k_str)
            }
            OpCode::SetList => {
                // SETLIST A B C k: for i = 1, B do R[A][C+i] := R[A+i] end
                let k_str = if k { "k" } else { "" };
                format!("SETLIST {} {} {}{}", a, b, c, k_str)
            }
            OpCode::ExtraArg => format!("EXTRAARG {}", bx),
            OpCode::Tbc => format!("TBC {}", a),
            OpCode::Close => format!("CLOSE {}", a),
            
            // Bitwise operations
            OpCode::BAnd => format!("BAND {} {} {}", a, b, c),
            OpCode::BOr => format!("BOR {} {} {}", a, b, c),
            OpCode::BXor => format!("BXOR {} {} {}", a, b, c),
            OpCode::Shl => format!("SHL {} {} {}", a, b, c),
            OpCode::Shr => format!("SHR {} {} {}", a, b, c),
            OpCode::BNot => format!("BNOT {} {}", a, b),
            
            // Bitwise with constant
            OpCode::BAndK => format!("BANDK {} {} {}", a, b, c),
            OpCode::BOrK => format!("BORK {} {} {}", a, b, c),
            OpCode::BXorK => format!("BXORK {} {} {}", a, b, c),
            OpCode::ShrI => {
                let sc = Instruction::get_sc(instr);
                format!("SHRI {} {} {}", a, b, sc)
            }
            OpCode::ShlI => {
                let sc = Instruction::get_sc(instr);
                format!("SHLI {} {} {}", a, b, sc)
            }
            
            // Load float/boolean
            OpCode::LoadF => {
                // LOADF loads a float from sBx field
                // The sBx field encodes a float value
                format!("LOADF {} {}", a, sbx)
            }
            OpCode::LoadFalse => format!("LOADFALSE {}", a),
            OpCode::LoadTrue => format!("LOADTRUE {}", a),
            OpCode::LFalseSkip => format!("LFALSESKIP {}", a),
            
            // Test instructions (iAk format)
            OpCode::Test => format!("TEST {} {}", a, k as u32),
            OpCode::TestSet => format!("TESTSET {} {} {}", a, b, k as u32),
            
            _ => format!("{:?} {} {} {}", opcode, a, b, c),
        };
        
        // Add comment for some instructions (like luac)
        let comment = match opcode {
            OpCode::GetTabUp | OpCode::SetTabUp => {
                // Show upvalue name and constant name（对齐luac）
                if b < chunk.upvalue_count as u32 && c < chunk.constants.len() as u32 {
                    format!(" ; _ENV {}", format_constant(chunk, c, vm))
                } else {
                    String::new()
                }
            }
            OpCode::GetUpval => {
                // Show upvalue name
                if b < chunk.upvalue_descs.len() as u32 {
                    String::new() // TODO: add upvalue name when available
                } else {
                    String::new()
                }
            }
            OpCode::Closure => {
                // Show child function address (just use index)
                format!(" ; function_{}", bx)
            }
            OpCode::LoadK => {
                // Show constant value（对齐luac）
                if bx < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, bx, vm))
                } else {
                    String::new()
                }
            }
            OpCode::Return => {
                // Show return count
                let nret = if c == 0 {
                    "0 out"
                } else {
                    &format!("{} out", c - 1)
                };
                format!(" ; {}", nret)
            }
            _ => String::new()
        };

        // Print instruction in luac format: [line] OPCODE args ; comment
        println!("\t{}\t[{}]\t{}{}", pc + 1, line, detail, comment);
    }
    
    // Flush stdout to ensure all output is written
    use std::io::Write;
    std::io::stdout().flush().ok();

    // Recursively dump child protos
    if !chunk.child_protos.is_empty() {
        for (_i, child) in chunk.child_protos.iter().enumerate() {
            // TODO: get actual line numbers from child chunk
            dump_chunk(child, filename, 0, 0, false, vm);
        }
    }
}
