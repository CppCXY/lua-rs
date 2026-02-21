//! Multi-VM worker pool.
//!
//! Since `LuaVM` is `!Send` (contains raw pointers and `Rc`), we cannot move
//! VMs across threads. Instead, we use a **thread-per-VM** model:
//!
//! - N worker threads are spawned, each owning its own `LuaVM` and a
//!   single-threaded tokio runtime (`current_thread`).
//! - Incoming requests are dispatched round-robin to workers via channels.
//! - Each worker processes requests sequentially on its own VM, but async
//!   functions (sleep, read_file, etc.) are truly async within that worker.
//!
//! This architecture lets us:
//! 1. Isolate VMs (one crash doesn't affect others)
//! 2. Use multiple CPU cores (one VM per thread)
//! 3. Support async I/O within each VM

use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};

use crate::http::{HttpRequest, HttpResponse, StatusCode};
use crate::lua_runtime;

/// A request sent to a worker thread.
struct WorkerRequest {
    request: HttpRequest,
    respond: oneshot::Sender<HttpResponse>,
}

/// Handle to the VM pool — used by the main server loop to dispatch requests.
pub struct VmPool {
    senders: Vec<mpsc::Sender<WorkerRequest>>,
    next: AtomicUsize,
}

impl VmPool {
    /// Create a pool of `n` worker threads, each with its own LuaVM.
    ///
    /// `lua_script` is loaded into every VM (the handler script).
    pub fn new(n: usize, lua_script: &str) -> Self {
        assert!(n > 0, "VmPool requires at least 1 worker");
        let mut senders = Vec::with_capacity(n);

        for worker_id in 0..n {
            let (tx, rx) = mpsc::channel::<WorkerRequest>(64);
            let script = lua_script.to_string();

            std::thread::Builder::new()
                .name(format!("lua-worker-{}", worker_id))
                .spawn(move || {
                    worker_main(worker_id, script, rx);
                })
                .expect("failed to spawn worker thread");

            senders.push(tx);
        }

        VmPool {
            senders,
            next: AtomicUsize::new(0),
        }
    }

    /// Dispatch a request to the next worker (round-robin).
    ///
    /// Returns the response asynchronously.
    pub async fn handle(&self, request: HttpRequest) -> HttpResponse {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        let (tx, rx) = oneshot::channel();

        let msg = WorkerRequest {
            request,
            respond: tx,
        };

        if self.senders[idx].send(msg).await.is_err() {
            return HttpResponse::error("Worker thread has died");
        }

        match rx.await {
            Ok(resp) => resp,
            Err(_) => HttpResponse::error("Worker dropped response channel"),
        }
    }

    /// Number of workers.
    pub fn worker_count(&self) -> usize {
        self.senders.len()
    }
}

/// Entry point for each worker thread.
///
/// Creates a single-threaded tokio runtime and a LuaVM, then processes
/// requests from the channel until it's closed.
fn worker_main(worker_id: usize, lua_script: String, mut rx: mpsc::Receiver<WorkerRequest>) {
    // Build a single-threaded tokio runtime for this worker
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime for worker");

    rt.block_on(async move {
        // Create the LuaVM for this worker
        let mut vm = match lua_runtime::create_vm(&lua_script) {
            Ok(vm) => vm,
            Err(e) => {
                eprintln!("[worker-{}] Failed to create LuaVM: {:?}", worker_id, e);
                return;
            }
        };

        eprintln!("[worker-{}] Ready", worker_id);

        // Process requests
        while let Some(msg) = rx.recv().await {
            let request = msg.request;

            // Build a simple JSON-ish representation of headers for Lua
            let headers_json = headers_to_json(&request.headers);

            let response = match lua_runtime::call_handler(
                &mut vm,
                &request.method,
                &request.path,
                request.query.as_deref(),
                &headers_json,
                &request.body,
            )
            .await
            {
                Ok((status, content_type, body)) => {
                    let status_code = match status {
                        200 => StatusCode::Ok,
                        400 => StatusCode::BadRequest,
                        404 => StatusCode::NotFound,
                        _ => StatusCode::InternalServerError,
                    };
                    HttpResponse {
                        status: status_code,
                        headers: vec![("Content-Type".into(), content_type)],
                        body,
                    }
                }
                Err(e) => {
                    eprintln!("[worker-{}] Lua error: {:?}", worker_id, e);
                    HttpResponse::error(format!("Lua error: {:?}", e))
                }
            };

            // Send response back (ignore error if receiver dropped)
            let _ = msg.respond.send(response);
        }

        eprintln!("[worker-{}] Shutting down", worker_id);
    });
}

/// Convert headers HashMap to a simple JSON string for Lua consumption.
fn headers_to_json(headers: &std::collections::HashMap<String, String>) -> String {
    let mut json = String::from("{");
    let mut first = true;
    for (k, v) in headers {
        if !first {
            json.push(',');
        }
        first = false;
        // Simple escaping — sufficient for header values
        json.push('"');
        json.push_str(&k.replace('"', "\\\""));
        json.push_str("\":\"");
        json.push_str(&v.replace('"', "\\\""));
        json.push('"');
    }
    json.push('}');
    json
}
