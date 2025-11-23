# Lua WASM

WebAssembly bindings for the Lua interpreter, allowing you to run Lua code in the browser and interact with JavaScript.

## Features

- ✅ Execute Lua 5.4 code in the browser
- ✅ Register JavaScript functions callable from Lua
- ✅ Set and get global variables
- ✅ Bidirectional Lua ↔ JavaScript value conversion
- ✅ Full standard library support (except IO)

## Building

### Prerequisites

1. Install Rust: https://rustup.rs/
2. Install wasm-pack:
   ```bash
   cargo install wasm-pack
   ```

### Build WASM module

```bash
# In the luars_wasm directory
wasm-pack build --target web
```

This will generate the WASM files in the `pkg/` directory.

## Running the Demo

After building, you need to serve the files with a web server (due to CORS restrictions):

### Option 1: Using Python
```bash
python -m http.server 8000
```

### Option 2: Using Node.js http-server
```bash
npx http-server -p 8000
```

### Option 3: Using Rust's basic-http-server
```bash
cargo install basic-http-server
basic-http-server .
```

Then open http://localhost:8000 in your browser.

## API

### Creating a Lua VM

```javascript
import init, { LuaWasm } from './pkg/luars_wasm.js';

await init();
const lua = new LuaWasm();
```

### Executing Lua Code

```javascript
try {
    const result = lua.execute(`
        local x = 10
        local y = 20
        return x + y
    `);
    console.log('Result:', result);
} catch (e) {
    console.error('Lua error:', e);
}
```

### Registering JavaScript Functions

```javascript
// Register a function that Lua can call
lua.registerFunction('jsAlert', (message) => {
    alert(message);
});

// Use it in Lua
lua.execute(`
    jsAlert("Hello from Lua!")
`);
```

### Setting/Getting Global Variables

```javascript
// Set a global variable
lua.setGlobal('myNumber', 42);

// Get a global variable
const value = lua.getGlobal('myNumber');
console.log(value); // 42
```

### Evaluating Expressions

```javascript
// Evaluate and return result
const result = lua.eval('2 + 2 * 3');
console.log(result); // 8
```

## Examples

### Example 1: Basic Math

```javascript
const result = lua.execute(`
    local function factorial(n)
        if n <= 1 then return 1 end
        return n * factorial(n - 1)
    end
    
    return factorial(5)
`);
console.log(result); // "120"
```

### Example 2: Calling JS from Lua

```javascript
// Register a logging function
lua.registerFunction('log', (msg) => {
    console.log('[Lua]', msg);
});

lua.execute(`
    for i = 1, 5 do
        log("Count: " .. i)
    end
`);
```

### Example 3: Working with Tables

```javascript
lua.execute(`
    local person = {
        name = "Alice",
        age = 30,
        hobbies = {"reading", "coding"}
    }
    
    for key, value in pairs(person) do
        print(key .. ": " .. tostring(value))
    end
`);
```

## Limitations

- IO operations are not available in WASM environment
- FFI (Foreign Function Interface) is disabled for WASM
- File system operations are not supported
- Some OS-specific functions may not work

## Architecture

The WASM module uses:
- `wasm-bindgen` for JavaScript interop
- `js-sys` for JavaScript standard library access
- `web-sys` for Web APIs
- The core `luars` interpreter with `wasm` feature enabled

## Development

To watch for changes and rebuild automatically:

```bash
cargo watch -i .gitignore -i "pkg/*" -s "wasm-pack build --target web"
```

## License

MIT License - See main project LICENSE file for details.
