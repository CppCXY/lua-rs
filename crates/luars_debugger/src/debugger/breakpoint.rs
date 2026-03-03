//! Breakpoint management.

use std::collections::HashMap;

use crate::proto::BreakPointProto;

/// Internal breakpoint with hit counter.
#[derive(Debug, Clone)]
pub struct BreakPoint {
    pub file: String,
    pub line: i32,
    pub condition: String,
    pub hit_condition: String,
    pub log_message: String,
    pub hit_count: i32,
}

impl From<BreakPointProto> for BreakPoint {
    fn from(p: BreakPointProto) -> Self {
        Self {
            file: normalize_path(&p.file),
            line: p.line,
            condition: p.condition,
            hit_condition: p.hit_condition,
            log_message: p.log_message,
            hit_count: 0,
        }
    }
}

/// Stores breakpoints keyed by normalized file path.
#[derive(Debug, Default)]
pub struct BreakPointManager {
    /// file_path → list of breakpoints
    breakpoints: HashMap<String, Vec<BreakPoint>>,
}

impl BreakPointManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all breakpoints.
    pub fn clear(&mut self) {
        self.breakpoints.clear();
    }

    /// Add a breakpoint.
    pub fn add(&mut self, bp: BreakPoint) {
        self.breakpoints
            .entry(bp.file.clone())
            .or_default()
            .push(bp);
    }

    /// Remove a breakpoint by file and line.
    pub fn remove(&mut self, file: &str, line: i32) {
        let key = normalize_path(file);
        if let Some(list) = self.breakpoints.get_mut(&key) {
            list.retain(|b| b.line != line);
            if list.is_empty() {
                self.breakpoints.remove(&key);
            }
        }
    }

    /// Check if there are any breakpoints at all.
    pub fn has_any(&self) -> bool {
        !self.breakpoints.is_empty()
    }

    /// Find a breakpoint matching the given source and line.
    /// Returns a mutable reference so the caller can bump hit_count.
    pub fn find_mut(&mut self, source: &str, line: i32) -> Option<&mut BreakPoint> {
        let key = normalize_path(source);
        self.breakpoints
            .get_mut(&key)
            .and_then(|list| list.iter_mut().find(|bp| bp.line == line))
    }

    /// Find a breakpoint matching the given source and line (immutable).
    pub fn find(&self, source: &str, line: i32) -> Option<&BreakPoint> {
        let key = normalize_path(source);
        self.breakpoints
            .get(&key)
            .and_then(|list| list.iter().find(|bp| bp.line == line))
    }
}

/// Normalize a file path for matching:
/// - lowercase
/// - forward slashes
/// - strip leading `@` (Lua source prefix)
pub fn normalize_path(path: &str) -> String {
    let p = path.strip_prefix('@').unwrap_or(path);
    p.replace('\\', "/").to_lowercase()
}

/// Check if a Lua source name matches a breakpoint file path.
/// Lua sources can be `@path/to/file.lua` (file) or `=stdin` etc.
pub fn source_matches(source: &str, bp_file: &str) -> bool {
    let norm_src = normalize_path(source);
    let norm_bp = normalize_path(bp_file);
    // Exact match
    if norm_src == norm_bp {
        return true;
    }
    // Suffix match (source ends with breakpoint file or vice versa)
    norm_src.ends_with(&norm_bp) || norm_bp.ends_with(&norm_src)
}
