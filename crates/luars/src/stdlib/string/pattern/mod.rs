// High-performance Lua pattern matching — byte-oriented, zero-AST design
//
// Modeled after C Lua's lstrlib.c. Operates on `&[u8]` (raw bytes).
// In Lua, all string operations are byte-oriented — each byte is a "character".
//
// 1. NO AST / parse phase — pattern string is interpreted directly during matching
// 2. Fixed-size capture array (32 slots) — no heap allocation during matching
// 3. MatchState struct tracks all state on the stack
// 4. Pattern is `&[char]`, walked with index arithmetic (like C pointers)
// 5. Recursion-limited to prevent stack overflow on pathological patterns

mod class;
mod engine;

pub use engine::{CaptureValue, find, find_all_matches, gsub, is_plain_pattern};
