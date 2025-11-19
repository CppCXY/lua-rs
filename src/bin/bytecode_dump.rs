use lua_rs::Chunk;
use lua_rs::LuaVM;
use lua_rs::lua_vm::{Instruction, OpCode};
use std::env;
use std::fs;

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
            println!("\nInstructions:");
            println!("{:<4} {:<12} {:<40} {}", "PC", "OpCode", "Details", "Raw");
            println!("{}", "-".repeat(80));

            for (pc, &instr) in chunk.code.iter().enumerate() {
                let opcode = Instruction::get_opcode(instr);
                let a = Instruction::get_a(instr);
                let b = Instruction::get_b(instr);
                let c = Instruction::get_c(instr);
                let bx = Instruction::get_bx(instr);
                let sbx = Instruction::get_sbx(instr);

                let details = match opcode {
                    OpCode::Move => format!("R({}) := R({})", a, b),
                    OpCode::LoadK => format!("R({}) := K({})", a, bx),
                    OpCode::LoadI => format!("R({}) := {}", a, sbx), // Load immediate integer
                    OpCode::LoadNil => format!("R({}) := nil", a),
                    OpCode::LoadBool => format!("R({}) := {}", a, b != 0),
                    OpCode::Add => format!("R({}) := R({}) + R({})", a, b, c),
                    OpCode::Sub => format!("R({}) := R({}) - R({})", a, b, c),
                    OpCode::Mul => format!("R({}) := R({}) * R({})", a, b, c),
                    OpCode::Div => format!("R({}) := R({}) / R({})", a, b, c),
                    OpCode::Mod => format!("R({}) := R({}) % R({})", a, b, c),
                    OpCode::IDiv => format!("R({}) := R({}) // R({})", a, b, c),
                    OpCode::Pow => format!("R({}) := R({}) ^ R({})", a, b, c),
                    OpCode::Unm => format!("R({}) := -R({})", a, b),
                    OpCode::Not => format!("R({}) := not R({})", a, b),
                    OpCode::Len => format!("R({}) := #R({})", a, b),
                    OpCode::Concat => format!("R({}) := R({})..R({})", a, b, c),
                    OpCode::Eq => format!("R({}) := R({}) == R({})", a, b, c),
                    OpCode::Lt => format!("R({}) := R({}) < R({})", a, b, c),
                    OpCode::Le => format!("R({}) := R({}) <= R({})", a, b, c),
                    OpCode::Ne => format!("R({}) := R({}) ~= R({})", a, b, c),
                    OpCode::Gt => format!("R({}) := R({}) > R({})", a, b, c),
                    OpCode::Ge => format!("R({}) := R({}) >= R({})", a, b, c),
                    OpCode::And => format!("R({}) := R({}) and R({})", a, b, c),
                    OpCode::Or => format!("R({}) := R({}) or R({})", a, b, c),
                    OpCode::Jmp => format!("PC += {} (-> {})", sbx, (pc as i32 + 1 + sbx)),
                    OpCode::Test => format!("if not R({}) then PC++", a),
                    OpCode::ForPrep => format!(
                        "R({}) -= R({}+2); PC += {} (-> {})",
                        a,
                        a,
                        sbx,
                        (pc as i32 + 1 + sbx)
                    ),
                    OpCode::ForLoop => format!(
                        "R({}) += R({}+2); if loop then PC += {} (-> {})",
                        a,
                        a,
                        sbx,
                        (pc as i32 + 1 + sbx)
                    ),
                    OpCode::Call => format!("R({})(R({})..R({}))", a, a + 1, a + b - 1),
                    OpCode::Return => format!("return R({})", a),
                    OpCode::GetTable => format!("R({}) := R({})[R({})]", a, b, c),
                    OpCode::SetTable => format!("R({})[R({})] := R({})", a, b, c),
                    OpCode::GetGlobal => format!("R({}) := _G[K({})]", a, bx),
                    OpCode::SetGlobal => format!("_G[K({})] := R({})", bx, a),
                    OpCode::GetUpval => format!("R({}) := UpValue[{}]", a, b),
                    OpCode::SetUpval => format!("UpValue[{}] := R({})", b, a),
                    OpCode::Closure => format!("R({}) := closure(PROTO[{}])", a, bx),
                    _ => format!("A:{} B:{} C:{} Bx:{}", a, b, c, bx),
                };

                println!("{:<4} {:<12?} {:<40} 0x{:08x}", pc, opcode, details, instr);
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
    let mut loadk_count = 0;
    let mut loadi_count = 0;
    let mut arithmetic_count = 0;
    let mut loop_count = 0;
    let mut jump_count = 0;

    for &instr in &chunk.code {
        match Instruction::get_opcode(instr) {
            OpCode::Move => move_count += 1,
            OpCode::LoadK => loadk_count += 1,
            OpCode::LoadI => loadi_count += 1,
            OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Div
            | OpCode::Mod
            | OpCode::IDiv
            | OpCode::Pow => arithmetic_count += 1,
            OpCode::ForPrep | OpCode::ForLoop => loop_count += 1,
            OpCode::Jmp => jump_count += 1,
            _ => {}
        }
    }

    println!("Instruction Statistics:");
    println!(
        "  Move instructions:       {} ({:.1}%)",
        move_count,
        move_count as f64 / chunk.code.len() as f64 * 100.0
    );
    println!(
        "  LoadK instructions:      {} ({:.1}%)",
        loadk_count,
        loadk_count as f64 / chunk.code.len() as f64 * 100.0
    );
    println!(
        "  LoadI instructions:      {} ({:.1}%)",
        loadi_count,
        loadi_count as f64 / chunk.code.len() as f64 * 100.0
    );
    println!("  Arithmetic operations:   {}", arithmetic_count);
    println!("  Loop control:            {}", loop_count);
    println!("  Jumps:                   {}", jump_count);

    if move_count > chunk.code.len() / 5 {
        println!("\n⚠️  Warning: High percentage of Move instructions detected!");
        println!("   Consider optimizing register allocation in the compiler.");
    }

    if loop_count > 0 {
        println!("\n✓ Optimized for-loops detected (ForPrep/ForLoop)");
    }

    if loadi_count > 0 {
        println!(
            "\n✓ LoadI optimization enabled ({} immediate loads)",
            loadi_count
        );
    }
}
