# luars-doc

Generate [EmmyLuaLS](https://github.com/EmmyLuaLs/emmylua-analyzer-rust) API
documentation from Rust source files that use `#[derive(LuaUserData)]` and
`#[lua_methods]`.

## Installation

```bash
cargo install luars_doc
```

## Usage

```bash
# Single file
luars_doc -- --file src/game_types.rs

# Recursive directory scan
luars_doc -- --dir crates/luars/src

# Write to file
luars_doc -- --dir src --out api.lua
```

The tool always outputs `---@meta` as the first line, so emmyluals treats the file
as a type-definition module.

## Extraction rules

### Structs and enums

- `#[derive(LuaUserData)]` is required for a type to appear in the output.
- Only `pub` fields are exported to Lua.
- `#[lua(skip)]` on a field hides it from Lua docs.
- `#[lua(readonly)]` marks the field as read-only.
- `#[lua(name = "...")]` overrides the Lua-visible field name.
- `#[lua_impl(Display, PartialEq, ...)]` on the struct maps to the corresponding
  metamethod annotations (see the operator table below).
- `#[lua(close = "...")]` is annotated as `@field __close fun(self: T)`.
- `#[lua(iter)]` on a `Vec<T>` field is **not** reflected in the output yet.

### Methods

- `#[lua_methods]` on an `impl` block collects all `pub fn` items.
- `#[lua(skip)]` on a method hides it.
- `#[lua(name = "...")]` overrides the Lua-visible method name.
- Instance methods (`&self` / `&mut self`) are emitted as `function T:name(...)`.
- Associated functions (no `self`, e.g. constructors) are emitted as
  `function T.name(...)`.
- `///` doc comments are preserved and placed above the annotation block.

### Type mapping

| Rust type | Lua type |
|---|---|
| `i8` … `i64`, `u8` … `u64`, `isize`, `usize` | `integer` |
| `f32`, `f64` | `number` |
| `bool` | `boolean` |
| `String`, `&str` | `string` |
| `Vec<T>` | `T[]` |
| `Option<T>` | `T\|nil` |
| `Result<T, E>` | `T` (errors become Lua errors) |
| `HashMap<K, V>` | `table<K, V>` |
| `Self` | the struct name |

### Operator mapping

Only the [EmmyLua-supported
operators](https://github.com/EmmyLuaLs/emmylua-analyzer-rust/blob/main/docs/emmylua_doc/annotations_EN/operator.md)
are emitted.

| `#[lua_impl(...)]` | EmmyLua annotation |
|---|---|
| `Add` | `---@operator add(T): T` |
| `Sub` | `---@operator sub(T): T` |
| `Mul` | `---@operator mul(T): T` |
| `Div` | `---@operator div(T): T` |
| `Rem` | `---@operator mod(T): T` |
| `Neg` | `---@operator unm: T` |
| `Pow` | `---@operator pow(T): T` |
| `PartialEq` | `---@operator eq(T): boolean` |
| `PartialOrd` | `---@operator lt(T): boolean` + `---@operator le(T): boolean` |
| `Display` | `---@field __tostring fun(self: T): string` |

The following Rust traits have **no** EmmyLua operator counterpart and are
**silently skipped**: `Not`, `BitAnd`, `BitOr`, `BitXor`, `Shl`, `Shr`.

## Example

Given this Rust source:

```rust
/// A 2D point with arithmetic operators.
#[derive(LuaUserData)]
#[lua_impl(Display, PartialEq, PartialOrd, Add, Sub)]
struct Point {
    pub x: f64,
    pub y: f64,
}

#[lua_methods]
impl Point {
    /// Create a new Point.
    pub fn new(x: f64, y: f64) -> Self { Self { x, y } }

    /// Translate the point by (dx, dy).
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
    }
}
```

The tool produces:

```lua
---@meta

--- A 2D point with arithmetic operators.
---@class Point
---@field x number
---@field y number
---@field __tostring fun(self: Point): string
---@operator eq(Point): boolean
---@operator lt(Point): boolean
---@operator le(Point): boolean
---@operator add(Point): Point
---@operator sub(Point): Point
Point = {}

--- Create a new Point.
---@param x number
---@param y number
---@return Point
function Point.new(x, y) end

--- Translate the point by (dx, dy).
---@param dx number
---@param dy number
function Point:translate(dx, dy) end
```
