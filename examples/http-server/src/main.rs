//! luars async HTTP server — demonstrates the luars async feature with a
//! multi-VM architecture.
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                  Main Thread (tokio multi-thread)           │
//! │  TcpListener.accept() → parse HTTP → dispatch to pool      │
//! └─────────────┬──────────────┬──────────────┬────────────────┘
//!               │              │              │
//!         ┌─────▼─────┐ ┌─────▼─────┐ ┌─────▼─────┐
//!         │ Worker 0   │ │ Worker 1   │ │ Worker N   │
//!         │ LuaVM      │ │ LuaVM      │ │ LuaVM      │
//!         │ tokio (ST)  │ │ tokio (ST)  │ │ tokio (ST)  │
//!         └────────────┘ └────────────┘ └────────────┘
//! ```
//!
//! Usage:
//!   cargo run -p http-server -- [--port PORT] [--workers N] [--script PATH]

mod async_io;
mod http;
mod lua_runtime;
mod vm_pool;

use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Default Lua handler script (embedded).
const DEFAULT_SCRIPT: &str = include_str!("lua/handler.lua");

/// Command-line configuration.
struct Config {
    port: u16,
    workers: usize,
    script: Option<PathBuf>,
}

impl Config {
    fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let mut port = 8080u16;
        let mut workers = num_cpus();
        let mut script = None;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--port" | "-p" => {
                    i += 1;
                    if let Some(val) = args.get(i) {
                        port = val.parse().unwrap_or(8080);
                    }
                }
                "--workers" | "-w" => {
                    i += 1;
                    if let Some(val) = args.get(i) {
                        workers = val.parse().unwrap_or_else(|_| num_cpus());
                    }
                }
                "--script" | "-s" => {
                    i += 1;
                    if let Some(val) = args.get(i) {
                        script = Some(PathBuf::from(val));
                    }
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => {
                    eprintln!("Unknown argument: {}", other);
                    print_help();
                    std::process::exit(1);
                }
            }
            i += 1;
        }

        Config {
            port,
            workers,
            script,
        }
    }
}

fn print_help() {
    eprintln!(
        r#"luars async HTTP server

Usage: http-server [OPTIONS]

Options:
  -p, --port <PORT>        Listen port (default: 8080)
  -w, --workers <N>        Number of Lua VM workers (default: num CPUs)
  -s, --script <PATH>      Path to Lua handler script (default: built-in)
  -h, --help               Show this help
"#
    );
}

/// Get the number of available CPUs (fallback to 2).
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
}

#[tokio::main]
async fn main() {
    let config = Config::from_args();

    // Load the Lua script
    let lua_script = if let Some(ref path) = config.script {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to read script {:?}: {}", path, e);
                std::process::exit(1);
            }
        }
    } else {
        DEFAULT_SCRIPT.to_string()
    };

    eprintln!("=== luars async HTTP server ===");
    eprintln!("Port:    {}", config.port);
    eprintln!("Workers: {}", config.workers);
    if let Some(ref path) = config.script {
        eprintln!("Script:  {}", path.display());
    } else {
        eprintln!("Script:  (built-in handler.lua)");
    }
    eprintln!();

    // Create the VM pool
    let pool = std::sync::Arc::new(vm_pool::VmPool::new(config.workers, &lua_script));
    eprintln!("VM pool ready: {} workers", pool.worker_count());

    // Bind TCP listener
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        eprintln!("Failed to bind {}: {}", addr, e);
        std::process::exit(1);
    });
    eprintln!("Listening on http://localhost:{}", config.port);
    eprintln!();

    // Accept loop
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Accept error: {}", e);
                continue;
            }
        };

        let pool = pool.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer_addr, &pool).await {
                eprintln!("[{}] Connection error: {}", peer_addr, e);
            }
        });
    }
}

/// Handle a single TCP connection: read request, dispatch to VM pool, send response.
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    pool: &vm_pool::VmPool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Read request (up to 64KB)
    let mut buf = vec![0u8; 65536];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let raw = String::from_utf8_lossy(&buf[..n]);

    // Parse HTTP request
    let request = match http::HttpRequest::parse(&raw) {
        Some(req) => req,
        None => {
            let resp = http::HttpResponse {
                status: http::StatusCode::BadRequest,
                headers: vec![("Content-Type".into(), "text/plain".into())],
                body: "Bad Request: malformed HTTP".into(),
            };
            stream.write_all(&resp.to_bytes()).await?;
            return Ok(());
        }
    };

    eprintln!("[{}] {} {}", peer_addr, request.method, request.path);

    // Dispatch to VM pool
    let response = pool.handle(request).await;

    // Send response
    stream.write_all(&response.to_bytes()).await?;

    Ok(())
}
