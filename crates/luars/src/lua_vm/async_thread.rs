//! Async support for lua-rs: bridge Lua coroutines to Rust Futures.
//!
//! This module implements the "coroutine-driven Future bridging" pattern:
//! - An async Rust function is wrapped as a synchronous CFunction that stores a
//!   `Pin<Box<dyn Future>>` in the coroutine's `pending_future` slot, then yields
//!   with a special sentinel value.
//! - `AsyncThread` (implements `Future`) drives the coroutine: each poll checks
//!   for a pending future, polls it, and resumes the coroutine with the result.
//! - From Lua's perspective, async functions look and behave exactly like normal
//!   synchronous functions — the yield/resume is completely transparent.
//!
//! # Architecture
//!
//! ```text
//! Tokio/async runtime
//!   └── AsyncThread::poll()
//!         ├── has pending future? → poll it
//!         │     ├── Pending → return Poll::Pending
//!         │     └── Ready(result) → resume(result) → check again
//!         └── no pending future → resume(args)
//!               ├── coroutine finished → return Poll::Ready
//!               ├── async yield (sentinel) → take future, poll it
//!               └── normal yield → wake & return Pending
//! ```

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::UserDataTrait;
use crate::lua_value::{LuaUserdata, LuaValue};
use crate::lua_vm::lua_ref::RefId;
use crate::lua_vm::{LuaResult, LuaVM};

// ============ AsyncReturnValue ============

/// Return value from an async function.
///
/// Because `LuaValue` strings are GC-managed and can only be created through
/// the VM, async futures cannot directly construct string `LuaValue`s. Instead,
/// they return `AsyncReturnValue`s which are converted to `LuaValue`s by the
/// `AsyncThread` after the future completes (using the VM's string interner).
///
/// For non-GC types (integers, floats, booleans, nil), the value is stored
/// directly as a `LuaValue`. For strings, an owned `String` is stored and
/// later interned via `vm.create_string()`. For userdata, a `LuaUserdata` is
/// stored and later GC-allocated via `vm.create_userdata()`.
pub enum AsyncReturnValue {
    /// A value that doesn't need GC allocation (integer, float, bool, nil, lightuserdata)
    Value(LuaValue),
    /// A string that needs to be interned via the VM's string pool
    String(String),
    /// A userdata that needs to be GC-allocated via the VM
    UserData(LuaUserdata),
}

impl AsyncReturnValue {
    /// Create a nil return value
    #[inline]
    pub fn nil() -> Self {
        AsyncReturnValue::Value(LuaValue::nil())
    }

    /// Create an integer return value
    #[inline]
    pub fn integer(n: i64) -> Self {
        AsyncReturnValue::Value(LuaValue::integer(n))
    }

    /// Create a float return value
    #[inline]
    pub fn float(n: f64) -> Self {
        AsyncReturnValue::Value(LuaValue::float(n))
    }

    /// Create a boolean return value
    #[inline]
    pub fn boolean(b: bool) -> Self {
        AsyncReturnValue::Value(LuaValue::boolean(b))
    }

    /// Create a string return value (will be interned when passed back to Lua)
    #[inline]
    pub fn string(s: impl Into<String>) -> Self {
        AsyncReturnValue::String(s.into())
    }

    /// Create a userdata return value (will be GC-allocated when passed back to Lua)
    #[inline]
    pub fn userdata<T: UserDataTrait>(data: T) -> Self {
        AsyncReturnValue::UserData(LuaUserdata::new(data))
    }
}

/// Convenience conversions
impl From<i64> for AsyncReturnValue {
    fn from(n: i64) -> Self {
        AsyncReturnValue::integer(n)
    }
}
impl From<f64> for AsyncReturnValue {
    fn from(n: f64) -> Self {
        AsyncReturnValue::float(n)
    }
}
impl From<bool> for AsyncReturnValue {
    fn from(b: bool) -> Self {
        AsyncReturnValue::boolean(b)
    }
}
impl From<String> for AsyncReturnValue {
    fn from(s: String) -> Self {
        AsyncReturnValue::String(s)
    }
}
impl From<&str> for AsyncReturnValue {
    fn from(s: &str) -> Self {
        AsyncReturnValue::String(s.to_string())
    }
}
impl From<LuaValue> for AsyncReturnValue {
    fn from(v: LuaValue) -> Self {
        AsyncReturnValue::Value(v)
    }
}
impl From<LuaUserdata> for AsyncReturnValue {
    fn from(ud: LuaUserdata) -> Self {
        AsyncReturnValue::UserData(ud)
    }
}

