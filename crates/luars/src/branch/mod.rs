// Branch prediction hints for performance optimization
// Based on Lua 5.5's l_unlikely/l_likely macros
//
// In Rust stable, we use identity functions that serve as semantic markers.
// LLVM's optimizer will still perform branch prediction based on:
// 1. Profile-guided optimization (PGO)
// 2. Heuristic analysis (error paths, null checks, etc.)
// 3. Actual runtime behavior from previous runs
//
// These markers help with code readability and may be upgraded to
// use std::intrinsics when they become stable.

/// Hints to the compiler that this condition is unlikely to be true.
/// Marks error paths and rare conditions for better branch prediction.
///
/// Based on Lua 5.5's l_unlikely (uses __builtin_expect in C).
#[inline(always)]
#[cold]
pub(crate) fn unlikely(b: bool) -> bool {
    if b {
        cold()
    }
    b
}

/// Hints to the compiler that this condition is likely to be true.
/// Marks fast paths for better branch prediction.
///
/// Based on Lua 5.5's l_likely (uses __builtin_expect in C).
#[inline(always)]
pub(crate) fn likely(b: bool) -> bool {
    if !b {
        cold()
    }
    b
}

#[inline(always)]
#[cold]
fn cold() {
    // This function is marked as cold to indicate that paths leading here are unlikely.
    // It can be used to separate cold code from hot paths.
}
