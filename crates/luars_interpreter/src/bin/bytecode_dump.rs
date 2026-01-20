use luars::lua_vm::{Instruction, OpCode, SafeOption};
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

    let mut vm = LuaVM::new(SafeOption::default());

    // Create chunk name with @ prefix like Lua
    let chunk_name = if filename.starts_with('@') {
        filename.clone()
    } else {
        format!("@{}", filename)
    };

    match vm.compile_with_name(&source, &chunk_name) {
        Ok(chunk) => {
            dump_chunk(
                &chunk,
                &filename,
                chunk.linedefined,
                chunk.lastlinedefined,
                true,
                &vm,
            );
        }
        Err(_) => {
            let err_msg = vm.get_error_message();
            eprintln!("{}", err_msg);
            std::process::exit(1);
        }
    }
}

/// 格式化常量值为luac格式的字符串（对齐luac的PrintConstant）
fn format_constant(chunk: &Chunk, idx: u32, _vm: &LuaVM) -> String {
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
            let content = val.as_str().unwrap_or("");
            // Escape special characters like official luac (including all control characters)
            let mut escaped = String::new();
            for ch in content.chars() {
                match ch {
                    '\\' => escaped.push_str("\\\\"),
                    '\n' => escaped.push_str("\\n"),
                    '\r' => escaped.push_str("\\r"),
                    '\t' => escaped.push_str("\\t"),
                    '"' => escaped.push_str("\\\""),
                    '\0' => escaped.push_str("\\000"),
                    // Escape other control characters as \ddd
                    c if c.is_control() => {
                        escaped.push_str(&format!("\\{:03}", c as u8));
                    }
                    c => escaped.push(c),
                }
            }
            let char_count = escaped.chars().count();
            // 如果字符串超过64个字符，截断并添加 ...
            if char_count > 64 {
                let truncated: String = escaped.chars().take(64).collect();
                format!("\"{} ...\"", truncated)
            } else {
                format!("\"{}\"", escaped)
            }
        } else {
            format!("{:?}", val)
        }
    } else {
        format!("?({})", idx)
    }
}