// ============ Async Future type alias ============

/// Type-erased async future stored in LuaState.pending_future.
/// Returns `AsyncReturnValue`s which are converted to `LuaValue`s by the
/// `AsyncThread` using the VM's string interner.
/// Not `Send` — must run on a single-threaded / LocalSet executor.
pub type AsyncFuture = Pin<Box<dyn Future<Output = LuaResult<Vec<AsyncReturnValue>>>>>;

// ============ Async sentinel ============

/// Static storage whose *address* is used as the async sentinel value.
/// When an async CFunction yields, it yields a single light userdata whose
/// pointer equals `&ASYNC_SENTINEL_STORAGE`. This lets `AsyncThread` distinguish
/// "async yield" from "normal coroutine.yield()".
static ASYNC_SENTINEL_STORAGE: u8 = 0;

/// Create a `LuaValue::lightuserdata` pointing to the sentinel address.
#[inline]
pub fn async_sentinel_value() -> LuaValue {
    LuaValue::lightuserdata(&ASYNC_SENTINEL_STORAGE as *const u8 as *mut std::ffi::c_void)
}

/// Check whether a set of yield values represents an async yield.
/// An async yield is exactly one value: a light userdata equal to the sentinel pointer.
#[inline]
pub fn is_async_sentinel(values: &[LuaValue]) -> bool {
    if values.len() != 1 {
        return false;
    }
    let v = &values[0];
    v.ttislightuserdata()
        && v.pvalue() == &ASYNC_SENTINEL_STORAGE as *const u8 as *mut std::ffi::c_void
}

// ============ ResumeResult ============

/// Classifies the outcome of a single `coroutine.resume()` call.
enum ResumeResult {
    /// Coroutine finished (completed or errored). Carries the final result.
    Finished(LuaResult<Vec<LuaValue>>),
    /// Async yield — the coroutine yielded with ASYNC_SENTINEL and stored a
    /// pending future in `LuaState::pending_future`.
    AsyncYield(AsyncFuture),
    /// Normal `coroutine.yield(values)` from Lua code (not an async call).
    NormalYield(Vec<LuaValue>),
}

// ============ Helper: convert AsyncReturnValues to LuaValues ============

/// Convert a vector of `AsyncReturnValue` to `LuaValue` using the VM for string interning.
fn materialize_values(vm: &mut LuaVM, values: Vec<AsyncReturnValue>) -> LuaResult<Vec<LuaValue>> {
    let mut result = Vec::with_capacity(values.len());
    for v in values {
        match v {
            AsyncReturnValue::Value(lv) => result.push(lv),
            AsyncReturnValue::String(s) => {
                let lv = vm.create_string(&s)?;
                result.push(lv);
            }
            AsyncReturnValue::UserData(ud) => {
                let lv = vm.create_userdata(ud)?;
                result.push(lv);
            }
        }
    }
    Ok(result)
}

// ============ AsyncThread ============

/// Wraps a Lua coroutine as a Rust `Future`.
///
/// Drives the coroutine to completion by repeatedly resuming it and polling
/// any pending async futures. The coroutine's `LuaState` is rooted in the
/// VM registry to prevent garbage collection while the `AsyncThread` is alive.
///
/// # Safety
///
/// - `AsyncThread` is `!Send` and `!Sync` (contains raw pointer to `LuaVM`).
/// - Must be polled from the same thread that created it.
/// - The `LuaVM` must outlive the `AsyncThread`.
///
/// # Example
///
/// ```ignore
/// let async_thread = vm.create_async_thread(chunk)?;
/// let results = async_thread.await?;
/// ```
pub struct AsyncThread {
    /// The coroutine's thread value, stored as a raw LuaValue.
    /// This value is also rooted in the registry via `ref_id` to prevent GC.
    thread_val: LuaValue,

    /// Raw pointer to the owning VM (for resume and registry access).
    /// Not `Send`/`Sync` — this is intentional.
    vm: *mut LuaVM,

    /// Registry reference ID that keeps the thread alive against GC.
    /// Released on drop.
    ref_id: RefId,

