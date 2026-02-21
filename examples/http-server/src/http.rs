//! Minimal HTTP/1.1 request/response parser.
//!
//! Only supports enough of the HTTP spec to demonstrate the async Lua server:
//! - GET/POST method parsing
//! - Path + query string extraction
//! - Header parsing
//! - Body reading (Content-Length based)
//! - Simple response builder

#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;

// ─── Request ────────────────────────────────────────────────────────────────

/// A parsed HTTP request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl HttpRequest {
    /// Parse a raw HTTP request from bytes.
    ///
    /// Returns `None` if the request is incomplete or malformed.
    pub fn parse(raw: &str) -> Option<Self> {
        let mut lines = raw.split("\r\n");

        // Request line: "GET /path?query HTTP/1.1"
        let request_line = lines.next()?;
        let mut parts = request_line.splitn(3, ' ');
        let method = parts.next()?.to_uppercase();
        let full_path = parts.next()?;
        let _version = parts.next()?; // HTTP/1.x

        // Split path and query string
        let (path, query) = if let Some(idx) = full_path.find('?') {
            (
                full_path[..idx].to_string(),
                Some(full_path[idx + 1..].to_string()),
            )
        } else {
            (full_path.to_string(), None)
        };

        // Parse headers
        let mut headers = HashMap::new();
        let mut header_end = false;
        for line in lines.by_ref() {
            if line.is_empty() {
                header_end = true;
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.trim().to_lowercase(), value.trim().to_string());
            }
        }

        if !header_end {
            return None; // Incomplete headers
        }

        // Collect body (everything after the blank line)
        let body: String = lines.collect::<Vec<_>>().join("\r\n");

        Some(HttpRequest {
            method,
            path,
            query,
            headers,
            body,
        })
    }

    /// Get the Content-Length header value.
    pub fn content_length(&self) -> usize {
        self.headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }
}

// ─── Response ───────────────────────────────────────────────────────────────

/// HTTP status codes we support.
#[derive(Debug, Clone, Copy)]
pub enum StatusCode {
    Ok = 200,
    BadRequest = 400,
    NotFound = 404,
    InternalServerError = 500,
}

impl StatusCode {
    pub fn reason_phrase(self) -> &'static str {
        match self {
            StatusCode::Ok => "OK",
            StatusCode::BadRequest => "Bad Request",
            StatusCode::NotFound => "Not Found",
            StatusCode::InternalServerError => "Internal Server Error",
        }
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", *self as u16, self.reason_phrase())
    }
}

/// A simple HTTP response builder.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpResponse {
    /// Create a 200 OK response with a text body.
    pub fn ok(body: impl Into<String>) -> Self {
        Self {
            status: StatusCode::Ok,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body: body.into(),
        }
    }

    /// Create a JSON response.
    pub fn json(body: impl Into<String>) -> Self {
        Self {
            status: StatusCode::Ok,
            headers: vec![(
                "Content-Type".into(),
                "application/json; charset=utf-8".into(),
            )],
            body: body.into(),
        }
    }

    /// Create a 404 Not Found response.
    pub fn not_found() -> Self {
        Self {
            status: StatusCode::NotFound,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body: "404 Not Found".into(),
        }
    }

    /// Create a 500 Internal Server Error response.
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::InternalServerError,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body: msg.into(),
        }
    }

    /// Add a header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }

    /// Serialize to a complete HTTP/1.1 response string.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut resp = format!("HTTP/1.1 {}\r\n", self.status);
        resp.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        resp.push_str("Connection: close\r\n");
        for (k, v) in &self.headers {
            resp.push_str(&format!("{}: {}\r\n", k, v));
        }
        resp.push_str("\r\n");
        resp.push_str(&self.body);
        resp.into_bytes()
    }
}
