# @neyuki/net

Long-lived connections: local pipes and Unix sockets, TCP sockets, WebSocket clients and servers, and UDP sockets. Each connection has a reader thread that fills a buffer, so reading never blocks the interpreter. Inside a task, `read`, `receive`, `write` and the connect functions suspend only that task while they wait (see `task`); in the main chunk they block, like `http.request`. Binary data is passed as a `buffer`, so framing can be done with the `buffer` read and write functions. The streams of a child process started with `os.spawn` are the same kind of connection.

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

## net.listenWebSocket(options: number | table?): WebSocketServer

Opens a WebSocket server. `options` is a port number or a table with `host` (default `"127.0.0.1"`; `"0.0.0.0"` listens on every interface) and `port` (default 8080; 0 picks a free one). Raises when the address is taken. The server's `host` and `port` fields hold the address it listens on (`port` is the one picked when 0 was asked for), and `url` is `ws://host:port`. Clients connect over plain `ws://`; for `wss://`, put the server behind a TLS proxy.

Opening handshakes are answered on the server's own threads, so a client is ready, and its messages are already arriving, by the time `accept` returns it. Requests that are not WebSocket upgrades are refused with `400`, and clients asking for a protocol version other than 13 with `426`.

```lua
local server = net.listenWebSocket({ port = 9000 })
print("listening on " .. server.url)

-- Accept in a task too: the main chunk blocking in accept would keep the
-- scheduler from running the client tasks.
task.spawn(function()
    while true do
        local ws = server:accept()
        task.spawn(function()
            while true do
                local message = ws:receive()
                if message == nil then
                    break -- the client hung up
                end
                ws:send(message) -- echo it back, text as text and binary as binary
            end
        end)
    end
end)
```

## net.udp(options: number | table?): UdpSocket

Opens a UDP socket. `options` is a port number or a table with `host` (default `"0.0.0.0"`, every IPv4 interface; `"::"` for IPv6), `port` (default 0, a free one) and `broadcast` (true to allow sending to broadcast addresses such as `255.255.255.255`). The socket's `host` and `port` fields hold the local address it is bound to. Datagrams are queued by a reader thread as they arrive; up to 4096 wait for `receive`, and further ones are dropped, as a full socket buffer would.

```lua
local socket = net.udp()
socket:sendTo("ping", "127.0.0.1", 9999)
local reply, host, port = socket:receive(2)
if reply == nil then
    print("no answer within 2 seconds")
else
    print(host, port, buffer.tostring(reply))
end
```

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

False after `close` and once the other end has closed the connection.

## ws:close()

Sends a close frame and shuts the connection down.

## server:accept(timeout: number?): WebSocket?

Waits for the next client and returns it as a WebSocket with the same `send`, `receive`, `isOpen` and `close` as a client connection, plus:

- `path`: the request target, such as `"/chat?room=1"`
- `headers`: the request headers, with lower-case names (repeated headers are joined with `", "`)
- `remote`: the client's address and port, such as `"127.0.0.1:52144"`

Returns nil once `timeout` seconds pass without a client (0 only checks); with no `timeout` it waits as long as it takes. Raises once the server is closed.

## server:close()

Stops listening and drops the clients nobody accepted yet; accepted ones stay open. Closing twice is fine.

## server:isOpen(): boolean

False after `close`.

## udp:sendTo(data: string | buffer, host: string, port: number): number

Sends `data` as one datagram to `host:port` and returns its size. `host` may be a name, which is resolved to an address of the socket's family (IPv4 or IPv6). A datagram holds at most 65,507 bytes over IPv4; keep them under about 1,200 bytes to avoid fragmentation on the internet.

## udp:connect(host: string, port: number)

Sets the peer that `send` goes to. From then on the socket only receives datagrams from that peer. Nothing is sent: UDP has no connection to set up.

## udp:send(data: string | buffer): number

Sends `data` as one datagram to the peer set by `connect`; returns its size. Raises when `connect` was not called.

```lua
local socket = net.udp()
socket:connect("127.0.0.1", 9999)
socket:send(buffer.fromhex("01ff"))
```

## udp:receive(timeout: number?): buffer?, string, number

Returns the next datagram as a buffer, followed by the sender's address and port. Returns nil once `timeout` seconds pass without one (0 only checks); with no `timeout` it waits as long as it takes. Raises once the socket is closed.

## udp:joinMulticast(group: string, interface: string?)

Joins the multicast `group` (such as `"239.255.0.1"` or `"ff02::1"`) so the socket receives datagrams sent to it; bind the socket to the group's port. `interface` is the local IPv4 address of the network interface to join on (default: the system's choice); IPv6 groups are joined on the default interface.

```lua
local socket = net.udp({ port = 5353 })
socket:joinMulticast("224.0.0.251")
```

## udp:leaveMulticast(group: string, interface: string?)

Leaves a group joined with `joinMulticast`.

## udp:close()

Closes the socket; closing twice is fine.

## udp:isOpen(): boolean

False after `close`.