    /// Currently pending async future (taken from the coroutine after async yield).
    pending: Option<AsyncFuture>,

    /// Initial arguments for the first resume (consumed on first poll).
    initial_args: Option<Vec<LuaValue>>,
}

impl AsyncThread {
    /// Create a new `AsyncThread` from a thread `LuaValue`.
    ///
    /// The thread value is rooted in the VM registry to prevent garbage
    /// collection. The caller must ensure `vm` is valid for the lifetime
    /// of this `AsyncThread`.
    ///
    /// # Arguments
    /// - `thread_val` — A `LuaValue` of type Thread (from `create_thread`)
    /// - `vm` — Raw pointer to the owning `LuaVM`
    /// - `args` — Arguments passed to the coroutine's first resume
    pub(crate) fn new(thread_val: LuaValue, vm: *mut LuaVM, args: Vec<LuaValue>) -> Self {
        // Root the thread in the registry so GC won't collect it
        let ref_id = {
            let vm_ref = unsafe { &mut *vm };
            let lua_ref = vm_ref.create_ref(thread_val);
            // Extract the RefId; for threads (GC objects) this will be Registry variant
            lua_ref.ref_id().unwrap_or(0)
        };

        AsyncThread {
            thread_val,
            vm,
            ref_id,
            pending: None,
            initial_args: Some(args),
        }
    }

    /// Resume the coroutine with the given arguments and classify the result.
    fn do_resume(&mut self, args: Vec<LuaValue>) -> ResumeResult {
        let thread_state = match self.thread_val.as_thread_mut() {
            Some(state) => state,
            None => {
                return ResumeResult::Finished(Err(
                    unsafe { &mut *self.vm }.error("AsyncThread: invalid thread value".to_string())
                ));
            }
        };

        match thread_state.resume(args) {
            Ok((true, results)) => {
                // Coroutine completed normally
                ResumeResult::Finished(Ok(results))
            }
            Ok((false, values)) => {
                // Coroutine yielded — check if async or normal
                if is_async_sentinel(&values) {
                    // Take the pending future from the coroutine's LuaState
                    match thread_state.take_pending_future() {
                        Some(fut) => ResumeResult::AsyncYield(fut),
                        None => {
                            // Bug: yielded with sentinel but no future stored
                            ResumeResult::Finished(Err(unsafe { &mut *self.vm }
                                .error("async yield without pending future".to_string())))
                        }
                    }
                } else {
                    ResumeResult::NormalYield(values)
                }
            }
            Err(e) => ResumeResult::Finished(Err(e)),
        }
    }

    /// Poll the pending future and handle its completion.
    /// Returns `Poll::Pending` if the future is not ready, otherwise resumes
    /// the coroutine and processes the result.
    fn poll_pending(&mut self, cx: &mut Context<'_>) -> Poll<LuaResult<Vec<LuaValue>>> {
        loop {
            if let Some(ref mut fut) = self.pending {
                match fut.as_mut().poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(result) => {
                        self.pending = None;
                        let resume_args = match result {
                            Ok(async_values) => {
                                // Convert AsyncReturnValues to LuaValues using the VM
                                let vm = unsafe { &mut *self.vm };
                                match materialize_values(vm, async_values) {
                                    Ok(values) => values,
                                    Err(e) => return Poll::Ready(Err(e)),
                                }
                            }
                            Err(e) => {
                                // Future errored — propagate to caller
                                return Poll::Ready(Err(e));
                            }
                        };
                        // Future completed — resume coroutine with results
                        match self.do_resume(resume_args) {
                            ResumeResult::Finished(r) => return Poll::Ready(r),
                            ResumeResult::AsyncYield(fut) => {
                                self.pending = Some(fut);
                                continue; // poll the new future immediately
                            }
                            ResumeResult::NormalYield(_vals) => {
                                // Normal yield inside async context — wake immediately
                                // to re-poll (this lets the event loop breathe)
                                cx.waker().wake_by_ref();
                                return Poll::Pending;
                            }
                        }
                    }
                }
            } else {
                // No pending future — should not reach here
                return Poll::Ready(Err(unsafe { &mut *self.vm }
                    .error("AsyncThread: no pending future to poll".to_string())));
            }
        }
    }
}

impl Future for AsyncThread {
    type Output = LuaResult<Vec<LuaValue>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // If there's a pending future from a previous async yield, poll it
        if this.pending.is_some() {
            return this.poll_pending(cx);
        }

