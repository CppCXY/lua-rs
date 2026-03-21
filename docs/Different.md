# Behavior Differences: luars vs C Lua 5.5

This document records all known behavioral differences between luars (Rust implementation) and the official C Lua 5.5 reference interpreter.

---

## 1. No C API / testC support

luars is a pure Rust implementation with no C API (`lua_State*`, `lua_pushinteger`, etc.). The official test suite's `T` (testC) library is unavailable, causing the following test files/sections to be fully or partially skipped:

| Test file | Scope | Reason |
|-----------|-------|--------|
| `api.lua` | whole file | C API tests |
| `memerr.lua` | whole file | C memory-error injection |
| `code.lua` | whole file | opcode-level tests via `T.listcode` |
| `main.lua` | whole file | interactive interpreter / CLI arg tests |
| `db.lua` | whole file | full debug-library tests (hook-dependent) |
| `coroutine.lua` | 2 sections (~L595, ~L1062) | C API coroutine tests |
| `events.lua` | 1 section (~L331) | userdata event tests |
| `errors.lua` | 1 section (~L94) | C function error-message tests |
| `gc.lua` | 1 section (~L401) | userdata GC tests |
| `strings.lua` | 2 sections (~L474, ~L534) | `pushfstring` / external-string tests |
| `nextvar.lua` | 2 sections (~L146, ~L665) | table-size / table-lib-on-non-tables tests |

---

## 2. Rust host API string view

At the Lua level, luars now stores strings as raw byte strings, so arbitrary bytes are preserved just like in C Lua.

The remaining difference is on the Rust host side:

- `LuaValue::as_str()` only returns `Some(&str)` when the underlying bytes are valid UTF-8.
- `LuaValue::as_bytes()` should be used when exact byte preservation matters.
- `LuaState::create_bytes()` can be used to create Lua strings from arbitrary bytes without forcing UTF-8.

This is an embedding/API difference, not a Lua-language semantic difference.

---

## 3. No locale-dependent number parsing

Number parsing always uses `.` as the decimal separator and is unaffected by `os.setlocale`.

Affected: `literals.lua` (~L300–L325) — locale test block skipped via `if false then`.

---

## 4. No C module loading

- `package.loadlib` always returns `"loadlib not implemented"`.
- The `package.cpath` searcher (searcher 3) always fails; `.so` / `.dll` modules cannot be loaded.

Affected: `attrib.lua` (~L265, ~L335) — C library / external-string tests skipped.

---

## 5. debug library limitations
The `debug` library is partially implemented. `debug.setuservalue` and `debug.getuservalue` are not implemented. Because we are different userdata implementations.

---

## 6. Bytecode format

luars uses its own bytecode format, incompatible with C Lua binary chunks. `string.dump` output can only be loaded by luars itself. Upvalue names are not included in the dump.

The on-disk format still distinguishes UTF-8 string constants from raw byte-string constants so luars can round-trip non-UTF-8 data through `string.dump` and `load`, even though the runtime no longer has a separate `binary` value tag.

Affected: `calls.lua` (~L484) — binary-chunk header tests skipped.

---

## 7. `io.popen` not implemented

`io.popen` always raises `"io.popen not yet implemented"`. Tests in `files.lua` are guarded by `pcall(io.popen, ...)` and gracefully skipped.

---

## 8. GC implementation differences

- `__gc` finalizers do not save/restore `L->allowhook` or set `CIST_FIN`.
- Stack shrinking during GC is not implemented.

---

## 9. Coroutine limitations

- Multi-value `..` (OP_CONCAT) across a yield does not resume correctly — missing `finishOp` continuation for CONCAT. Test in `coroutine.lua` (~L982) skipped.
- Stack-overflow limit differs from C Lua's 1 MiB policy. Test in `coroutine.lua` (~L839) skipped.

---

## 10. `os.date` DST flag

`os.date` does not compute the `isdst` (daylight saving time) field correctly; it defaults to `false`.

---

## 11. Long-string address reuse

C Lua may reuse the same address for identical long-string constants (useful with `const` tags). luars allocates separate objects.

Affected: `literals.lua` (~L213) — long-string reuse test skipped.
