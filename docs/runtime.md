# Runtime and tools

## Commands

Run these commands from the `neyuki/` directory:

```text
cargo run -- lint examples/hello.nyk
cargo run -- compile examples/hello.nyk
cargo run -- run examples/hello.nyk
cargo run -- disasm examples/hello.nyk
cargo run -- test
```

`lint` prints tokens and checks parsing. `compile` compiles a file to `.nykb` bytecode. `run` compiles the file (or loads a `.nykb`) and executes it on the register VM. `disasm` prints the bytecode the compiler emits, and `dump-ir` the intermediate representation before and after optimization. `test` runs every `.nyk` file in `tests/`.

Programs go through the optimizing IR pipeline by default. `run --tree-walker` executes the file on the original AST interpreter instead, and `compile --direct` uses the older single-pass bytecode compiler; both exist for comparison and debugging.

## Builtins

The base runtime provides `print`, `tostring`, `type`, `typeof`, `assert`, `int`, `float`, `try`, and `require`. `type` groups integers and floats as `number`; `typeof` reports `bigint` for arbitrary-precision integers and `float` for floating-point values.

`try(function, ...)` returns a leading boolean followed by the function result or an error message. The bundled `@neyuki/fs`, `@neyuki/http`, `@neyuki/io`, `@neyuki/math`, `@neyuki/string` and `@neyuki/table` modules can be loaded with `require`.

```lua
local math = require("@neyuki/math")
print(math.max(2, 7, 4))
```

`@neyuki/string` follows Lua's string library: `byte`, `char`, `count` (codepoints; `#s` counts bytes), `find`, `format`, `gmatch`, `gsub`, `len`, `lower`, `match`, `pack`, `packsize`, `rep`, `reverse`, `split`, `sub`, `unpack` and `upper`. Indices are byte positions and patterns use Lua pattern syntax. `pack` returns one character per byte (codes 0-255) so binary data fits in a string; `unpack` expects the same encoding.

```lua
local string = require("@neyuki/string")
for key, value in string.gmatch("a=1, b=2", "(%w+)=(%w+)") do
    print(string.format("%s -> %d", key, value))
end
```

`@neyuki/fs` exposes the filesystem: `open(path, mode?)` returns a `File` (modes `"r"`, `"w"` and `"a"`; the default is `"r"` and a missing file is an error), plus `exists`, `remove` (files and empty directories), `rename`, `mkdir(path, recursive?)`, `list` (sorted entry names) and `isDir`. A `File` has a read-only `path` and the methods `read()`, `write(text, append?)` (replaces the contents unless `append` is true; files opened with `"a"` always append), `lines()` (an iterator yielding one line at a time) and `close()`.

```lua
local fs = require("@neyuki/fs")
local file = fs.open("notes.txt", "w")
file:write("one
two
")
file:close()
for line in fs.open("notes.txt"):lines() do
    print(line)
end
```

`@neyuki/io` talks to the terminal. `read(...formats)` reads from stdin, one value per format: `"l"` (a line without its newline, the default), `"L"` (a line with its newline), `"a"` (everything remaining), `"n"` (a whitespace-delimited number) or a count of characters; every format yields `nil` once input is exhausted. `lines(format?)` returns an iterator that reads with `format` until input runs out, `write(...)` prints strings and numbers to stdout with no separators or trailing newline (and returns the stream so calls chain), `flush()` flushes stdout and `prompt(text?, format?)` writes `text`, flushes and reads one value. The streams `io.stdin`, `io.stdout` and `io.stderr` expose the same operations as methods (`stdin:read`, `stdin:lines`, `stdout:write`, `stderr:flush`, ...) plus `isTerminal()`, which reports whether the stream is attached to a terminal rather than a pipe or file.

```lua
local io = require("@neyuki/io")
local name = io.prompt("What is your name? ")
io.write("Hello, ", name, "!
")
local total = 0
for value in io.lines("n") do
    total += value
end
io.stderr:write("sum: ", total, "
")
```

`@neyuki/http` is an HTTP/1.1 client and server. `request(url, options?)` performs one request, where `options` may hold `method` (default `"GET"`), `headers` (a `{ name = value }` table), `body`, `timeout` (seconds for the whole exchange; unlimited by default) and `follow` (whether redirects are followed; `true` by default). The shorthands `get`, `head` and `delete` take `(url, options?)` and `post`, `put` and `patch` take `(url, body?, options?)`. Only transport failures (DNS, connection, TLS, timeout) raise errors; every status code comes back as a `Response` with `status`, `ok` (true for 2xx), `headers` (names lower-cased), `body`, `url` (after redirects) and `header(name)` for a case-insensitive lookup. `encode` and `decode` percent-encode a string, `query(params)` builds a sorted `a=1&b=2` query string and `parseQuery(target)` splits a `path?query` target into the path and a decoded parameter table. Bodies are strings; responses that are not valid UTF-8 have those bytes replaced with U+FFFD.

```lua
local http = require("@neyuki/http")
local response = http.get("https://httpbin.org/get?" .. http.query({ q = "neyuki" }), { timeout = 10 })
if (response.ok) then
    print(response:header("content-type"), response.body)
end
```

`listen(options?)` binds a server (`options` is a port number or `{ host?, port? }`; the defaults are `127.0.0.1` and `8080`, and port `0` picks a free one) and returns a `Server` with `host`, `port`, `url`, `accept(timeout?)` (the next `Request`, or `nil` once `timeout` seconds pass without one), `isOpen()` and `close()`. A `Request` has `method`, `path`, `headers`, `body`, `remote`, `header(name)`, `query()` (the path and decoded query parameters) and `respond(reply)`, which may be called once with `nil` (204), a string (200, `text/plain`) or `{ status?, headers?, body? }`. `serve(options?, handler)` runs `listen` and then calls `handler(request)` for every request forever, sending its return value with `respond` unless the handler already answered; a handler error is reported on stderr and answered with a 500. The runtime is single-threaded, so a script cannot serve and request itself at the same time.

```lua
local http = require("@neyuki/http")
http.serve(8080, function(request: Request): any
    local path, params = request:query()
    if (path == "/hello") then return "hello " .. (params.name ?? "world") end
    return { status = 404, body = "not found" }
end)
```

## Current boundaries

The compiler currently validates syntax rather than enforcing the declared type annotations. Runtime modules are resolved from the project checkout, and the base `require` implementation recognizes only the bundled modules listed above. User-defined functions return their explicit `return` values; a function without a return produces `nil` when called.