fn dump_chunk(
    chunk: &Chunk,
    filename: &str,
    linedefined: usize,
    lastlinedefined: usize,
    is_main: bool,
    vm: &LuaVM,
) {
    // Format: main <file:line,line> or function <file:line,line>
    let func_name = if is_main {
        format!("main <{}:0,0>", filename)
    } else {
        format!(
            "function <{}:{},{}>",
            filename, linedefined, lastlinedefined
        )
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
        if chunk.child_protos.len() != 1 {
            "s"
        } else {
            ""
        }
    );

    for (pc, &instr) in chunk.code.iter().enumerate() {
        let opcode = instr.get_opcode();
        let a = instr.get_a();
        let b = instr.get_b();
        let c = instr.get_c();
        let bx = instr.get_bx();
        let ax = instr.get_ax();
        let sbx = instr.get_sbx();
        let k = instr.get_k();
        // For vABCk format instructions, also get vB and vC
        let vb = instr.get_vb();
        let vc = instr.get_vc();
        // Get line number for this instruction (luac format)
        let line = if pc < chunk.line_info.len() {
            chunk.line_info[pc]
        } else {
            0
        };

        let detail = match opcode {
            OpCode::VarargPrep => format!("VARARGPREP {}", a),
            OpCode::Vararg => {
                let k_flag = instr.get_k();
                if k_flag {
                    format!("VARARG {} {} {}k", a, b, c)
                } else {
                    format!("VARARG {} {} {}", a, b, c)
                }
            }
            OpCode::GetVarg => format!("GETVARG {} {} {}", a, b, c),
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
            OpCode::SetTable => {
                let k_str = if k { "k" } else { "" };
                format!("SETTABLE {} {} {}{}", a, b, c, k_str)
            }
            OpCode::NewTable => {
                // NEWTABLE uses vABCk format with vB and vC fields (not B and C)
                // NEWTABLE never shows k flag (per luac.c:430)
                format!("NEWTABLE {} {} {}", a, vb, vc)
            }
            OpCode::Self_ => {
                let k_str = if k { "k" } else { "" };
                format!("SELF {} {} {}{}", a, b, c, k_str)
            }
            OpCode::Add => format!("ADD {} {} {}", a, b, c),
            OpCode::AddI => {
                // ADDI uses signed 8-bit immediate in sC field
                let sc = instr.get_sc();
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
                // TAILCALL A B C k: function at A, B args, k flag for needclose
                let k_suffix = if k { "k" } else { "" };
                format!("TAILCALL {} {} {}{}", a, b, c, k_suffix)
            }
            OpCode::Return => {
                // k=1: show "1k", k=0: show "1" (no k suffix)
                let k_suffix = if k { "k" } else { "" };
                format!("RETURN {} {} {}{}", a, b, c, k_suffix)
            }
            // Return0/Return1 format per luac.c:610-613
            // RETURN0: no operands
            // RETURN1: only A field
            OpCode::Return0 => format!("RETURN0"),
            OpCode::Return1 => format!("RETURN1 {}", a),
            OpCode::Closure => format!("CLOSURE {} {}", a, bx),
            OpCode::Jmp => format!("JMP {}", instr.get_sj()),
            OpCode::Eq => format!("EQ {} {} {}", a, b, k as u32),
            OpCode::Lt => format!("LT {} {} {}", a, b, k as u32),
            OpCode::Le => format!("LE {} {} {}", a, b, k as u32),
            OpCode::EqI => {
                // sB field is signed 8-bit integer
                let sb = b as i32 - Instruction::OFFSET_SC;
                format!("EQI {} {} {}", a, sb, k as u32)
            }
            OpCode::LtI => {
                let sb = b as i32 - Instruction::OFFSET_SC;
                format!("LTI {} {} {}", a, sb, k as u32)
            }
            OpCode::LeI => {
                let sb = b as i32 - Instruction::OFFSET_SC;
                format!("LEI {} {} {}", a, sb, k as u32)
            }
            OpCode::GtI => {
                let sb = b as i32 - Instruction::OFFSET_SC;
                format!("GTI {} {} {}", a, sb, k as u32)
            }
            OpCode::GeI => {
                let sb = b as i32 - Instruction::OFFSET_SC;
                format!("GEI {} {} {}", a, sb, k as u32)
            }
            OpCode::ForLoop => {
                // FORLOOP uses Bx as unsigned distance (no sBx conversion needed)
                format!("FORLOOP {} {}", a, bx)
            }
            OpCode::ForPrep => {
                // FORPREP uses Bx as unsigned distance (no sBx conversion needed)
                format!("FORPREP {} {}", a, bx)
            }
            OpCode::TForPrep => {
                // TFORPREP uses Bx as unsigned distance
                format!("TFORPREP {} {}", a, bx)
            }
            OpCode::TForLoop => {
                // TFORLOOP uses Bx as unsigned distance
                format!("TFORLOOP {} {}", a, bx)
            }
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
                let sb = b as i32 - Instruction::OFFSET_SC;
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
                // Official luac.c:571 shows: printf("%d %d %d",a,b,isk) - no k suffix
                format!("EQK {} {} {}", a, b, k as u32)
            }
            OpCode::SetList => {
                // SETLIST uses vABCk format with vB and vC fields (not B and C)
                // SETLIST A vB vC k: for i = 1, vB do R[A][vC+i] := R[A+i] end
                let k_str = if k { "k" } else { "" };
                format!("SETLIST {} {} {}{}", a, vb, vc, k_str)
            }
            OpCode::ExtraArg => format!("EXTRAARG {}", ax),
            OpCode::Tbc => format!("TBC {}", a),
            OpCode::Close => format!("CLOSE {}", a),

            // Bitwise operations
            OpCode::BAnd => format!("BAND {} {} {}", a, b, c),
            OpCode::BOr => format!("BOR {} {} {}", a, b, c),
            OpCode::BXor => format!("BXOR {} {} {}", a, b, c),
            OpCode::Shl => format!("SHL {} {} {}", a, b, c),
            OpCode::Shr => format!("SHR {} {} {}", a, b, c),

            // Bitwise with constant
            OpCode::BAndK => format!("BANDK {} {} {}", a, b, c),
            OpCode::BOrK => format!("BORK {} {} {}", a, b, c),
            OpCode::BXorK => format!("BXORK {} {} {}", a, b, c),
            OpCode::ShrI => {
                let sc = instr.get_sc();
                format!("SHRI {} {} {}", a, b, sc)
            }
            OpCode::ShlI => {
                let sc = instr.get_sc();
                format!("SHLI {} {} {}", a, b, sc)
            }

            // Unary operations (only A and B parameters)
            OpCode::Unm => format!("UNM {} {}", a, b),
            OpCode::BNot => format!("BNOT {} {}", a, b),
            OpCode::Not => format!("NOT {} {}", a, b),

            // Additional arithmetic operations
            OpCode::Mod => format!("MOD {} {} {}", a, b, c),
            OpCode::Pow => format!("POW {} {} {}", a, b, c),
            OpCode::IDiv => format!("IDIV {} {} {}", a, b, c),
            OpCode::DivK => format!("DIVK {} {} {}", a, b, c),
            OpCode::IDivK => format!("IDIVK {} {} {}", a, b, c),
            OpCode::ModK => format!("MODK {} {} {}", a, b, c),
            OpCode::PowK => format!("POWK {} {} {}", a, b, c),

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

            // Lua 5.5: ERRNNIL instruction (ABx format)
            OpCode::ErrNNil => format!("ERRNNIL {} {}", a, bx),

            _ => format!("{:?} {} {} {}", opcode, a, b, c),
        };

        // Add comment for some instructions (like luac)
        let comment = match opcode {
            OpCode::GetTabUp => {
                // GETTABUP: Show upvalue name and constant name
                if b < chunk.upvalue_count as u32 && c < chunk.constants.len() as u32 {
                    format!(" ; _ENV {}", format_constant(chunk, c, vm))
                } else {
                    String::new()
                }
            }
            OpCode::SetTabUp => {
                // SETTABUP: Show upvalue name, key constant, and value constant (if k=1)
                let mut comment = String::new();
                if b < chunk.upvalue_count as u32 {
                    comment.push_str(" ; _ENV");
                    if b < chunk.constants.len() as u32 {
                        comment.push_str(&format!(" {}", format_constant(chunk, b, vm)));
                    }
                    // If k flag is set, show value constant
                    if k && c < chunk.constants.len() as u32 {
                        comment.push_str(&format!(" {}", format_constant(chunk, c, vm)));
                    }
                }
                comment
            }
            OpCode::GetField => {
                // GETFIELD A B C: table in B, field name in C
                if c < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, c, vm))
                } else {
                    String::new()
                }
            }
            OpCode::SetField => {
                // SETFIELD A B C: table in A, field name in B, value in C
                // Show field name constant and value constant (if k=1)
                let mut comment = String::new();
                if b < chunk.constants.len() as u32 {
                    comment.push_str(&format!(" ; {}", format_constant(chunk, b, vm)));
                    // If k flag is set, show value constant
                    if k && c < chunk.constants.len() as u32 {
                        comment.push_str(&format!(" {}", format_constant(chunk, c, vm)));
                    }
                }
                comment
            }
            OpCode::SetTable => {
                // SETTABLE A B C: table in A, key in B, value in C
                // If k flag is set, show value constant
                if k && c < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, c, vm))
                } else {
                    String::new()
                }
            }
            OpCode::SetI => {
                // SETI A B C: table in A, index in B, value in C
                // If k flag is set, show value constant
                if k && c < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, c, vm))
                } else {
                    String::new()
                }
            }
            OpCode::Self_ => {
                // SELF A B C: R[A+1]=R[B], R[A]=R[B][RK(C)]
                // If k flag is set, show method name constant
                if k && c < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, c, vm))
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
            // All K-suffix arithmetic operations show constant value
            OpCode::AddK
            | OpCode::SubK
            | OpCode::MulK
            | OpCode::ModK
            | OpCode::PowK
            | OpCode::DivK
            | OpCode::IDivK => {
                if c < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, c, vm))
                } else {
                    String::new()
                }
            }
            // All K-suffix bitwise operations show constant value
            OpCode::BAndK | OpCode::BOrK | OpCode::BXorK => {
                if c < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, c, vm))
                } else {
                    String::new()
                }
            }
            OpCode::EqK => {
                // EQK A B k: show constant value
                if b < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, b, vm))
                } else {
                    String::new()
                }
            }
            OpCode::MmBinK => {
                // MMBINK: show event name and constant value
                let event_name = match c {
                    6 => "__add",
                    7 => "__sub",
                    8 => "__mul",
                    9 => "__mod",
                    10 => "__pow",
                    11 => "__div",
                    12 => "__idiv",
                    13 => "__band",
                    14 => "__bor",
                    15 => "__bxor",
                    16 => "__shl",
                    17 => "__shr",
                    _ => "",
                };
                let mut comment = format!(" ; {}", event_name);
                if b < chunk.constants.len() as u32 {
                    comment.push_str(&format!(" {}", format_constant(chunk, b, vm)));
                }
                if k {
                    comment.push_str(" flip");
                }
                comment
            }
            OpCode::MmBinI => {
                // MMBINI: show event name
                let event_name = match c {
                    6 => "__add",
                    7 => "__sub",
                    8 => "__mul",
                    9 => "__mod",
                    10 => "__pow",
                    11 => "__div",
                    12 => "__idiv",
                    13 => "__band",
                    14 => "__bor",
                    15 => "__bxor",
                    16 => "__shl",
                    17 => "__shr",
                    _ => "",
                };
                if k {
                    format!(" ; {} flip", event_name)
                } else {
                    format!(" ; {}", event_name)
                }
            }
            OpCode::MmBin => {
                // MMBIN: show event name
                let event_name = match c {
                    6 => "__add",
                    7 => "__sub",
                    8 => "__mul",
                    9 => "__mod",
                    10 => "__pow",
                    11 => "__div",
                    12 => "__idiv",
                    13 => "__band",
                    14 => "__bor",
                    15 => "__bxor",
                    16 => "__shl",
                    17 => "__shr",
                    _ => "",
                };
                format!(" ; {}", event_name)
            }
            OpCode::NewTable => {
                // NEWTABLE: show array size (vc + EXTRAARGC)
                // For now just show vc since we don't parse EXTRAARG here
                format!(" ; {}", vc)
            }
            OpCode::Closure => {
                // Show child function address - luac shows actual pointer
                // We'll show a stable identifier based on bx
                if bx < chunk.child_protos.len() as u32 {
                    // Use child proto's address-like format
                    format!(" ; {:p}", &chunk.child_protos[bx as usize] as *const _)
                } else {
                    String::new()
                }
            }
            OpCode::LoadK => {
                // Show constant value（对齐luac）
                if bx < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, bx, vm))
                } else {
                    String::new()
                }
            }
            OpCode::ErrNNil => {
                // ERRNNIL A Bx: check R[A] ~= nil, Bx-1 is index of global name
                // Bx = k + 1, so k = Bx - 1
                if bx > 0 && (bx - 1) < chunk.constants.len() as u32 {
                    format!(" ; {}", format_constant(chunk, bx - 1, vm))
                } else {
                    String::new()
                }
            }
            OpCode::LoadNil => {
                // Show number of nils loaded
                let count = b + 1;
                format!(" ; {} out", count)
            }
            OpCode::Call | OpCode::TailCall => {
                // Show parameter and return counts
                // B = num params + 1 (or 0 for all in)
                // C = num returns + 1 (or 0 for all out)
                let params = if b == 0 {
                    "all in"
                } else {
                    &format!("{} in", b - 1)
                };
                let returns = if c == 0 {
                    "all out"
                } else {
                    &format!("{} out", c - 1)
                };
                format!(" ; {} {}", params, returns)
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
            OpCode::Jmp => {
                // Show jump target: "to X" where X is the target instruction (1-based)
                let sj = instr.get_sj();
                let target = (pc as isize + sj as isize + 1) as usize + 1; // +1 for 1-based indexing
                format!(" ; to {}", target)
            }
            OpCode::ForPrep => {
                // Show exit target: "exit to X" where X is instruction after exit
                // VM executes: pc += Bx + 1, then continues at pc (which becomes pc+1 in next iteration)
                // So target = current_pc + Bx + 1 + 1 (one for VM jump, one for next instruction)
                let target = pc + 1 + bx as usize + 1 + 1; // pc is 0-based, +1 for 1-based, +Bx+1 for jump, +1 for next instr
                format!(" ; exit to {}", target)
            }
            OpCode::ForLoop => {
                // Show loop target: "to X" where X is the loop body start
                // VM executes: pc -= Bx, then continues at pc+1 in next iteration
                // target = current_pc - Bx + 1 (for next instruction after VM decrements pc)
                let target = pc + 1 - bx as usize + 1; // pc is 0-based, +1 for 1-based, -Bx for backward, +1 for next
                format!(" ; to {}", target)
            }
            OpCode::TForPrep => {
                // Show target after iterator setup
                // VM executes: pc += Bx, then continues at pc
                let target = pc + 1 + bx as usize + 1; // +1 for 1-based, +Bx for jump, +1 for next instr
                format!(" ; to {}", target)
            }
            OpCode::TForLoop => {
                // Show loop target
                // VM executes: pc -= Bx, then continues at pc
                let target = pc + 1 - bx as usize; // +1 for 1-based, -Bx for backward jump
                format!(" ; to {}", target)
            }
            OpCode::Vararg => {
                // VARARG: show return count
                if c == 0 {
                    format!(" ; all out")
                } else {
                    format!(" ; {} out", c - 1)
                }
            }
            OpCode::GetVarg => {
                // GETVARG: access named vararg parameter
                // A = destination register, B = vararg param register, C = key register
                format!("")
            }
            _ => String::new(),
        };

        // Print instruction in luac format: [line] OPCODE args ; comment
        // Split detail into opcode name and arguments for proper formatting
        // Official luac uses: printf("%-9s\t",opnames[o]) for opcode, then args
        let parts: Vec<&str> = detail.splitn(2, ' ').collect();
        let opcode_name = parts[0];
        let args = if parts.len() > 1 { parts[1] } else { "" };

        println!(
            "\t{}\t[{}]\t{:<9}\t{}{}",
            pc + 1,
            line,
            opcode_name,
            args,
            comment
        );
    }

    // Print constants list (for debugging)
    if !chunk.constants.is_empty() {
        println!("constants ({}) for {}:", chunk.constants.len(), func_name);
        for (idx, _val) in chunk.constants.iter().enumerate() {
            println!("\t{}\t{}", idx, format_constant(chunk, idx as u32, vm));
        }
    }

    // Flush stdout to ensure all output is written
    use std::io::Write;
    std::io::stdout().flush().ok();

    // Recursively dump child protos
    if !chunk.child_protos.is_empty() {
        for (_i, child) in chunk.child_protos.iter().enumerate() {
            dump_chunk(
                child,
                filename,
                child.linedefined,
                child.lastlinedefined,
                false,
                vm,
            );
        }
    }
    println!("") // for debug
}