        // First resume or resume after normal yield
        let args = this.initial_args.take().unwrap_or_default();
        match this.do_resume(args) {
            ResumeResult::Finished(r) => Poll::Ready(r),
            ResumeResult::AsyncYield(fut) => {
                this.pending = Some(fut);
                this.poll_pending(cx)
            }
            ResumeResult::NormalYield(_vals) => {
                // Normal yield — wake immediately to try again
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

impl Drop for AsyncThread {
    fn drop(&mut self) {
        // Release the registry reference to allow the thread to be GC'd
        if self.ref_id > 0 {
            let vm = unsafe { &mut *self.vm };
            vm.release_ref_id(self.ref_id);
        }
    }
}

// ============ Async function wrapper ============

/// Wrap an async function factory into a synchronous `Fn(&mut LuaState) -> LuaResult<usize>`.
///
/// The returned closure, when called from Lua:
/// 1. Collects function arguments from the Lua stack
/// 2. Invokes `f(args)` to create a `Future`
/// 3. Stores the future in `LuaState::pending_future`
/// 4. Yields with `ASYNC_SENTINEL` to signal the `AsyncThread`
///
/// This function is used internally by `register_async`.
///
/// # Type parameters
/// - `F`: Factory closure `Fn(Vec<LuaValue>) -> Fut`
/// - `Fut`: The async future type `Future<Output = LuaResult<Vec<AsyncReturnValue>>>`
pub fn wrap_async_function<F, Fut>(
    f: F,
) -> impl Fn(&mut crate::lua_vm::LuaState) -> LuaResult<usize> + 'static
where
    F: Fn(Vec<LuaValue>) -> Fut + 'static,
    Fut: Future<Output = LuaResult<Vec<AsyncReturnValue>>> + 'static,
{
    move |state: &mut crate::lua_vm::LuaState| {
        // 1. Collect arguments from the Lua stack
        let args = state.get_args();

        // 2. Create the future by calling the factory
        let future = f(args);

        // 3. Store the future in the coroutine's pending slot
        state.set_pending_future(Box::pin(future));

        // 4. Yield with the async sentinel to signal AsyncThread
        state.do_yield(vec![async_sentinel_value()])?;

        // This point is never reached — do_yield always returns Err(Yield)
        Ok(0)
    }
}

// ============ AsyncCallHandle ============

/// Lua source for the reusable runner coroutine.
///
/// Receives the target function on first resume, then loops:
/// 1. Yield to wait for call arguments
/// 2. Call the target function via `pcall` (so Lua errors don't kill the runner)
/// 3. Yield the pcall results back to Rust
///
/// **Requires** the table standard library (`table.pack` / `table.unpack`).
pub(crate) const ASYNC_CALL_RUNNER: &str = "\
local f = ...\n\
local args = table.pack(coroutine.yield())\n\
while true do\n\
  args = table.pack(coroutine.yield(pcall(f, table.unpack(args, 1, args.n))))\n\
end";

/// A reusable async call handle that keeps a runner coroutine alive
/// across multiple calls to the same Lua function.
///
/// Created via [`LuaVM::create_async_call_handle`] or
/// [`LuaVM::create_async_call_handle_global`].
///
/// Unlike [`LuaVM::call_async`] which creates a new coroutine for each
/// invocation, `AsyncCallHandle` reuses a single coroutine, reducing GC
/// pressure and allocation overhead for repeated calls.
///
/// The runner uses `pcall` internally, so Lua errors in the target function
/// are caught and returned as `Err` without invalidating the handle. Only
/// infrastructure failures (async future errors, coroutine death) mark the
/// handle as dead.
///
/// # Example
///
/// ```ignore
/// let mut handle = vm.create_async_call_handle_global("process")?;
/// for item in items {
///     let arg = vm.create_string(&item)?;
///     let result = handle.call(vec![arg]).await?;
/// }
/// ```
pub struct AsyncCallHandle {
    /// The runner coroutine thread value.
    thread_val: LuaValue,
    /// Raw pointer to the owning VM.
    vm: *mut LuaVM,
    /// Registry reference that keeps the thread alive against GC.
    ref_id: RefId,
    /// Whether the handle is still usable.
    alive: bool,
}

impl AsyncCallHandle {
    /// Create a new `AsyncCallHandle`.
    ///
    /// The target function is passed to the runner coroutine on the first
    /// resume. After initialization, the handle is ready for [`call`](Self::call).
    pub(crate) fn new(thread_val: LuaValue, vm: *mut LuaVM, func: LuaValue) -> LuaResult<Self> {
        let ref_id = {
            let vm_ref = unsafe { &mut *vm };
            let lua_ref = vm_ref.create_ref(thread_val);
            lua_ref.ref_id().unwrap_or(0)
        };

        let handle = AsyncCallHandle {
            thread_val,
            vm,
            ref_id,
            alive: true,
        };

        // First resume: pass the target function to the runner.
        // The runner captures it via `...` and yields, waiting for call args.
        let thread_state = handle
            .thread_val
            .as_thread_mut()
            .ok_or_else(|| unsafe { &mut *vm }.error("invalid thread value".to_string()))?;
        let (finished, _) = thread_state.resume(vec![func])?;
        if finished {
            return Err(
                unsafe { &mut *vm }.error("runner coroutine finished during init".to_string())
            );
        }

        Ok(handle)
    }

    /// Returns `true` if the handle is still usable for calls.
    ///
    /// A handle becomes dead if:
    /// - An async future returned an error
    /// - The runner coroutine terminated unexpectedly
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Resume the runner coroutine and classify the result.
    fn do_resume(&mut self, args: Vec<LuaValue>) -> ResumeResult {
        let thread_state = match self.thread_val.as_thread_mut() {
            Some(state) => state,
            None => {
                return ResumeResult::Finished(Err(
                    unsafe { &mut *self.vm }.error("invalid thread value".to_string())
                ));
            }
        };

        match thread_state.resume(args) {
            Ok((true, results)) => ResumeResult::Finished(Ok(results)),
            Ok((false, values)) => {
                if is_async_sentinel(&values) {
                    match thread_state.take_pending_future() {
                        Some(fut) => ResumeResult::AsyncYield(fut),
                        None => ResumeResult::Finished(Err(unsafe { &mut *self.vm }
                            .error("async yield without pending future".to_string()))),
                    }
                } else {
                    ResumeResult::NormalYield(values)
                }
            }
            Err(e) => ResumeResult::Finished(Err(e)),
        }
    }

    /// Call the wrapped function with the given arguments.
    ///
    /// Drives the runner coroutine, handling async yields transparently.
    /// Returns the function's return values on success.
    ///
    /// If the target function raises a Lua error, it is caught by the internal
    /// `pcall` and returned as `Err`, but the handle **remains alive** for
    /// future calls. If an async future fails or the coroutine dies, the handle
    /// is marked as dead and subsequent calls return an error immediately.
    pub async fn call(&mut self, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>> {
        if !self.alive {
            return Err(
                unsafe { &mut *self.vm }.error("async call handle is no longer alive".to_string())
            );
        }

        let mut resume_args = args;
        loop {
            match self.do_resume(resume_args) {
                ResumeResult::Finished(result) => {
                    self.alive = false;
                    match result {
                        Ok(_) => {
                            return Err(unsafe { &mut *self.vm }
                                .error("runner coroutine finished unexpectedly".to_string()));
                        }
                        Err(e) => return Err(e),
                    }
                }
                ResumeResult::AsyncYield(fut) => match fut.await {
                    Ok(async_values) => {
                        let vm = unsafe { &mut *self.vm };
                        resume_args = materialize_values(vm, async_values)?;
                    }
                    Err(e) => {
                        self.alive = false;
                        return Err(e);
                    }
                },
                ResumeResult::NormalYield(values) => {
                    // The runner yields pcall results: (ok: bool, results... | err)
                    let ok = values.first().and_then(|v| v.as_boolean()).unwrap_or(false);
                    if ok {
                        return Ok(values[1..].to_vec());
                    } else {
                        // Lua error caught by pcall — handle stays alive
                        let err_msg = values
                            .get(1)
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown error")
                            .to_string();
                        return Err(unsafe { &mut *self.vm }.error(err_msg));
                    }
                }
            }
        }
    }
}

impl Drop for AsyncCallHandle {
    fn drop(&mut self) {
        if self.ref_id > 0 {
            let vm = unsafe { &mut *self.vm };
            vm.release_ref_id(self.ref_id);
        }
    }
}
