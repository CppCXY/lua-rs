//! TCP transport layer for the EmmyLua debugger protocol.
//!
//! Wire format (text, newline-delimited):
//!   `<cmd_number>\n<json_body>\n`

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use crate::proto::Message;

/// Shared TCP connection handle.
pub struct Transporter {
    stream: Arc<Mutex<Option<TcpStream>>>,
}

impl Default for Transporter {
    fn default() -> Self {
        Self::new()
    }
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
    /// Wire format: `<cmd_number>\n<json_body>\n`
    pub fn send(&self, message: Message) -> std::io::Result<()> {
        let mut guard = self.stream.lock().unwrap();
        let stream = guard.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "not connected")
        })?;
        let msg_id = message.get_cmd();
        let json = match serde_json::to_string(&message) {
            Ok(json) => json,
            Err(e) => {
                eprintln!("[debugger] failed to serialize message: {e}");
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "failed to serialize message",
                ));
            }
        };
        let msg = format!("{}\n{}\n", msg_id as i32, json);
        stream.write_all(msg.as_bytes())?;
        stream.flush()?;
        Ok(())
    }

    /// Receive messages in a loop, calling the handler for each.
    /// Wire format: `<cmd_number>\n<json_body>\n`
    /// Blocks the calling thread until the connection is closed.
    pub fn receive_loop<F>(&self, mut handler: F)
    where
        F: FnMut(i32, &str),
    {
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
                Ok(0) => break,
                Err(e) => {
                    eprintln!("[debugger] read error: {e}");
                    break;
                }
                _ => {}
            }
            let cmd_header: i32 = header.trim().parse().unwrap_or(0);

            body.clear();
            match reader.read_line(&mut body) {
                Ok(0) => break,
                Err(e) => {
                    eprintln!("[debugger] read error: {e}");
                    break;
                }
                _ => {}
            }
            handler(cmd_header, &body);
        }
        eprintln!("[debugger] connection closed");
        *self.stream.lock().unwrap() = None;
    }

    /// Disconnect and close the stream.
    pub fn disconnect(&self) {
        *self.stream.lock().unwrap() = None;
    }
}
