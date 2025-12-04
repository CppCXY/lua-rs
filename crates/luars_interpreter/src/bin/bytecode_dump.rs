use luars::lua_vm::{Instruction, OpCode};
use luars::{Chunk, LuaVM};
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
    println!(
        "{}params: {}, vararg: {}, max_stack: {}",
        indent, chunk.param_count, chunk.is_vararg, chunk.max_stack_size
    );
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
                let k_str = if k { " 1" } else { " 0" };
                format!("TAILCALL {} {}{}", a, b, k_str)
            }
            OpCode::Return => {
                // k=0: show "0k", k=1: show "1" (no k suffix)
                if k {
                    format!("RETURN {} {} 1", a, b)
                } else {
                    format!("RETURN {} {} 0k", a, b)
                }
            }
            OpCode::Return0 => format!("RETURN0"),
            OpCode::Return1 => format!("RETURN1 {}", a),
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
                format!("GetI {} {} {}", a, b, c)
            }
            OpCode::SetI => {
                // SETI A B C/k: R[A][B] := RK(C) - B is unsigned integer index
                let k_str = if k { "k" } else { "" };
                format!("SetI {} {} {}{}", a, b, c, k_str)
            }
            OpCode::EqK => {
                // EQK A B k: if ((R[A] == K[B]) ~= k) then pc++
                let k_str = if k { "k" } else { "" };
                format!("EqK {} {} {}{}", a, b, k as u32, k_str)
            }
            OpCode::SetList => {
                // SETLIST A B C k: for i = 1, B do R[A][C+i] := R[A+i] end
                let k_str = if k { "k" } else { "" };
                format!("SetList {} {} {}{}", a, b, c, k_str)
            }
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

    // Show locals if any
    if !chunk.locals.is_empty() {
        println!("\n{}locals:", indent);
        for (i, name) in chunk.locals.iter().enumerate() {
            println!("{}  {} = {} (register {})", indent, i, name, i);
        }
    }

    // Show upvalues if any
    if chunk.upvalue_count > 0 {
        println!("\n{}upvalues ({}):", indent, chunk.upvalue_count);
        for (i, uv) in chunk.upvalue_descs.iter().enumerate() {
            let uv_type = if uv.is_local { "local" } else { "upvalue" };
            println!("{}  {} = {} index={}", indent, i, uv_type, uv.index);
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
