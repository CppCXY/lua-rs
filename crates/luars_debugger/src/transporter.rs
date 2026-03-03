//! TCP transport layer for the EmmyLua debugger protocol.
//!
//! Wire format: `<cmd_number>\n<json_body>\n`
//! On receive, the first line (cmd header) is skipped; dispatch uses `"cmd"` in JSON.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use crate::proto::MessageCMD;

/// Shared TCP connection handle.
pub struct Transporter {
    stream: Arc<Mutex<Option<TcpStream>>>,
}

impl Transporter {
    pub fn new() -> Self {
        Self {
            stream: Arc::new(Mutex::new(None)),
        }
    }

    /// Start listening on the given port. Blocks until one client connects.
    pub fn listen(&self, host: &str, port: u16) -> std::io::Result<()> {
        let addr = format!("{host}:{port}");
        let listener = TcpListener::bind(&addr)?;
        eprintln!("[debugger] listening on {addr}");
        let (stream, peer) = listener.accept()?;
        eprintln!("[debugger] accepted connection from {peer}");
        stream.set_nodelay(true).ok();
        *self.stream.lock().unwrap() = Some(stream);
        Ok(())
    }

    /// Connect to a remote debugger adapter.
    pub fn connect(&self, host: &str, port: u16) -> std::io::Result<()> {
        let addr = format!("{host}:{port}");
        eprintln!("[debugger] connecting to {addr}");
        let stream = TcpStream::connect(&addr)?;
        eprintln!("[debugger] connected to {addr}");
        stream.set_nodelay(true).ok();
        *self.stream.lock().unwrap() = Some(stream);
        Ok(())
    }

    /// Check if a connection is established.
    pub fn is_connected(&self) -> bool {
        self.stream.lock().unwrap().is_some()
    }

    /// Send a message with the given CMD and JSON body.
    pub fn send(&self, cmd: MessageCMD, body: &str) -> std::io::Result<()> {
        let mut guard = self.stream.lock().unwrap();
        let stream = guard.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "not connected")
        })?;
        let msg = format!("{}\n{}\n", cmd as i32, body);
        stream.write_all(msg.as_bytes())?;
        stream.flush()?;
        Ok(())
    }

    /// Receive the next message. Blocks until a complete message is available.
    /// Returns `(cmd_number, json_body)`.
    /// Returns `None` if the connection was closed.
    pub fn receive(&self) -> Option<(i32, String)> {
        // We need to keep a BufReader across calls — but since
        // Transporter is shared via Arc<Mutex<>>, we do a simple approach:
        // clone the TcpStream for reading.
        let cloned = {
            let guard = self.stream.lock().unwrap();
            guard.as_ref()?.try_clone().ok()?
        };
        let mut reader = BufReader::new(cloned);
        loop {
            // Line 1: cmd header
            let mut header = String::new();
            match reader.read_line(&mut header) {
                Ok(0) | Err(_) => return None,
                _ => {}
            }
            let cmd: i32 = header.trim().parse().unwrap_or(0);

            // Line 2: JSON body
            let mut body = String::new();
            match reader.read_line(&mut body) {
                Ok(0) | Err(_) => return None,
                _ => {}
            }
            let body = body
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string();
            return Some((cmd, body));
        }
    }

    /// Receive messages in a loop, calling the handler for each.
    /// Blocks the calling thread until the connection is closed.
    pub fn receive_loop<F>(&self, mut handler: F)
    where
        F: FnMut(i32, &str),
    {
        // Clone the stream once for the read side
        let cloned = {
            let guard = self.stream.lock().unwrap();
            match guard.as_ref().and_then(|s| s.try_clone().ok()) {
                Some(s) => s,
                None => return,
            }
        };
        let mut reader = BufReader::new(cloned);
        let mut header = String::new();
        let mut body = String::new();
        loop {
            header.clear();
            match reader.read_line(&mut header) {
                Ok(0) | Err(_) => break,
                _ => {}
            }
            let _cmd_header: i32 = header.trim().parse().unwrap_or(0);

            body.clear();
            match reader.read_line(&mut body) {
                Ok(0) | Err(_) => break,
                _ => {}
            }
            let json = body.trim_end_matches('\n').trim_end_matches('\r');
            // Dispatch by the "cmd" field in JSON body
            let cmd = extract_cmd_from_json(json);
            handler(cmd, json);
        }
        eprintln!("[debugger] connection closed");
        *self.stream.lock().unwrap() = None;
    }

    /// Disconnect and close the stream.
    pub fn disconnect(&self) {
        *self.stream.lock().unwrap() = None;
    }
}

/// Quick extraction of the `"cmd"` integer field from a JSON string.
fn extract_cmd_from_json(json: &str) -> i32 {
    // Fast path: look for `"cmd":` pattern before full parse
    if let Some(pos) = json.find("\"cmd\"") {
        let rest = &json[pos + 5..];
        // Skip optional whitespace and colon
        let rest = rest.trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            // Parse the integer
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '-')
                .unwrap_or(rest.len());
            return rest[..end].parse().unwrap_or(0);
        }
    }
    0
}
