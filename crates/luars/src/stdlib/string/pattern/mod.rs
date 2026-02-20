// High-performance Lua pattern matching — char-based, zero-AST design
//
// Modeled after C Lua's lstrlib.c but operating on `&[char]` for full UTF-8.
// Key design differences from the old `pattern/` module:
//
// 1. NO AST / parse phase — pattern string is interpreted directly during matching
// 2. Fixed-size capture array (32 slots) — no heap allocation during matching
// 3. MatchState struct tracks all state on the stack
// 4. Pattern is `&[char]`, walked with index arithmetic (like C pointers)
// 5. Recursion-limited to prevent stack overflow on pathological patterns
//
// The public API mirrors the old module for drop-in replacement.

mod class;
mod engine;

pub use engine::{CaptureValue, MatchInfo, find, find_all_matches, gsub, match_pattern};
