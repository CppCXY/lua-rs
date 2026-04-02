//! High-level async HTTP server example.

mod async_io;
mod http;
mod lua_runtime;
mod vm_pool;

use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const DEFAULT_SCRIPT: &str = include_str!("lua/handler.lua");

struct Config {
    port: u16,
    workers: usize,
    script: Option<PathBuf>,
}

impl Config {
    fn from_args() -> Self {
        let mut port = 8080_u16;
        let mut workers = num_cpus();
        let mut script = None;
        let args: Vec<String> = std::env::args().collect();
        let mut index = 1;

        while index < args.len() {
            match args[index].as_str() {
                "--port" | "-p" => {
                    index += 1;
                    if let Some(value) = args.get(index) {
                        port = value.parse().unwrap_or(8080);
                    }
                }
                "--workers" | "-w" => {
                    index += 1;
                    if let Some(value) = args.get(index) {
                        workers = value.parse().unwrap_or_else(|_| num_cpus());
                    }
                }
                "--script" | "-s" => {
                    index += 1;
                    if let Some(value) = args.get(index) {
                        script = Some(PathBuf::from(value));
                    }
                }
                _ => {}
            }
            index += 1;
        }

        Self {
            port,
            workers,
            script,
        }
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_args();
    let script = match &config.script {
        Some(path) => std::fs::read_to_string(path)?,
        None => DEFAULT_SCRIPT.to_owned(),
    };

    let pool = std::sync::Arc::new(vm_pool::VmPool::new(config.workers, &script));
    let listener = TcpListener::bind(("127.0.0.1", config.port)).await?;

    println!(
        "high-level http example listening on http://127.0.0.1:{}",
        config.port
    );
    println!("workers: {}", pool.worker_count());

    loop {
        let (mut stream, peer_addr) = listener.accept().await?;
        let pool = pool.clone();

        tokio::spawn(async move {
            if let Err(error) = handle_connection(&mut stream, &pool).await {
                eprintln!("[{}] connection error: {}", peer_addr, error);
            }
        });
    }
}

async fn handle_connection(
    stream: &mut tokio::net::TcpStream,
    pool: &vm_pool::VmPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = vec![0_u8; 65_536];
    let read = stream.read(&mut buffer).await?;
    if read == 0 {
        return Ok(());
    }

    let raw = String::from_utf8_lossy(&buffer[..read]);
    let response = match http::HttpRequest::parse(&raw) {
        Some(request) => pool.handle(request).await,
        None => http::HttpResponse {
            status: http::StatusCode::BadRequest,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body: "Bad Request: malformed HTTP".into(),
        },
    };

    stream.write_all(&response.to_bytes()).await?;
    Ok(())
}
