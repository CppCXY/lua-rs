//! Multi-runtime worker pool for the high-level HTTP example.

use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};

use crate::http::{HttpRequest, HttpResponse, StatusCode};
use crate::lua_runtime;

struct WorkerRequest {
    request: HttpRequest,
    respond: oneshot::Sender<HttpResponse>,
}

pub struct VmPool {
    senders: Vec<mpsc::Sender<WorkerRequest>>,
    next: AtomicUsize,
}

impl VmPool {
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

    pub fn worker_count(&self) -> usize {
        self.senders.len()
    }
}

fn worker_main(worker_id: usize, lua_script: String, mut rx: mpsc::Receiver<WorkerRequest>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime for worker");

    rt.block_on(async move {
        let mut runtime = match lua_runtime::create_runtime(&lua_script) {
            Ok(runtime) => runtime,
            Err(e) => {
                eprintln!(
                    "[worker-{}] Failed to create Lua runtime: {:?}",
                    worker_id, e
                );
                return;
            }
        };

        eprintln!("[worker-{}] Ready", worker_id);

        while let Some(msg) = rx.recv().await {
            let request = msg.request;
            let headers_json = headers_to_json(&request.headers);

            let response = match runtime
                .call_handler(
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

            let _ = msg.respond.send(response);
        }

        eprintln!("[worker-{}] Shutting down", worker_id);
    });
}

fn headers_to_json(headers: &std::collections::HashMap<String, String>) -> String {
    let mut json = String::from("{");
    let mut first = true;
    for (k, v) in headers {
        if !first {
            json.push(',');
        }
        first = false;
        json.push('"');
        json.push_str(&k.replace('"', "\\\""));
        json.push_str("\":\"");
        json.push_str(&v.replace('"', "\\\""));
        json.push('"');
    }
    json.push('}');
    json
}
