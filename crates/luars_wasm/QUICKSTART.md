# Quick Start Guide

## 🚀 5-Minute Quick Experience with Lua WASM

### Prerequisites

- Rust toolchain installed
- A working HTTP server (Python, Node.js, or others)

### Steps

#### 1. Add WASM Target (only needed once)

```powershell
rustup target add wasm32-unknown-unknown
```

#### 2. Install wasm-pack (only needed once)

```powershell
cargo install wasm-pack
```

This may take a few minutes, please be patient.

#### 3. Build the WASM Module

```powershell
cd E:\lua-rs\crates\luars_wasm
wasm-pack build --target web
```

Or use the provided script:

```powershell
.\setup.ps1
```

After building, the following files will be generated in the `pkg/` directory:
- `luars_wasm_bg.wasm` - WASM binary file
- `luars_wasm.js` - JavaScript bindings
- `luars_wasm.d.ts` - TypeScript type definitions

#### 4. Start a Web Server

**Option A: Using Python**
```powershell
python -m http.server 8000
```

**Option B: Using Node.js**
```powershell
npx http-server -p 8000
```

**Option C: Using Rust**
```powershell
cargo install basic-http-server
basic-http-server .
```

#### 5. Open Your Browser

Visit http://localhost:8000

You should see the Lua WASM demo page!

### 🎮 Try These Examples

Click the "Example" button on the page, or enter the following in the editor:

```lua
-- Simple Hello World
print("Hello from Lua in your browser!")

-- Calculate Fibonacci sequence
local function fib(n)
    if n <= 1 then return n end
    return fib(n-1) + fib(n-2)
end

print("Fibonacci(10) =", fib(10))

-- Using tables
local colors = {"red", "green", "blue"}
for i, color in ipairs(colors) do
    print(i, color)
end

return "All done!"
```

Click the "Run Code" button to execute!

### ⚡ Performance Tips

- Release builds are faster: `wasm-pack build --target web --release`
- WASM file size is about 2-3 MB (optimized)
- The first load requires downloading and compiling the WASM

### 🐛 FAQ

**Q: "MIME type error" is shown**
A: You must use an HTTP server; you cannot open files directly with the file:// protocol

**Q: Build failed**
A: Make sure the wasm32-unknown-unknown target is installed:
```powershell
rustup target add wasm32-unknown-unknown
```

**Q: wasm-pack not found**
A: Restart your terminal or VS Code, and ensure ~/.cargo/bin is in your PATH

**Q: Errors in browser console**
A: Check the browser console (F12) for detailed error messages

### 📚 Next Steps

- Read `README.md` for the full API documentation
- See `PROJECT_SUMMARY.md` for project architecture
- Modify `index.html` to customize the demo page
- Edit `src/lib.rs` to add more features

### 🎉 Success!

You have now successfully run Lua code in your browser!

Try running more complex Lua programs and explore all the features of Lua 5.4.
