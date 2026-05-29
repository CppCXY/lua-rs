mod emit;
mod parser;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

/// Generate EmmyLua / LuaLS API documentation from luars userdata Rust types.
///
/// Parses `.rs` files that use `#[derive(LuaUserData)]` and `#[lua_methods]`
/// and emits Lua annotation stubs suitable for Lua Language Server type checking.
#[derive(Parser)]
#[command(name = "luars-doc", version)]
struct Cli {
    /// One or more `.rs` files to process.
    #[arg(short, long = "file", value_name = "FILE")]
    files: Vec<PathBuf>,

    /// Recursively scan a directory for `.rs` files.
    #[arg(short, long = "dir", value_name = "DIR")]
    dirs: Vec<PathBuf>,

    /// Output file (stdout if omitted).
    #[arg(short, long = "out", value_name = "FILE")]
    output: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut bundle = emit::DocBundle::default();

    // Process individual files
    if !cli.files.is_empty() {
        let file_bundle = parser::parse_files(&cli.files)?;
        bundle.types.extend(file_bundle.types);
    }

    // Process directories
    for dir in &cli.dirs {
        let dir_bundle = parser::parse_dir(dir)?;
        bundle.types.extend(dir_bundle.types);
    }

    // If nothing specified, show help
    if cli.files.is_empty() && cli.dirs.is_empty() {
        eprintln!("luars-doc: no input files specified. Use --file or --dir.");
        return Ok(());
    }

    let output = emit::emit_lua(&bundle);

    match cli.output {
        Some(path) => {
            let len = output.len();
            std::fs::write(&path, &output)?;
            eprintln!("Wrote {} bytes to {}", len, path.display());
        }
        None => {
            print!("{}", output);
        }
    }

    Ok(())
}
