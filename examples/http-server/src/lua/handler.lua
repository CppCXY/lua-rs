-- handler.lua — Default HTTP request handler for luars async server
--
-- This script demonstrates:
--   1. Route-based request dispatching
--   2. Async I/O (sleep, read_file)
--   3. JSON-style response building
--   4. Error handling with pcall
--
-- Available async functions (registered from Rust):
--   sleep(seconds)               → true
--   read_file(path)              → content, err
--   write_file(path, content)    → ok, err
--   time()                       → unix_timestamp (float)
--   env(name)                    → value or nil
--   log(...)                     → nil (prints to server stderr)

-- ─── Simple JSON encoder ────────────────────────────────────────────────────

local function json_encode_value(val, depth)
    depth = depth or 0
    if depth > 20 then return '"[max depth]"' end

    local t = type(val)
    if t == "string" then
        return '"' .. val:gsub('\\', '\\\\'):gsub('"', '\\"'):gsub('\n', '\\n'):gsub('\r', '\\r') .. '"'
    elseif t == "number" then
        if val ~= val then return '"NaN"' end
        if val == 1/0 then return '"Infinity"' end
        if val == -1/0 then return '"-Infinity"' end
        if val == math.floor(val) and math.abs(val) < 2^53 then
            return string.format("%d", val)
        end
        return string.format("%.14g", val)
    elseif t == "boolean" then
        return val and "true" or "false"
    elseif t == "nil" then
        return "null"
    elseif t == "table" then
        -- Check if array-like
        local is_array = true
        local max_i = 0
        for k, _ in pairs(val) do
            if type(k) ~= "number" or k ~= math.floor(k) or k < 1 then
                is_array = false
                break
            end
            if k > max_i then max_i = k end
        end
        if is_array and max_i == #val then
            local parts = {}
            for i = 1, #val do
                parts[i] = json_encode_value(val[i], depth + 1)
            end
            return "[" .. table.concat(parts, ",") .. "]"
        else
            local parts = {}
            for k, v in pairs(val) do
                parts[#parts + 1] = json_encode_value(tostring(k), depth + 1) .. ":" .. json_encode_value(v, depth + 1)
            end
            return "{" .. table.concat(parts, ",") .. "}"
        end
    else
        return '"[' .. t .. ']"'
    end
end

local function json(tbl)
    return json_encode_value(tbl)
end

-- ─── Route table ────────────────────────────────────────────────────────────

local routes = {}

local function route(method, path, handler)
    routes[#routes + 1] = { method = method, path = path, handler = handler }
end

local function find_route(method, path)
    for _, r in ipairs(routes) do
        if r.method == method and r.path == path then
            return r.handler
        end
    end
    -- Try wildcard method match
    for _, r in ipairs(routes) do
        if r.method == "*" and r.path == path then
            return r.handler
        end
    end
    return nil
end

-- ─── Route definitions ─────────────────────────────────────────────────────

-- GET / — Welcome page
route("GET", "/", function(req)
    return 200, "text/html; charset=utf-8", [[
<!DOCTYPE html>
<html>
<head><title>luars async HTTP server</title></head>
<body>
<h1>Welcome to luars async HTTP server!</h1>
<p>This server is powered by Lua running inside a Rust VM with async support.</p>
<h2>Available endpoints:</h2>
<ul>
    <li><a href="/hello">GET /hello</a> — Hello world</li>
    <li><a href="/time">GET /time</a> — Current server time</li>
    <li><a href="/sleep?seconds=1">GET /sleep?seconds=N</a> — Async sleep demo</li>
    <li><a href="/counter">GET /counter</a> — Per-VM request counter</li>
    <li><a href="/echo">POST /echo</a> — Echo request body</li>
    <li><a href="/file?path=Cargo.toml">GET /file?path=FILE</a> — Async file read</li>
    <li><a href="/compute?n=35">GET /compute?n=N</a> — CPU-bound Fibonacci</li>
    <li><a href="/multi-io">GET /multi-io</a> — Multiple async I/O operations</li>
</ul>
</body>
</html>
]]
end)

-- GET /hello — Simple response
route("GET", "/hello", function(req)
    return 200, "application/json", json({
        message = "Hello from Lua!",
        method = req.method,
        path = req.path,
    })
end)

-- GET /time — Current server time via async time() function
route("GET", "/time", function(req)
    local t = time()
    return 200, "application/json", json({
        unix_timestamp = t,
        formatted = os.date("%Y-%m-%d %H:%M:%S", math.floor(t)),
    })
end)

-- GET /sleep?seconds=N — Demonstrates async sleep
route("GET", "/sleep", function(req)
    local seconds = tonumber(req.query and req.query:match("seconds=([%d%.]+)")) or 1
    if seconds > 10 then seconds = 10 end  -- Cap at 10 seconds

    local t1 = time()
    sleep(seconds)
    local t2 = time()

    return 200, "application/json", json({
        requested_sleep = seconds,
        actual_elapsed = t2 - t1,
        message = string.format("Slept for %.3f seconds (async!)", t2 - t1),
    })
end)

-- Per-VM counter (demonstrates VM isolation)
local request_counter = 0

route("GET", "/counter", function(req)
    request_counter = request_counter + 1
    return 200, "application/json", json({
        count = request_counter,
        message = "Each VM has its own counter (VM isolation demo)",
    })
end)

-- POST /echo — Echo the request body
route("POST", "/echo", function(req)
    return 200, "application/json", json({
        method = req.method,
        path = req.path,
        body = req.body,
        body_length = #req.body,
    })
end)

-- GET /file?path=FILE — Async file reading
route("GET", "/file", function(req)
    local path = req.query and req.query:match("path=([^&]+)")
    if not path then
        return 400, "application/json", json({ error = "Missing 'path' query parameter" })
    end

    -- Security: only allow reading files in current directory
    if path:match("%.%.") or path:match("^/") or path:match("^\\") then
        return 400, "application/json", json({ error = "Path traversal not allowed" })
    end

    local content, err = read_file(path)
    if err then
        return 404, "application/json", json({ error = err, path = path })
    end

    return 200, "text/plain; charset=utf-8", content
end)

-- GET /compute?n=N — CPU-bound Fibonacci (shows that compute doesn't block other workers)
route("GET", "/compute", function(req)
    local n = tonumber(req.query and req.query:match("n=(%d+)")) or 30
    if n > 40 then n = 40 end  -- Cap to avoid extremely long computation

    local function fib(x)
        if x <= 1 then return x end
        return fib(x - 1) + fib(x - 2)
    end

    local t1 = time()
    local result = fib(n)
    local t2 = time()

    return 200, "application/json", json({
        n = n,
        fibonacci = result,
        elapsed_seconds = t2 - t1,
        message = "CPU work runs on a dedicated worker thread",
    })
end)

-- GET /multi-io — Demonstrates multiple async I/O operations in sequence
route("GET", "/multi-io", function(req)
    local results = {}

    -- 1. Get current time
    local t1 = time()
    results[#results + 1] = { op = "time", value = t1 }

    -- 2. Async sleep (short)
    sleep(0.1)
    results[#results + 1] = { op = "sleep", duration = 0.1 }

    -- 3. Try reading a file
    local content, err = read_file("Cargo.toml")
    if content then
        results[#results + 1] = { op = "read_file", file = "Cargo.toml", size = #content }
    else
        results[#results + 1] = { op = "read_file", file = "Cargo.toml", error = err }
    end

    -- 4. Get time again to show total elapsed
    local t2 = time()
    results[#results + 1] = { op = "total_time", elapsed = t2 - t1 }

    return 200, "application/json", json({
        operations = results,
        message = "Multiple async I/O operations completed in sequence",
    })
end)

-- ─── Request handler entry point ────────────────────────────────────────────

--- Called from Rust for each incoming HTTP request.
--- @param method string      HTTP method (GET, POST, etc.)
--- @param path string        Request path (without query string)
--- @param query string|nil   Query string (without leading ?)
--- @param headers_json string JSON-encoded headers
--- @param body string        Request body
--- @return number status_code
--- @return string content_type
--- @return string response_body
function handle_request(method, path, query, headers_json, body)
    -- Build request object
    local req = {
        method = method,
        path = path,
        query = query,
        headers_raw = headers_json,
        body = body or "",
    }

    -- Find matching route
    local handler = find_route(method, req.path)
    if not handler then
        return 404, "application/json", json({
            error = "Not Found",
            method = method,
            path = req.path,
        })
    end

    -- Call handler with error protection
    local ok, status, content_type, resp_body = pcall(handler, req)
    if not ok then
        log("Handler error:", status)  -- 'status' contains the error message on pcall failure
        return 500, "application/json", json({
            error = "Internal Server Error",
            detail = tostring(status),
        })
    end

    return status or 200, content_type or "text/plain", resp_body or ""
end

-- NOTE: Do not call async functions (log, sleep, etc.) at the top level here,
-- because this script is loaded via execute_string (sync). Async functions
-- can only be called from within handle_request (which runs via execute_async).
-- To see load-time messages, use print() instead:
print("[lua] Handler script loaded successfully, routes: " .. #routes)
