
# 0.26.0

refactor many internal struct, reduce many unsafe and optimize performance.


# 0.24.0 

## Userdata Sub-Reference System

Complete redesign of userdata lifetime management. Borrowed userdata (sub-references)
now share a `RefAliveToken` with their parent, automatically becoming invalid when the
parent is garbage collected.

### Breaking Changes

- **`LuaUserdata::get_trait()` now returns `Result<&dyn UserDataTrait, LuaError>`**
  instead of `&dyn UserDataTrait` directly. All call sites must handle the `Err` case.
  The same applies to `get_trait_mut()`, `get_data()`, `get_data_mut()`.

- **`LuaError::ExpiredReference`** — new error variant for expired sub-references.
  Static message: *"attempt to use an expired reference"*. Does not consume
  `vm.error_message`.

- **`RefUserData<T>` removed** — replaced by `LuaUserdata`'s `Borrowed` storage variant.
  Use `LuaUserdata::from_ptr(ptr, token)` or `LuaUserdata::from_ref(&mut ref, token)`.

- **`SubRefGuard` removed** — liveness tracking is now built into `LuaUserdata` itself
  via `RefAliveToken`.

- **`ScopedBorrowedUserData<T>` removed** — `Scope::create_userdata_ref` now uses
  `LuaUserdata::from_ref` internally.

### Additions

- **`UserdataStorage` enum** — `Owned(Box<dyn UserDataTrait>)` | `Borrowed(*mut dyn UserDataTrait)`.
  `LuaUserdata` now uses this enum for its data field.

- **`RefAliveToken`** — `Rc<Cell<bool>>` wrapper. Owned userdata creates one on construction
  and flips it on drop. Borrowed userdata shares a clone from the parent.

- **`#[derive(LuaUserData)]` — non-primitive field access returns sub-references**.
  Fields of userdata type (e.g. `pub pos: Position`) are no longer copied — accessing
  them creates a borrowed sub-reference sharing the parent's token.

- **Auto-detection of `RefAliveToken`** — non-pub fields of type `RefAliveToken`
  are automatically detected by the derive macro, enabling `IntoLua for &T` and
  `IntoLua for &mut T` without any additional attribute.

- **`#[lua_methods]` — auto-wraps reference returns** — methods returning `&T`,
  `&mut T`, `Option<&T>`, `Result<&T, E>`, etc. are automatically wrapped as
  borrowed userdata using the parent's token.

- **`UdValue::SubRef(*const dyn UserDataTrait)`** — new variant for sub-reference
  markers returned by `get_field`. The VM converts these to borrowed `LuaUserdata`.

- **`check_alive_or_error()` removed** — replaced by `get_trait()` returning `Result`.
  All VM userdata dispatch points call `get_trait()?` to propagate `ExpiredReference`.

### Internal

- All VM call sites (`helper.rs`, `metamethod.rs`, `call.rs`, `lua_state.rs`,
  `stdlib`) updated to handle `get_trait()` → `Result`.
- `Scope` uses `RefAliveToken` directly (no separate `Rc<Cell<bool>>`).
- `DeadUserdata` sentinel removed — errors propagate via `LuaError::ExpiredReference`.
