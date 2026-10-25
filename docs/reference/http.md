# @neyuki/http

An HTTP/1.1 client and server, with TLS through the operating system's own library. Bodies are strings; response bytes that are not valid UTF-8 are replaced with U+FFFD. The interpreter is single-threaded, so a server and its client can share a script only when the request runs in a task, where it waits on a background thread instead of blocking (see `task`). Responses are decoded when they are gzip or brotli compressed.

```lua
local http = require("@neyuki/http")

local response = http.get("https://api.github.com/repos/neyuki-lang/neyuki", {
    headers = { ["user-agent"] = "neyuki" },
    timeout = 10,
})
if response.ok then
    print(json.decode(response.body).stargazers_count)
end
```

## http.request(url: string, options: table?): Response

Performs one request. `options` may hold `method` (default `"GET"`), `headers` (a `{ name = value }` table), `body`, `timeout` (seconds for the whole exchange; unlimited by default), `follow` (whether to follow redirects; default `true`) and `maxBody` (the largest response body accepted, default 64 MiB).

Only transport failures (DNS, connection, TLS, timeout) raise errors. Every status code comes back as a `Response` with `status`, `ok` (true for 2xx), `headers` (names lower-cased), `body`, `url` (after redirects) and `header(name)`, a case-insensitive lookup.

## http.get(url: string, options: table?): Response

A `GET` request.

## http.head(url: string, options: table?): Response

A `HEAD` request.

## http.delete(url: string, options: table?): Response

A `DELETE` request.

## http.post(url: string, body: string?, options: table?): Response

A `POST` request with `body`.

```lua
http.post("https://example.com/api/items", json.encode({ name = "tea" }), {
    headers = { ["content-type"] = "application/json" },
})
```

## http.put(url: string, body: string?, options: table?): Response

A `PUT` request with `body`.

## http.patch(url: string, body: string?, options: table?): Response

A `PATCH` request with `body`.

## http.encode(s: string): string

Percent-encodes a string for use in a URL.

## http.decode(s: string): string

Reverses `encode`.

## http.query(params: table): string

Builds an `a=1&b=2` query string from a table, sorted by key.

```lua
http.get("https://example.com/search?" .. http.query({ q = "snow", page = 2 }))
```

## http.parseQuery(target: string): (string, table)

Splits a `path?query` target into the path and a table of decoded parameters.

## http.listen(options: number | table?): Server

Binds a server. `options` is a port or `{ host?, port?, maxBody? }`, defaulting to `127.0.0.1`, port `8080` and 64 MiB; port `0` picks a free one. A request whose body is over `maxBody` is answered with 413 and never returned.

The `Server` has `host`, `port`, `url`, `accept(timeout?)` (the next `Request`, or `nil` once `timeout` seconds pass without one), `isOpen()` and `close()`.

A `Request` has `method`, `path`, `headers`, `body`, `remote`, `header(name)`, `query()` (the path and its decoded parameters), `forward(target, options?)` and `respond(reply)`. `respond` may be called once, with `nil` (204), a string (200, `text/plain`) or `{ status?, headers?, body? }`.

## http.serve(options: number | table?, handler: (Request) -> any): never

Runs `listen` and calls `handler(request)` for every request, forever, sending its return value with `respond` unless the handler already answered. A handler error is printed to standard error and answered with a 500.

```lua
http.serve(8080, function(request)
    local path, params = request:query()
    if path == "/hello" then return "hello " .. (params.name ?? "world") end
    if path == "/api/time" then
        return { headers = { ["content-type"] = "application/json" }, body = json.encode({ now = os.time() }) }
    end
    return { status = 404, body = "not found" }
end)
```

## http.forward(options: number | table?, target: number | table): Forward

A TCP port forward: binds `options` (as for `listen`) and relays every connection to `target` (a local port or `{ host?, port }`) byte for byte, so HTTP, WebSockets and TLS all pass through. Relaying runs on background threads. The `Forward` has `host`, `port`, `url`, `target`, `isOpen()`, `close()` and `wait(timeout?)`, which blocks for `timeout` seconds or for as long as the forward runs.

```lua
http.forward(1234, 5173):wait() -- expose a dev server on port 1234
```

`request:forward(target, options?)` answers a single request with whatever `target` (a port, `{ host?, port }` or a base URL) replies, passing the method, path, headers and body through unchanged; `options.timeout` bounds the upstream call. If the upstream cannot be reached the client gets a 502 and the error is raised. WebSocket upgrades need `http.forward` instead.
