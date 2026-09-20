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

`try(function, ...)` returns a leading boolean followed by the function result or an error message. The bundled `@neyuki/crypto`, `@neyuki/fs`, `@neyuki/http`, `@neyuki/io`, `@neyuki/math`, `@neyuki/string` and `@neyuki/table` modules can be loaded with `require`.

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

Two functions hand traffic to another server. `forward(options?, target)` is a TCP port forward: it binds `options` (as for `listen`) and relays every connection to `target` (a port on `localhost`, which reaches a server bound to either `127.0.0.1` or `::1`, or a `{ host?, port }` table) byte for byte, so HTTP, WebSockets, TLS and anything else that speaks TCP pass through; a Vite dev server on 5173 becomes reachable on 1234 with `http.forward(1234, 5173)`, hot reloading included. The relaying runs on background threads and returns a `Forward` with `host`, `port`, `url`, `target`, `isOpen()`, `close()` and `wait(timeout?)`, which blocks for `timeout` seconds or, without one, for as long as the forward runs, so a script that only forwards stays alive. A `Request` also has `forward(target, options?)`, which answers that one request with whatever `target` (a port, a `{ host?, port }` table or a base URL such as `"https://api.example.com"`) replies: the method, path, headers and body go upstream and the status, headers and body come back unchanged, redirects included, so a handler can serve a few routes itself and `return request:forward(5173)` for the rest. `options.timeout` bounds the upstream call. If the upstream cannot be reached the client receives a 502 and the error is raised, which `serve` reports on stderr. WebSocket upgrades do not pass through `request:forward` (the handshake is answered like a plain request, so a client such as Vite's falls back to connecting directly); use `http.forward` for those.

```lua
local http = require("@neyuki/http")
-- everything on 1234 goes straight to the dev server, sockets included...
local dev = http.forward(1234, 5173)
-- ...while 8080 answers /api itself and proxies the rest
http.serve(8080, function(request: Request): any
    if (request.path == "/api/time") then return tostring(os.time()) end
    return request:forward(5173)
end)
```

`@neyuki/crypto` is built on the RustCrypto and dalek crates rather than Neyuki code, so keys do not leak through timing and bulk encryption runs at native speed. Binary values are strings in an *encoding*, `"hex"`, `"base64"` or `"base64url"`: digests default to hex and everything else (keys, ciphertexts, signatures, random bytes) to base64. Asymmetric keys are PEM strings (PKCS#8 private, SPKI public) that OpenSSL and other languages read; PKCS#1 `RSA PRIVATE KEY` files are accepted too. `encode(s, encoding)` and `decode(s, encoding)` convert; `decode` errors if the bytes are not UTF-8 text, so keys and ciphertexts should stay encoded.

- `hash(s, algorithm?, encoding?)` digests `s` with `sha256` (default), `sha224`, `sha384`, `sha512`, `sha3-256`, `sha3-512`, `blake2b`, `blake2s`, `sha1` or `md5`. `hmac(s, key, algorithm?, encoding?)` is the keyed version (no blake2). `equals(a, b)` compares in constant time, for MACs and tokens.
- `hashPassword(password, options?)` produces a salted, self-describing hash with `argon2id` (default; `cost`, `memory` in KiB and `parallelism` default to 2, 19456 and 1) or `bcrypt` (`cost` defaults to 12; passwords over 72 bytes are refused). `verifyPassword(password, hash)` checks either kind and returns `false` on a mismatch. `pbkdf2(password, salt, options?)` (`algorithm`, `iterations` default 600000, `length` default 32, `encoding`) and `hkdf(key, options?)` (`algorithm`, `salt`, `info`, `length`, `encoding`, which applies to `key` too) derive keys.
- `generateKey(algorithm?, options?)` makes a key for `aes-256-gcm` (default), `aes-128-gcm`, `chacha20-poly1305` or `xchacha20-poly1305`. `encrypt(s, key, options?)` seals `s` with a fresh random nonce and returns `nonce || ciphertext || tag`; `decrypt(s, key, options?)` reverses it and errors on a wrong key or tampered data. `options` may hold `algorithm`, `aad` (data that is authenticated but not encrypted, which `decrypt` must be given again) and `encoding` (of the key and the ciphertext). Only authenticated ciphers are offered.
- `generateKeyPair(algorithm?, options?)` returns `{ publicKey, privateKey }` for `ed25519` (default), `x25519` or `rsa` (`options.bits` defaults to 2048). `publicKey(privateKey)` extracts the public half. `sign(s, privateKey, options?)` and `verify(s, signature, publicKey, options?)` use Ed25519 or RSA, where `options` may set `hash` (default `sha256`) and `padding` (`pss`, the default, or `pkcs1`); `verify` returns `false` for a bad or malformed signature and accepts a private key in place of the public one. `publicEncrypt(s, publicKey, options?)` and `privateDecrypt(s, privateKey, options?)` are RSA-OAEP for short messages such as a symmetric key. `sharedSecret(privateKey, publicKey, options?)` is X25519 key agreement; run the result through `hkdf` with an `info` before using it as a key.
- `randomBytes(n, encoding?)`, `randomInt(n)` / `randomInt(min, max)` (inclusive, like `math.random`) and `uuid()` draw from the operating system's secure generator.

```lua
local crypto = require("@neyuki/crypto")

-- storing and checking a password
local stored = crypto.hashPassword("correct horse battery staple")
assert(crypto.verifyPassword("correct horse battery staple", stored))

-- encrypting a record at rest, bound to its id
local key = crypto.generateKey()
local sealed = crypto.encrypt("card ending 4242", key, { aad = "user:42" })
print(crypto.decrypt(sealed, key, { aad = "user:42" }))

-- signing a token that another service verifies with the public key
local keys = crypto.generateKeyPair("ed25519")
local token = crypto.encode("user:42", "base64url")
local signature = crypto.sign(token, keys.privateKey, { encoding = "base64url" })
assert(crypto.verify(token, signature, keys.publicKey, { encoding = "base64url" }))
```

## Current boundaries

The compiler currently validates syntax rather than enforcing the declared type annotations. Runtime modules are resolved from the project checkout, and the base `require` implementation recognizes only the bundled modules listed above. User-defined functions return their explicit `return` values; a function without a return produces `nil` when called.