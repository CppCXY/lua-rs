//! Breakpoint management.

use std::collections::{HashMap, HashSet};

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

impl From<crate::proto::BreakPoint> for BreakPoint {
    fn from(p: crate::proto::BreakPoint) -> Self {
        Self {
            file: normalize_path(&p.file),
            line: p.line,
            condition: p.condition.unwrap_or("".to_string()),
            hit_condition: p.hit_condition.unwrap_or("".to_string()),
            log_message: p.log_message.unwrap_or("".to_string()),
            hit_count: 0,
        }
    }
}

/// Stores breakpoints keyed by normalized file path.
/// Maintains a fast `line_set` for O(1) line-level pre-filtering.
#[derive(Debug, Default)]
pub struct BreakPointManager {
    /// file_path → list of breakpoints
    breakpoints: HashMap<String, Vec<BreakPoint>>,
    /// Set of all breakpoint line numbers across all files.
    /// Used for fast pre-filtering: if the current line is NOT in this set,
    /// we can skip source lookup entirely.
    line_set: HashSet<i32>,
}

impl BreakPointManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild the line_set from all breakpoints.
    fn rebuild_line_set(&mut self) {
        self.line_set.clear();
        for bps in self.breakpoints.values() {
            for bp in bps {
                self.line_set.insert(bp.line);
            }
        }
    }

    /// Clear all breakpoints.
    pub fn clear(&mut self) {
        self.breakpoints.clear();
        self.line_set.clear();
    }

    /// Add a breakpoint.
    pub fn add(&mut self, bp: BreakPoint) {
        self.line_set.insert(bp.line);
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
        self.rebuild_line_set();
    }

    /// Check if there are any breakpoints at all.
    pub fn has_any(&self) -> bool {
        !self.breakpoints.is_empty()
    }

    /// Fast check: does ANY breakpoint exist on this line (any file)?
    /// This is an O(1) lookup used for pre-filtering before the more
    /// expensive source path check.
    #[inline]
    pub fn has_line(&self, line: i32) -> bool {
        self.line_set.contains(&line)
    }

    /// Find a breakpoint matching the given source and line.
    /// `source` should be the raw Lua source name (with or without `@` prefix).
    /// Returns a mutable reference so the caller can bump hit_count.
    pub fn find_mut(&mut self, source: &str, line: i32) -> Option<&mut BreakPoint> {
        let key = normalize_path(source);

        // Single pass: check exact match first, then suffix match
        // We iterate all entries to avoid overlapping mutable borrows.
        let mut exact_found = false;
        let mut suffix_key: Option<String> = None;
        for bp_file in self.breakpoints.keys() {
            if *bp_file == key {
                exact_found = true;
                break;
            }
            if suffix_key.is_none()
                && (key.ends_with(bp_file.as_str()) || bp_file.ends_with(key.as_str()))
            {
                suffix_key = Some(bp_file.clone());
            }
        }

        let match_key = if exact_found { key } else { suffix_key? };

        let list = self.breakpoints.get_mut(&match_key)?;
        list.iter_mut().find(|bp| bp.line == line)
    }

    /// Find a breakpoint matching the given source and line (immutable).
    pub fn find(&self, source: &str, line: i32) -> Option<&BreakPoint> {
        let key = normalize_path(source);
        if let Some(list) = self.breakpoints.get(&key)
            && let Some(bp) = list.iter().find(|bp| bp.line == line)
        {
            return Some(bp);
        }
        for (bp_file, list) in &self.breakpoints {
            if (key.ends_with(bp_file.as_str()) || bp_file.ends_with(key.as_str()))
                && let Some(bp) = list.iter().find(|bp| bp.line == line)
            {
                return Some(bp);
            }
        }
        None
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

/// Strip the leading `@` from a Lua source name for display to IDE.
pub fn strip_at_prefix(source: &str) -> &str {
    source.strip_prefix('@').unwrap_or(source)
}
