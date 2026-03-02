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

## 2. String model: UTF-8 only, no arbitrary binary bytes

luars strings are UTF-8. Lua escape sequences like `\255` that produce non-UTF-8 bytes are not representable in the `string` type. A separate `binary` type exists for raw bytes but does not interoperate with string functions.

Affected tests:
- `pm.lua` (~L356, ~L413) — patterns using `\255`, pointer-identity test
- `utf8.lua` (~L130–L224) — invalid-byte / continuation-byte / extended-codepoint tests
- `strings.lua` (~L211) — `string.format("%c", ...)` tests
- `literals.lua` (~L125) — variable-name characters via binary find
- `files.lua` (~L572) — BOM byte-level comparison

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
