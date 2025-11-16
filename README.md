# lua-rs

[![CI](https://github.com/CppCXY/lua-rs/workflows/CI/badge.svg)](https://github.com/CppCXY/lua-rs/actions)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> ⚠️ **AI-Generated Project Notice**: This is an experimental Lua 5.4 interpreter implementation where **most of the functionality was developed by AI** (GitHub Copilot/Claude). While it demonstrates impressive AI coding capabilities, it should be considered a proof-of-concept rather than production-ready software.

A Lua 5.4 interpreter implemented in Rust, primarily developed through AI-assisted programming. This project serves as an exploration of:
- AI's capability to implement complex systems like language interpreters
- Lua 5.4 VM architecture and semantics
- Rust's suitability for interpreter implementation

## Test Coverage

Current test status: **120 out of 124 tests passing (96.8%)**

### Implemented Features ✅

#### Core Language Features
- ✅ All basic operators (arithmetic, logical, bitwise, comparison)
- ✅ Control flow (if/else, while, repeat, for loops, goto/labels)
- ✅ Functions and closures with upvalues
- ✅ Tables with metatables and metamethods
- ✅ Coroutines (create, resume, yield, status)
- ✅ Variable arguments (`...`) with multi-value expansion
- ✅ Multiple assignment and returns
- ✅ String pattern matching (Lua patterns, not regex)

#### Standard Libraries
- ✅ **Basic**: `print`, `assert`, `type`, `tonumber`, `tostring`, `pcall`, `xpcall`, `error`, `select`, `ipairs`, `pairs`, `next`, `rawget`, `rawset`, `rawlen`, `rawequal`, `getmetatable`, `setmetatable`
- ✅ **String**: All string manipulation functions including `pack`/`unpack` for binary data
- ✅ **Table**: `insert`, `remove`, `sort`, `concat`, `pack`, `unpack`, `move`
- ✅ **Math**: All math functions including `tointeger`, `ult`, bitwise operations
- ✅ **UTF-8**: Full UTF-8 support (`codes`, `codepoint`, `len`, `offset`, `char`)
- ✅ **Coroutine**: `create`, `resume`, `yield`, `status`, `close`, `isyieldable`
- ✅ **Package**: `require`, `module`, `searchers` (partial)
- ⚠️ **IO**: Basic file operations (has known memory issues, tests skipped)

### Known Limitations ⚠️

1. **Vararg Table Constructor**: `{...}` in certain contexts has register allocation issues
2. **Coroutine Edge Cases**: 3 coroutine tests fail (`test_coroutine_running`, `test_coroutine_status`, `test_coroutine_wrap`)
3. **IO Library UB**: File operations have undefined behavior issues causing crashes in tests
4. **No JIT**: Pure interpreter, no Just-In-Time compilation
5. **Limited Optimization**: Minimal compile-time optimizations
6. **No Debug Library**: Debug introspection not implemented

## Architecture

### Components

- **Parser**: Uses `emmylua-parser` for parsing Lua source code
- **Compiler**: Single-pass bytecode compiler
- **VM**: Register-based virtual machine with NaN-boxing value representation
- **GC**: Simple mark-and-sweep garbage collector
- **FFI**: Experimental C FFI support (incomplete)

### Value Representation

Uses NaN-boxing to store all Lua values in 64 bits:
- Immediate integers and floats
- Pointer-tagged for strings, tables, functions, etc.

## Building

```bash
# Build the project
cargo build --release

# Run tests (skipping IO tests due to UB)
cargo test --lib -- --skip test_io

# Run a Lua script
./target/release/main script.lua

# Dump bytecode
./target/release/bytecode_dump script.lua
```

## Examples

```lua
-- Variable arguments with table unpacking
local function sum(...)
    local args = {...}  -- ⚠️ Known issue: may fail in some contexts
    local total = 0
    for i = 1, #args do
        total = total + args[i]
    end
    return total
end

print(sum(1, 2, 3, 4, 5))  -- 15

-- Coroutines
local co = coroutine.create(function()
    for i = 1, 3 do
        print("Iteration:", i)
        coroutine.yield()
    end
end)

for i = 1, 3 do
    coroutine.resume(co)
end

-- String patterns
local text = "Hello, World!"
local matches = {}
for word in text:gmatch("%a+") do
    table.insert(matches, word)
end
-- matches = {"Hello", "World"}
```

## Development Status

This project is **not recommended for production use**. It was created primarily as:
1. An experiment in AI-assisted software development
2. A learning exercise for Lua VM internals
3. A demonstration of Rust's capabilities for interpreter implementation

### AI Development Notes

Most of the codebase was generated through iterative AI prompts and debugging sessions. The AI successfully:
- ✅ Implemented a working Lua 5.4 VM from scratch
- ✅ Achieved 96.8% test compatibility
- ✅ Handled complex features like coroutines and metatables
- ⚠️ Struggled with some edge cases and memory safety issues

## Contributing

While this is primarily an AI-generated experiment, issues and discussions are welcome for:
- Identifying bugs or undefined behavior
- Suggesting improvements to AI-generated code
- Discussing Lua VM implementation techniques

## License

MIT License - See [LICENSE](LICENSE) file for details.

## Acknowledgments

- **emmylua-parser**: For providing the parser infrastructure

---

**Disclaimer**: This implementation has not been audited for security or correctness. Use at your own risk.
