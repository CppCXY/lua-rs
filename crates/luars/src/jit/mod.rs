/// JIT compiler for hot Lua numeric for-loops.
///
/// Enabled only when the `jit` Cargo feature is active.
/// The interpreter always remains fully functional as the fallback.
///
/// # Trigger
///
/// Unlike traditional hot-count JITs that count _call-site_ invocations,
/// we trigger on _iteration count_ instead.  `ForPrep` fires exactly once
/// per loop invocation; a `for i=1,10000000 do` loop only invokes ForPrep
/// once.  After `handle_forprep_int` the stack slot `stack[base+a]` holds
/// the total iteration count.  We compile immediately when that count ≥
/// `JIT_MIN_ITERS` (default 1000), meaning "this loop is worth the compile".
///
/// # Compiled function contract
///
/// ```text
/// unsafe extern "C" fn(stack_base: *mut u8) -> i32
/// ```
/// - `stack_base` = `stack.as_mut_ptr().add(base)` cast to `*mut u8`.
/// - Return  0 = loop ran to completion; interpreter skips past ForLoop.
/// - Return -1 = type mismatch at entry (deopt), interpreter handles loop.
///
/// # Cache layout
///
/// `Chunk::jit_cache: RefCell<HashMap<u32, usize>>` maps ForPrep PC →
/// - absent (`None`): never visited
/// - `JIT_FAILED` (= 0): compilation tried and failed, never retry
/// - any other value: pointer to compiled machine code, valid forever

pub mod analyzer;
pub mod compiler;
pub mod runtime;

/// Minimum loop iteration count to trigger JIT compilation.
/// A loop that will run fewer than this many iterations is not worth compiling.
pub const JIT_MIN_ITERS: usize = 1000;

/// Sentinel stored in `jit_cache` to mean "compilation was attempted and failed".
/// 0 is safe because valid function pointers are never NULL.
pub const JIT_FAILED: usize = 0;

/// Compiled loop function: `fn(stack_base: *mut u8) -> i32`.
/// - Return 0  = loop ran to completion.
/// - Return -1 = deopt (type mismatch), let interpreter handle it.
pub type JitLoopFn = unsafe extern "C" fn(*mut u8) -> i32;

/// Try to JIT-compile the integer for-loop whose `ForPrep` is at `prep_pc`.
///
/// Returns `Some(fn_ptr)` on success, `None` if the loop is not JIT-able.
pub fn try_compile_loop(
    chunk: &crate::lua_value::Chunk,
    prep_pc: usize,
) -> Option<JitLoopFn> {
    let analysis = analyzer::analyze(chunk, prep_pc)?;
    compiler::compile(&analysis)
}
