local request_count = 0

local function json_escape(value)
    return tostring(value):gsub('\\', '\\\\'):gsub('"', '\\"')
end

local function json_object(fields)
    local parts = {}
    for index = 1, #fields do
        local item = fields[index]
        parts[index] = '"' .. item[1] .. '":' .. item[2]
    end
    return '{' .. table.concat(parts, ',') .. '}'
end

local function text_response(body)
    return 200, "text/plain; charset=utf-8", body
end

local function json_response(fields)
    return 200, "application/json; charset=utf-8", json_object(fields)
end

local routes = {
    ["GET /"] = function(req)
        request_count = request_count + 1
        return text_response("hello from sandboxed high-level luars")
    end,

    ["GET /health"] = function(req)
        return json_response({
            { "status", '"ok"' },
            { "requests", tostring(request_count) },
        })
    end,

    ["GET /hello"] = function(req)
        local name = req.query:match("name=([^&]+)") or "world"
        return text_response("hello, " .. name)
    end,

    ["GET /time"] = function(req)
        local now = time()
        return json_response({
            { "unix_seconds", string.format("%.3f", now) },
            { "worker_requests", tostring(request_count) },
        })
    end,

    ["GET /sleep"] = function(req)
        local seconds = tonumber(req.query:match("seconds=([%d%.]+)")) or 0.05
        if seconds > 2 then
            seconds = 2
        end
        sleep(seconds)
        return text_response(string.format("slept for %.3f seconds", seconds))
    end,

    ["GET /file"] = function(req)
        local path = req.query:match("path=([^&]+)") or "README.md"
        if path:match("%.%.") or path:match("^/") or path:match("^\\") then
            return 400, "text/plain; charset=utf-8", "path traversal is not allowed"
        end

        local content, err = read_file(path)
        if err then
            return 404, "text/plain; charset=utf-8", err
        end

        return 200, "text/plain; charset=utf-8", content
    end,
}

return function(method, path, query, headers_json, body)
    local request = {
        method = method,
        path = path,
        query = query or "",
        headers_json = headers_json or "{}",
        body = body or "",
    }

    local handler = routes[method .. " " .. path]
    if not handler then
        return 404, "application/json; charset=utf-8", json_object({
            { "error", '"not found"' },
            { "path", '"' .. json_escape(path) .. '"' },
        })
    end

    local ok, status, content_type, response_body = pcall(handler, request)
    if not ok then
        log("handler failure: " .. tostring(status))
        return 500, "application/json; charset=utf-8", json_object({
            { "error", '"internal server error"' },
            { "detail", '"' .. json_escape(status) .. '"' },
        })
    end

    return status, content_type, response_body
end
