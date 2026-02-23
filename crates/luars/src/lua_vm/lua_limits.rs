//! Centralized Lua VM limits and configuration constants.
//!
//! Mirrors Lua 5.5's `luaconf.h` / `llimits.h` design.
//! All magic numbers that control VM behavior are collected here
//! for easy tuning and configuration.

// ===== Stack =====

/// Extra stack slots above frame_top for C function calls, temporaries, etc.
/// Matches Lua 5.5's EXTRA_STACK (5).
pub const EXTRA_STACK: usize = 5;

/// Initial stack capacity for new Lua states.
/// Equivalent to 2 × LUA_MINSTACK (Lua 5.5: LUA_MINSTACK = 20).
pub const BASIC_STACK_SIZE: usize = 2 * LUA_MINSTACK;

/// Minimum guaranteed stack slots available to C functions.
/// Matches Lua 5.5's LUA_MINSTACK.
pub const LUA_MINSTACK: usize = 20;

/// Default maximum stack size (number of slots).
/// Matches Lua 5.5's LUAI_MAXSTACK.
pub const LUAI_MAXSTACK: usize = 1_000_000;

/// Default maximum function call nesting depth.
pub const MAX_CALL_DEPTH: usize = 256;

/// Extra call depth allowance granted during error-handler execution.
/// Allows error handlers and `__close` metamethods to run even after
/// a stack overflow.
pub const EXTRA_CI: usize = 30;

// ===== Strings =====

/// Maximum length for "short" strings (interned in hash table).
/// Matches Lua 5.5's LUAI_MAXSHORTLEN.
pub const LUAI_MAXSHORTLEN: usize = 40;

// ===== Compiler =====

/// Maximum number of local variables per function.
/// Matches Lua 5.5's MAXVARS.
pub const MAXVARS: usize = 200;

/// Maximum number of upvalues per function.
/// Matches Lua 5.5's MAXUPVAL.
pub const MAXUPVAL: usize = 255;

/// Maximum parser recursion depth (prevents stack overflow in parser).
/// Matches Lua 5.5's MAXCCALLS for the parser.
pub const MAXCCALLS: usize = 200;

/// Maximum index for R/K operand in instructions.
pub const MAXINDEXRK: usize = 255;

/// "No register" sentinel value in the compiler.
pub const NO_REG: u32 = 255;

/// Number of list items to flush per SETLIST instruction in table constructors.
/// Matches Lua 5.5's LFIELDS_PER_FLUSH.
pub const LFIELDS_PER_FLUSH: u32 = 50;

/// Unary operator priority in expression parser.
pub const UNARY_PRIORITY: i32 = 12;

/// Maximum length of source name in error messages.
pub const MAX_SRC_LEN: usize = 59;

// ===== Metamethods =====

/// Maximum depth for __index / __newindex metamethod chains.
/// Prevents infinite loops in metamethod resolution.
/// Matches Lua 5.5's MAXTAGLOOP.
pub const MAXTAGLOOP: usize = 2000;

// ===== Pattern Matching =====

/// Maximum number of captures in `string.find` / `string.gmatch` patterns.
/// Matches Lua 5.5's LUA_MAXCAPTURES.
pub const LUA_MAXCAPTURES: usize = 32;

/// Maximum match recursion depth for pattern matching.
pub const MAXCCALLS_PATTERN: usize = 200;

// ===== String Library =====

/// Maximum string size (1 GB).
pub const MAX_STRING_SIZE: i64 = 1 << 30;

// ===== Concat =====

/// Stack-allocated buffer size for small concatenations.
pub const CONCAT_STACK_BUF_SIZE: usize = 256;

// ===== GC Defaults =====

/// Default GC pause (percentage). Controls how long GC waits before starting
/// a new cycle. 250 = wait until memory is 2.5x the size after last collection.
pub const DEFAULT_GC_PAUSE: i32 = 250;

/// Default GC step multiplier (percentage). Controls how much work GC does
/// per step relative to memory allocation. 200 = collect 2x the allocated speed.
pub const DEFAULT_GC_STEPMUL: i32 = 200;

/// Default minor GC collection multiplier (percentage).
pub const DEFAULT_GC_MINORMUL: i32 = 200;

/// Minor-to-major GC transition threshold (percentage).
pub const DEFAULT_GC_MINORMAJOR: i32 = 70;

/// Major-to-minor GC transition threshold (percentage).
pub const DEFAULT_GC_MAJORMINOR: i32 = 50;

/// Maximum number of objects swept per single GC step.
pub const GC_SWEEPMAX: isize = 20;
