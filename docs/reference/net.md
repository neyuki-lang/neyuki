# @neyuki/net

Long-lived connections: local pipes and Unix sockets, TCP sockets and a WebSocket client. Each connection has a reader thread that fills a buffer, so reading never blocks the interpreter. Inside a task, `read`, `receive`, `write` and the connect functions suspend only that task while they wait (see `task`); in the main chunk they block, like `http.request`. Binary data is passed as a `buffer`, so framing can be done with the `buffer` read and write functions. The streams of a child process started with `os.spawn` are the same kind of connection.

```lua
local net = require("@neyuki/net")

local conn = net.connect("127.0.0.1", 6463)
conn:write("hello\n")
local reply = conn:read(5)
print(buffer.tostring(reply))
conn:close()
```

## net.connect(host: string, port: number, timeout: number?): Connection

Opens a TCP connection. `timeout` is how many seconds to wait for it (default 10). Raises when the host cannot be resolved or the connection is refused.

## net.connectPipe(name: string, timeout: number?): Connection

Opens a local pipe. On Windows `name` is a named pipe, `\.\pipe\<name>` (a name that already starts with `\` is used as given); elsewhere it is the path of a Unix domain socket. `timeout` is how many seconds to wait for a busy Windows pipe (default 10).

```lua
local path = os.getenv("XDG_RUNTIME_DIR") or ""
local conn = net.connectPipe(os.getenv("OS") and "discord-ipc-0" or path .. "/discord-ipc-0")
```

## net.connectUnix(path: string): Connection

Opens the Unix domain socket at `path`. Raises on Windows, where `connectPipe` is the equivalent.

## net.websocket(url: string, options: table?): WebSocket

Opens a WebSocket client connection to a `ws://` or `wss://` URL (TLS through the operating system's library). `options` may hold `headers`, a `{ name = value }` table sent with the upgrade request, and `timeout` in seconds (default 10). Pings are answered automatically.

## net.spawn(command: string | {string}, options: table?): Process

The same as `os.spawn`.

## conn:read(n: number): buffer?

Returns a buffer of exactly `n` bytes, waiting for them to arrive. If the peer hangs up first, returns what arrived, and nil once nothing is left. Raises when the connection broke or was closed. `n` may be at most 64 MiB.

```lua
local header = conn:read(8)
if header == nil then
    return -- the peer hung up
end
local opcode = buffer.readu32(header, 0)
local length = buffer.readu32(header, 4)
```

## conn:readAvailable(): buffer

Whatever has arrived and was not read yet, without waiting. The buffer is empty when nothing has arrived; check `atEof` to tell a quiet connection from a closed one.

## conn:write(data: string | buffer): number

Sends all of `data` and returns how many bytes that was. Raises when the peer is gone, and on a stream that cannot be written to (the output of a child process).

## conn:atEof(): boolean

True once the peer has hung up and everything it sent has been read.

## conn:isOpen(): boolean

False after `close` and once the peer has hung up. To reconnect when a service restarts, check this (or watch for nil from `read`), then connect again.

## conn:close()

Closes the connection; closing twice is fine. Closing the `stdin` of a child process tells it that input is over.

## ws:send(data: string | buffer): number

Sends a text message for a string and a binary message for a buffer; returns the payload size.

## ws:receive(): string | buffer, string

Waits for the next message and returns it with its kind: a string and `"text"`, or a buffer and `"binary"`. Returns nil once the connection has closed and every message was read. Raises when the connection broke.

```lua
local ws = net.websocket("ws://127.0.0.1:9222/devtools/page/1")
ws:send('{"id":1,"method":"Page.enable"}')
local message, kind = ws:receive()
print(kind, message)
ws:close()
```

## ws:isOpen(): boolean

False after `close` and once the server has closed the connection.

## ws:close()

Sends a close frame and shuts the connection down.
