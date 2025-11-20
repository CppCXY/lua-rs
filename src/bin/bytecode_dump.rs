use lua_rs::{Chunk, LuaVM};
use lua_rs::lua_vm::{Instruction, OpCode};
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();

    let source = if args.len() > 1 {
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
            dump_chunk(&chunk, "main", 0);
        }
        Err(e) => {
            eprintln!("Compilation error: {}", e);
            std::process::exit(1);
        }
    }
}

fn dump_chunk(chunk: &Chunk, name: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    
    println!("{}=== {} ===", indent, name);
    println!("{}params: {}, vararg: {}", indent, chunk.param_count, chunk.is_vararg);
    println!();
    
    for (pc, &instr) in chunk.code.iter().enumerate() {
        let opcode = Instruction::get_opcode(instr);
        let a = Instruction::get_a(instr);
        let b = Instruction::get_b(instr);
        let c = Instruction::get_c(instr);
        let bx = Instruction::get_bx(instr);
        let sbx = Instruction::get_sbx(instr);
        let k = Instruction::get_k(instr);
        
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
                let k_str = if k { " k" } else { "" };
                format!("SETTABUP {} {} {}{}", a, b, c, k_str)
            }
            OpCode::GetField => {
                let k_str = if k { " k" } else { "" };
                format!("GETFIELD {} {} {}{}", a, b, c, k_str)
            }
            OpCode::SetField => {
                let k_str = if k { " k" } else { "" };
                format!("SETFIELD {} {} {}{}", a, b, c, k_str)
            }
            OpCode::GetTable => format!("GETTABLE {} {} {}", a, b, c),
            OpCode::SetTable => format!("SETTABLE {} {} {}", a, b, c),
            OpCode::NewTable => format!("NEWTABLE {} {} {}", a, b, c),
            OpCode::Self_ => {
                let k_str = if k { " k" } else { "" };
                format!("SELF {} {} {}{}", a, b, c, k_str)
            }
            OpCode::Add => format!("ADD {} {} {}", a, b, c),
            OpCode::AddI => {
                // ADDI uses signed 8-bit immediate in C field
                // Values 0-127 are positive, 128-255 are negative (128=-128, 255=-1)
                let imm = if c > 127 { (c as i32) - 256 } else { c as i32 };
                format!("ADDI {} {} {}", a, b, imm)
            }
            OpCode::AddK => format!("ADDK {} {} {}", a, b, c),
            OpCode::Sub => format!("SUB {} {} {}", a, b, c),
            OpCode::SubK => format!("SUBK {} {} {}", a, b, c),
            OpCode::Mul => format!("MUL {} {} {}", a, b, c),
            OpCode::MulK => format!("MULK {} {} {}", a, b, c),
            OpCode::Div => format!("DIV {} {} {}", a, b, c),
            OpCode::Concat => format!("CONCAT {} {} {}", a, b, c),
            OpCode::Call => format!("CALL {} {} {}", a, b, c),
            OpCode::TailCall => {
                let k_str = if k { " 1" } else { " 0" };
                format!("TAILCALL {} {}{}", a, b, k_str)
            }
            OpCode::Return => {
                let k_str = if k { " 1" } else { " 0" };
                format!("RETURN {} {}{}", a, b, k_str)
            }
            OpCode::Closure => format!("CLOSURE {} {}", a, bx),
            OpCode::Jmp => format!("JMP {}", Instruction::get_sj(instr)),
            OpCode::Eq => format!("EQ {} {} {}", a, b, k as u32),
            OpCode::Lt => format!("LT {} {} {}", a, b, k as u32),
            OpCode::Le => format!("LE {} {} {}", a, b, k as u32),
            OpCode::EqI => {
                // B field is signed 8-bit integer
                let imm = if b > 127 { (b as i32) - 256 } else { b as i32 };
                format!("EQI {} {} {}", a, imm, k as u32)
            }
            OpCode::LtI => {
                let imm = if b > 127 { (b as i32) - 256 } else { b as i32 };
                format!("LTI {} {} {}", a, imm, k as u32)
            }
            OpCode::LeI => {
                let imm = if b > 127 { (b as i32) - 256 } else { b as i32 };
                format!("LEI {} {} {}", a, imm, k as u32)
            }
            OpCode::GtI => {
                let imm = if b > 127 { (b as i32) - 256 } else { b as i32 };
                format!("GTI {} {} {}", a, imm, k as u32)
            }
            OpCode::GeI => {
                let imm = if b > 127 { (b as i32) - 256 } else { b as i32 };
                format!("GEI {} {} {}", a, imm, k as u32)
            }
            OpCode::ForLoop => format!("FORLOOP {} {}", a, bx),
            OpCode::ForPrep => format!("FORPREP {} {}", a, bx),
            OpCode::TForPrep => format!("TFORPREP {} {}", a, bx),
            OpCode::TForLoop => format!("TFORLOOP {} {}", a, c),
            OpCode::MmBin => format!("MMBIN {} {} {} {}", a, b, c, k as u32),
            OpCode::MmBinI => format!("MMBINI {} {} {} {}", a, b, c, k as u32),
            OpCode::MmBinK => format!("MMBINK {} {} {} {}", a, b, c, k as u32),
            OpCode::ExtraArg => format!("EXTRAARG {}", bx),
            OpCode::Tbc => format!("TBC {}", a),
            OpCode::Close => format!("CLOSE {}", a),
            _ => format!("{:?} {} {} {}", opcode, a, b, c),
        };
        
        println!("{}{:4} {}", indent, pc + 1, detail);
    }
    
    // Show constants if any
    if !chunk.constants.is_empty() {
        println!("\n{}constants:", indent);
        for (i, val) in chunk.constants.iter().enumerate() {
            println!("{}  {} = {:?}", indent, i, val);
        }
    }
    
    // Recursively dump child protos
    if !chunk.child_protos.is_empty() {
        println!();
        for (i, child) in chunk.child_protos.iter().enumerate() {
            dump_chunk(child, &format!("function <PROTO[{}]>", i), depth + 1);
        }
    }
}
