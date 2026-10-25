//! Long-lived connections for `@neyuki/net`, and `os.spawn`.
//!
//! A connection is a local pipe (a Windows named pipe or a Unix domain
//! socket), a TCP socket, a WebSocket client or the standard streams of a
//! child process. Each one is backed by a reader thread that fills a shared
//! buffer, so the interpreter thread never waits on the network: `read` and
//! `receive` return at once when enough data is buffered, and otherwise wait
//! on a worker thread through `background::run`, which suspends only the
//! calling task. Writes and connects go through `background::run` too.
//! Outside a task all of them simply block, as `http.request` does.
//!
//! The objects are tables built here, with native methods that find their
//! connection by the hidden `_id` field, so `os.spawn` needs no `.nyk`
//! wrapper; `lib/net.nyk` validates arguments for the connect functions.
//! Dropping the connection (`close`) shuts the socket down, which ends the
//! reader thread.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rand::RngCore;
use sha1::{Digest, Sha1};

use crate::vm::libs::args::{frozen, number};
use crate::vm::machine::VM;
use crate::vm::value::{NativeDef, Value, VmTable};

pub(crate) const NATIVES: &[NativeDef] = &[
    NativeDef {
        name: "__net_connect",
        func: builtin_connect,
    },
    NativeDef {
        name: "__net_connect_pipe",
        func: builtin_connect_pipe,
    },
    NativeDef {
        name: "__net_connect_unix",
        func: builtin_connect_unix,
    },
    NativeDef {
        name: "__net_websocket",
        func: builtin_websocket,
    },
];

/// The most data held for one connection before its reader waits for the
/// script to catch up; also the largest `read` and WebSocket message.
const MAX_BUFFER: usize = 64 * 1024 * 1024;

/// How long a connect or a WebSocket handshake may take unless the script
/// says otherwise.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Waiting threads re-check their connection this often, so closing one
/// always wakes them.
const WAKE: Duration = Duration::from_millis(200);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

// --- shared connection state ------------------------------------------------

#[derive(Default)]
struct State {
    /// Bytes received on a stream connection and not yet read.
    buf: VecDeque<u8>,
    /// Messages received on a WebSocket: (is text, payload).
    messages: VecDeque<(bool, Vec<u8>)>,
    /// The peer hung up, or reading failed (then `error` says why).
    eof: bool,
    error: Option<String>,
    /// `close` was called here.
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// Nothing is ever read (a child's stdin), so end of stream is not
    /// the connection ending.
    write_only: bool,
}

impl Shared {
    fn new() -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            write_only: false,
        })
    }

    /// Records that reading is over, for `error` if it was not a clean hangup.
    fn finish(&self, error: Option<String>) {
        let mut state = lock(&self.state);
        state.eof = true;
        if state.error.is_none() && !state.closed {
            state.error = error;
        }
        self.wake.notify_all();
    }

    /// Blocks until `ready` holds or the connection is closed or hung up.
    fn wait_for(&self, ready: impl Fn(&State) -> bool) -> MutexGuard<'_, State> {
        let mut state = lock(&self.state);
        while !(state.closed || state.eof || ready(&state)) {
            state = self
                .wake
                .wait_timeout(state, WAKE)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        state
    }
}

type Writer = Arc<Mutex<Option<Box<dyn Write + Send>>>>;

struct Conn {
    shared: Arc<Shared>,
    writer: Writer,
    websocket: bool,
    /// Shuts the transport down so the reader thread ends.
    closer: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        lock(&self.shared.state).closed = true;
        self.shared.wake.notify_all();
        if let Some(closer) = self.closer.take() {
            closer();
        }
        lock(&self.writer).take();
    }
}

struct Process {
    child: Mutex<Child>,
    status: Mutex<Option<i64>>,
}

impl Process {
    /// The exit code once the process has ended; -1 when a signal ended it.
    fn poll(&self) -> Result<Option<i64>, String> {
        let mut status = lock(&self.status);
        if status.is_some() {
            return Ok(*status);
        }
        let exited = lock(&self.child)
            .try_wait()
            .map_err(|err| format!("cannot query process: {}", err))?;
        *status = exited.map(|s| s.code().map_or(-1, i64::from));
        Ok(*status)
    }
}

thread_local! {
    static CONNS: RefCell<HashMap<u64, Conn>> = RefCell::new(HashMap::new());
    static PROCESSES: RefCell<HashMap<u64, Arc<Process>>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

fn next_id() -> u64 {
    NEXT_ID.with(|id| {
        let value = id.get();
        id.set(value + 1);
        value
    })
}

fn register(conn: Conn) -> u64 {
    let id = next_id();
    CONNS.with(|conns| conns.borrow_mut().insert(id, conn));
    id
}

/// What a method needs of connection `id` without borrowing the registry
/// while it works.
fn parts(id: u64) -> Result<(Arc<Shared>, Writer, bool), String> {
    CONNS.with(|conns| {
        conns
            .borrow()
            .get(&id)
            .map(|c| (c.shared.clone(), c.writer.clone(), c.websocket))
            .ok_or_else(|| "connection is closed".to_string())
    })
}

// --- argument helpers -------------------------------------------------------

fn self_id(args: &[Value]) -> Result<u64, String> {
    match args.first() {
        Some(Value::Table(table)) => match table.borrow().get_str("_id") {
            Value::Int(id) => Ok(id as u64),
            _ => Err("expected an object (call methods with ':')".to_string()),
        },
        _ => Err("expected an object (call methods with ':')".to_string()),
    }
}

fn string_arg(args: &[Value], index: usize, name: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(value)) => Ok(value.to_string()),
        Some(value) => Err(format!(
            "{} must be a string, got {}",
            name,
            value.type_name()
        )),
        None => Err(format!("{} must be provided", name)),
    }
}

fn timeout_arg(args: &[Value], index: usize) -> Result<Duration, String> {
    match args.get(index) {
        None | Some(Value::Nil) => Ok(DEFAULT_TIMEOUT),
        Some(value) => {
            let seconds = number(value).map_err(|_| "timeout must be a number".to_string())?;
            if !seconds.is_finite() || seconds <= 0.0 {
                return Err("timeout must be a positive number of seconds".to_string());
            }
            Ok(Duration::from_secs_f64(seconds.min(86_400.0)))
        }
    }
}

/// Bytes of a string or buffer argument.
fn data_arg(args: &[Value], index: usize, name: &str) -> Result<(Vec<u8>, bool), String> {
    match args.get(index) {
        Some(Value::String(s)) => Ok((s.as_bytes().to_vec(), false)),
        Some(Value::Buffer(b)) => Ok((b.borrow().data.clone(), true)),
        Some(other) => Err(format!(
            "{} must be a string or buffer, got {}",
            name,
            other.type_name()
        )),
        None => Err(format!("{} must be provided", name)),
    }
}

// --- object tables ----------------------------------------------------------

fn stream_table(id: u64) -> Value {
    let mut table = VmTable::new();
    table.set_str("_id", Value::Int(id as i64));
    table.set_str("read", crate::native!("net.read", conn_read));
    table.set_str(
        "readAvailable",
        crate::native!("net.readAvailable", conn_read_available),
    );
    table.set_str("write", crate::native!("net.write", conn_write));
    table.set_str("atEof", crate::native!("net.atEof", conn_at_eof));
    table.set_str("close", crate::native!("net.close", conn_close));
    table.set_str("isOpen", crate::native!("net.isOpen", conn_is_open));
    frozen(table)
}

fn websocket_table(id: u64) -> Value {
    let mut table = VmTable::new();
    table.set_str("_id", Value::Int(id as i64));
    table.set_str("send", crate::native!("net.send", ws_send));
    table.set_str("receive", crate::native!("net.receive", ws_receive));
    table.set_str("close", crate::native!("net.close", conn_close));
    table.set_str("isOpen", crate::native!("net.isOpen", conn_is_open));
    frozen(table)
}

// --- methods ----------------------------------------------------------------

/// Removes up to `n` buffered bytes; `Ok(None)` is end of stream.
fn take_bytes(state: &mut State, n: usize) -> Result<Option<Vec<u8>>, String> {
    if state.closed {
        return Err("connection is closed".to_string());
    }
    if state.buf.len() >= n {
        return Ok(Some(state.buf.drain(..n).collect()));
    }
    if !state.buf.is_empty() {
        return Ok(Some(state.buf.drain(..).collect()));
    }
    match state.error.take() {
        Some(error) => Err(error),
        None => Ok(None),
    }
}

fn bytes_result(vm: &mut VM, data: Option<Vec<u8>>) -> Result<Vec<Value>, String> {
    match data {
        Some(data) => crate::vm::libs::buffer::new_buffer(vm, data),
        None => Ok(vec![Value::Nil]),
    }
}

/// `conn:read(n)`: a buffer of `n` bytes, fewer if the peer hangs up first,
/// or nil at end of stream. Suspends the task until the bytes arrive.
fn conn_read(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = self_id(args)?;
    let n = crate::vm::libs::args::int_arg(args, 1, "n")?;
    if n < 0 || n as usize > MAX_BUFFER {
        return Err(format!("n must be between 0 and {}", MAX_BUFFER));
    }
    let n = n as usize;
    let (shared, _, websocket) = parts(id)?;
    if websocket {
        return Err("use receive on a WebSocket".to_string());
    }
    {
        let mut state = lock(&shared.state);
        if state.closed || state.eof || state.buf.len() >= n {
            let data = take_bytes(&mut state, n)?;
            drop(state);
            return bytes_result(vm, data);
        }
    }
    crate::vm::libs::background::run(
        vm,
        move || {
            let mut state = shared.wait_for(|s| s.buf.len() >= n);
            take_bytes(&mut state, n)
        },
        |vm, data| bytes_result(vm, data),
    )
}

/// `conn:readAvailable()`: whatever is buffered now, an empty buffer when
/// nothing is.
fn conn_read_available(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = self_id(args)?;
    let (shared, _, _) = parts(id)?;
    let data: Vec<u8> = lock(&shared.state).buf.drain(..).collect();
    crate::vm::libs::buffer::new_buffer(vm, data)
}

/// `conn:write(data)`: sends a string or buffer; returns the byte count.
fn conn_write(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = self_id(args)?;
    let (data, _) = data_arg(args, 1, "data")?;
    let (_, writer, _) = parts(id)?;
    write_all(vm, writer, data)
}

fn write_all(vm: &mut VM, writer: Writer, data: Vec<u8>) -> Result<Vec<Value>, String> {
    crate::vm::libs::background::run(
        vm,
        move || {
            let mut guard = lock(&writer);
            let out = guard.as_mut().ok_or("connection is not writable")?;
            out.write_all(&data)
                .and_then(|()| out.flush())
                .map_err(|err| format!("write failed: {}", err))?;
            Ok(data.len())
        },
        |_vm, written| Ok(vec![Value::Int(written as i64)]),
    )
}

fn conn_at_eof(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let (shared, _, _) = parts(self_id(args)?)?;
    let state = lock(&shared.state);
    Ok(vec![Value::Bool(
        state.eof && state.buf.is_empty() && state.messages.is_empty(),
    )])
}

/// `conn:close()`; closing twice is fine.
fn conn_close(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = self_id(args)?;
    let conn = CONNS.with(|conns| conns.borrow_mut().remove(&id));
    drop(conn);
    Ok(vec![])
}

/// Open until `close`, and until the peer hangs up.
fn conn_is_open(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let open = match parts(self_id(args)?) {
        Ok((shared, _, _)) => {
            let state = lock(&shared.state);
            !state.closed && (!state.eof || shared.write_only)
        }
        Err(_) => false,
    };
    Ok(vec![Value::Bool(open)])
}

// --- reader threads ---------------------------------------------------------

fn spawn_thread(name: &str, body: impl FnOnce() + Send + 'static) -> Result<(), String> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(body)
        .map(|_| ())
        .map_err(|err| format!("cannot start reader thread: {}", err))
}

/// Copies everything `reader` yields into the connection's buffer.
fn pump(shared: Arc<Shared>, mut reader: Box<dyn Read + Send>) {
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return shared.finish(None),
            Ok(n) => {
                let mut state = lock(&shared.state);
                while state.buf.len() >= MAX_BUFFER && !state.closed {
                    state = shared
                        .wake
                        .wait_timeout(state, WAKE)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                if state.closed {
                    return;
                }
                state.buf.extend(&chunk[..n]);
                shared.wake.notify_all();
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return shared.finish(Some(format!("read failed: {}", err))),
        }
    }
}

fn stream_conn(
    reader: Box<dyn Read + Send>,
    writer: Option<Box<dyn Write + Send>>,
    closer: Option<Box<dyn FnOnce() + Send>>,
) -> Result<Conn, String> {
    let shared = Shared::new();
    let background = shared.clone();
    spawn_thread("neyuki-net-reader", move || pump(background, reader))?;
    Ok(Conn {
        shared,
        writer: Arc::new(Mutex::new(writer)),
        websocket: false,
        closer,
    })
}

// --- TCP, pipes, Unix sockets -----------------------------------------------

fn tcp_connect(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|err| format!("cannot resolve {}: {}", host, err))?;
    let mut last = format!("cannot resolve {}", host);
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(err) => last = format!("cannot connect to {}:{}: {}", host, port, err),
        }
    }
    Err(last)
}

fn tcp_conn(stream: TcpStream) -> Result<Conn, String> {
    let clone = |s: &TcpStream| {
        s.try_clone()
            .map_err(|err| format!("socket error: {}", err))
    };
    let reader = clone(&stream)?;
    let closing = clone(&stream)?;
    stream_conn(
        Box::new(reader),
        Some(Box::new(stream)),
        Some(Box::new(move || {
            let _ = closing.shutdown(std::net::Shutdown::Both);
        })),
    )
}

fn finish_conn(_vm: &mut VM, conn: Conn) -> Result<Vec<Value>, String> {
    Ok(vec![stream_table(register(conn))])
}

/// `__net_connect(host, port, timeout?)`.
fn builtin_connect(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let host = string_arg(args, 0, "host")?;
    let port = crate::vm::libs::args::int_arg(args, 1, "port")?;
    let port = u16::try_from(port).map_err(|_| "port must be between 0 and 65535".to_string())?;
    let timeout = timeout_arg(args, 2)?;
    crate::vm::libs::background::run(
        vm,
        move || tcp_conn(tcp_connect(&host, port, timeout)?),
        finish_conn,
    )
}

/// `__net_connect_pipe(name)`: `\\.\pipe\<name>` on Windows (a name that
/// already starts with `\\` is used as given), a socket path elsewhere.
fn builtin_connect_pipe(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let name = string_arg(args, 0, "name")?;
    let timeout = timeout_arg(args, 1)?;
    crate::vm::libs::background::run(vm, move || connect_pipe(&name, timeout), finish_conn)
}

/// `__net_connect_unix(path)`.
fn builtin_connect_unix(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = string_arg(args, 0, "path")?;
    crate::vm::libs::background::run(vm, move || connect_unix(&path), finish_conn)
}

#[cfg(unix)]
fn connect_unix(path: &str) -> Result<Conn, String> {
    use std::os::unix::net::UnixStream;
    let stream =
        UnixStream::connect(path).map_err(|err| format!("cannot connect to {}: {}", path, err))?;
    let clone = |s: &UnixStream| {
        s.try_clone()
            .map_err(|err| format!("socket error: {}", err))
    };
    let reader = clone(&stream)?;
    let closing = clone(&stream)?;
    stream_conn(
        Box::new(reader),
        Some(Box::new(stream)),
        Some(Box::new(move || {
            let _ = closing.shutdown(std::net::Shutdown::Both);
        })),
    )
}

#[cfg(not(unix))]
fn connect_unix(_path: &str) -> Result<Conn, String> {
    Err("Unix domain sockets are not supported on Windows; use connectPipe".to_string())
}

#[cfg(unix)]
fn connect_pipe(name: &str, _timeout: Duration) -> Result<Conn, String> {
    connect_unix(name)
}

#[cfg(windows)]
fn connect_pipe(name: &str, timeout: Duration) -> Result<Conn, String> {
    use std::fs::OpenOptions;

    const ERROR_PIPE_BUSY: i32 = 231;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    let path = if name.starts_with("\\\\") {
        name.to_string()
    } else {
        format!("\\\\.\\pipe\\{}", name)
    };
    let start = Instant::now();
    let file = loop {
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => break file,
            Err(err) if err.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                if start.elapsed() >= timeout {
                    return Err(format!("pipe {} is busy", path));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) if err.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => {
                return Err(format!("cannot connect to {}: pipe does not exist", path));
            }
            Err(err) => return Err(format!("cannot connect to {}: {}", path, err)),
        }
    };
    let writer = file
        .try_clone()
        .map_err(|err| format!("pipe error: {}", err))?;
    // A blocked `ReadFile` would hold up writes on the same synchronous
    // handle, so the reader only reads what `PeekNamedPipe` reports is there.
    let shared = Shared::new();
    let background = shared.clone();
    let reader = PipeReader {
        file,
        shared: shared.clone(),
    };
    spawn_thread("neyuki-net-reader", move || {
        pump(background, Box::new(reader))
    })?;
    Ok(Conn {
        shared,
        writer: Arc::new(Mutex::new(Some(Box::new(writer)))),
        websocket: false,
        closer: None,
    })
}

#[cfg(not(any(unix, windows)))]
fn connect_pipe(_name: &str, _timeout: Duration) -> Result<Conn, String> {
    Err("pipes are not supported on this platform".to_string())
}

#[cfg(windows)]
struct PipeReader {
    file: std::fs::File,
    shared: Arc<Shared>,
}

#[cfg(windows)]
unsafe extern "system" {
    fn PeekNamedPipe(
        pipe: *mut std::ffi::c_void,
        buffer: *mut std::ffi::c_void,
        size: u32,
        read: *mut u32,
        available: *mut u32,
        left: *mut u32,
    ) -> i32;
}

#[cfg(windows)]
impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::os::windows::io::AsRawHandle;

        const ERROR_BROKEN_PIPE: i32 = 109;
        const ERROR_NO_DATA: i32 = 232;
        loop {
            if lock(&self.shared.state).closed {
                return Ok(0);
            }
            let mut available = 0u32;
            // SAFETY: the handle is open for the life of `self.file`, and
            // only the `available` out-pointer is given.
            let ok = unsafe {
                PeekNamedPipe(
                    self.file.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                return match err.raw_os_error() {
                    Some(ERROR_BROKEN_PIPE) | Some(ERROR_NO_DATA) => Ok(0),
                    _ => Err(err),
                };
            }
            if available > 0 {
                let len = buf.len().min(available as usize);
                return self.file.read(&mut buf[..len]);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

// --- WebSocket client -------------------------------------------------------

enum Stream {
    Plain(TcpStream),
    Tls(Box<native_tls::TlsStream<TcpStream>>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

/// The writing half of a stream the reader thread also uses.
struct SharedStream(Arc<Mutex<Stream>>);

impl Write for SharedStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        lock(&self.0).flush()
    }
}

struct WsUrl {
    tls: bool,
    host: String,
    port: u16,
    /// The `Host` header value.
    authority: String,
    target: String,
}

fn parse_ws_url(url: &str) -> Result<WsUrl, String> {
    let (tls, rest) = if let Some(rest) = url.strip_prefix("ws://") {
        (false, rest)
    } else if let Some(rest) = url.strip_prefix("wss://") {
        (true, rest)
    } else {
        return Err("url must start with ws:// or wss://".to_string());
    };
    let (authority, target) = match rest.find(['/', '?']) {
        Some(at) if rest.as_bytes()[at] == b'/' => (&rest[..at], rest[at..].to_string()),
        Some(at) => (&rest[..at], format!("/{}", &rest[at..])),
        None => (rest, "/".to_string()),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let default_port = if tls { 443 } else { 80 };
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (host, after) = inner.split_once(']').ok_or("invalid host in url")?;
        match after.strip_prefix(':') {
            Some(port) => (host, port.parse().map_err(|_| "invalid port in url")?),
            None => (host, default_port),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().map_err(|_| "invalid port in url")?),
            None => (authority, default_port),
        }
    };
    if host.is_empty() {
        return Err("url has no host".to_string());
    }
    Ok(WsUrl {
        tls,
        host: host.to_string(),
        port,
        authority: authority.to_string(),
        target,
    })
}

/// One masked client frame.
fn ws_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    match payload.len() {
        n if n < 126 => frame.push(0x80 | n as u8),
        n if n <= 0xFFFF => {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    let mut mask = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut mask);
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    frame
}

fn ws_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    BASE64.encode(hasher.finalize())
}

fn header_text(headers: &[(String, String)]) -> Result<String, String> {
    let mut text = String::new();
    for (name, value) in headers {
        let bad = |s: &str| s.contains(['\r', '\n']);
        if name.is_empty() || bad(name) || bad(value) || name.contains(':') {
            return Err(format!("invalid header '{}'", name));
        }
        text.push_str(&format!("{}: {}\r\n", name, value));
    }
    Ok(text)
}

fn ws_connect(
    url: &str,
    headers: Vec<(String, String)>,
    timeout: Duration,
) -> Result<Conn, String> {
    let target = parse_ws_url(url)?;
    let extra = header_text(&headers)?;
    let tcp = tcp_connect(&target.host, target.port, timeout)?;
    let closing = tcp
        .try_clone()
        .map_err(|err| format!("socket error: {}", err))?;
    let _ = tcp.set_read_timeout(Some(timeout));
    let _ = tcp.set_write_timeout(Some(timeout));
    let reader_tcp = tcp
        .try_clone()
        .map_err(|err| format!("socket error: {}", err))?;
    let mut stream = if target.tls {
        let connector = native_tls::TlsConnector::new()
            .map_err(|err| format!("TLS is unavailable: {}", err))?;
        let tls = connector
            .connect(&target.host, tcp)
            .map_err(|err| format!("TLS handshake with {} failed: {}", target.host, err))?;
        Stream::Tls(Box::new(tls))
    } else {
        Stream::Plain(tcp)
    };

    let mut nonce = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce);
    let key = BASE64.encode(nonce);
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n{}\r\n",
        target.target, target.authority, key, extra
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|err| format!("WebSocket handshake failed: {}", err))?;

    // Read byte by byte so nothing past the headers is consumed.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 {
            return Err("WebSocket handshake response is too large".to_string());
        }
        match stream.read(&mut byte) {
            Ok(0) => return Err("connection closed during the WebSocket handshake".to_string()),
            Ok(_) => head.push(byte[0]),
            Err(err) => return Err(format!("WebSocket handshake failed: {}", err)),
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or("");
    if status.split_whitespace().nth(1) != Some("101") {
        return Err(format!("WebSocket upgrade refused: {}", status));
    }
    let accepted = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("sec-websocket-accept") && value.trim() == ws_accept(&key)
        })
    });
    if !accepted {
        return Err("WebSocket server sent a wrong Sec-WebSocket-Accept".to_string());
    }

    // From here the reader thread polls with a short timeout so it never
    // holds the stream while a send waits.
    let _ = reader_tcp.set_read_timeout(Some(Duration::from_millis(50)));
    let stream = Arc::new(Mutex::new(stream));
    let shared = Shared::new();
    let background = shared.clone();
    let reader_stream = stream.clone();
    spawn_thread("neyuki-ws-reader", move || {
        ws_pump(background, reader_stream)
    })?;
    let closing_stream = stream.clone();
    Ok(Conn {
        shared,
        writer: Arc::new(Mutex::new(Some(Box::new(SharedStream(stream))))),
        websocket: true,
        closer: Some(Box::new(move || {
            // The reader thread is polling, so this gets the lock quickly.
            let _ = SharedStream(closing_stream).write_all(&ws_frame(8, &1000u16.to_be_bytes()));
            let _ = closing.shutdown(std::net::Shutdown::Both);
        })),
    })
}

/// Reads frames off the WebSocket into the message queue, answering pings.
fn ws_pump(shared: Arc<Shared>, stream: Arc<Mutex<Stream>>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut partial: Option<(bool, Vec<u8>)> = None;
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        if lock(&shared.state).closed {
            return;
        }
        let read = lock(&stream).read(&mut chunk);
        match read {
            Ok(0) => return shared.finish(None),
            Ok(n) => pending.extend_from_slice(&chunk[..n]),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                // Give a waiting send the lock.
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(err) => return shared.finish(Some(format!("read failed: {}", err))),
        }
        std::thread::yield_now();
        loop {
            let frame = match next_frame(&mut pending) {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(error) => return shared.finish(Some(error)),
            };
            let (fin, opcode, payload) = frame;
            match opcode {
                0x8 => return shared.finish(None),
                0x9 => {
                    let _ = SharedStream(stream.clone()).write_all(&ws_frame(0xA, &payload));
                }
                0xA => {}
                0x0..=0x2 => {
                    let (text, mut data) = match (opcode, partial.take()) {
                        (0x0, Some(started)) => started,
                        (0x0, None) => {
                            return shared.finish(Some("unexpected continuation frame".into()));
                        }
                        (op, None) => (op == 0x1, Vec::new()),
                        (_, Some(_)) => {
                            return shared.finish(Some("interleaved fragmented messages".into()));
                        }
                    };
                    if data.len() + payload.len() > MAX_BUFFER {
                        return shared.finish(Some("WebSocket message is too large".into()));
                    }
                    data.extend_from_slice(&payload);
                    if fin {
                        let mut state = lock(&shared.state);
                        while state.messages.len() >= 4096 && !state.closed {
                            state = shared
                                .wake
                                .wait_timeout(state, WAKE)
                                .unwrap_or_else(|e| e.into_inner())
                                .0;
                        }
                        state.messages.push_back((text, data));
                        shared.wake.notify_all();
                    } else {
                        partial = Some((text, data));
                    }
                }
                _ => return shared.finish(Some("unknown WebSocket opcode".into())),
            }
        }
    }
}

type Frame = (bool, u8, Vec<u8>);

/// The first complete frame in `pending`, removed from it.
fn next_frame(pending: &mut Vec<u8>) -> Result<Option<Frame>, String> {
    if pending.len() < 2 {
        return Ok(None);
    }
    let fin = pending[0] & 0x80 != 0;
    let opcode = pending[0] & 0x0F;
    let masked = pending[1] & 0x80 != 0;
    let mut header = 2;
    let length = match pending[1] & 0x7F {
        126 => {
            if pending.len() < 4 {
                return Ok(None);
            }
            header = 4;
            u16::from_be_bytes([pending[2], pending[3]]) as u64
        }
        127 => {
            if pending.len() < 10 {
                return Ok(None);
            }
            header = 10;
            u64::from_be_bytes(pending[2..10].try_into().expect("eight bytes"))
        }
        n => n as u64,
    };
    if length > MAX_BUFFER as u64 {
        return Err("WebSocket frame is too large".to_string());
    }
    let length = length as usize;
    let mask_len = if masked { 4 } else { 0 };
    if pending.len() < header + mask_len + length {
        return Ok(None);
    }
    let mut payload = pending[header + mask_len..header + mask_len + length].to_vec();
    if masked {
        let mask = [
            pending[header],
            pending[header + 1],
            pending[header + 2],
            pending[header + 3],
        ];
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i % 4];
        }
    }
    pending.drain(..header + mask_len + length);
    Ok(Some((fin, opcode, payload)))
}

/// `__net_websocket(url, options?)`.
fn builtin_websocket(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let url = string_arg(args, 0, "url")?;
    let mut headers = Vec::new();
    let mut timeout = DEFAULT_TIMEOUT;
    match args.get(1) {
        None | Some(Value::Nil) => {}
        Some(Value::Table(options)) => {
            let options = options.borrow();
            match options.get_str("headers") {
                Value::Nil => {}
                Value::Table(list) => {
                    for (name, value) in list.borrow().fields.iter() {
                        match value {
                            Value::String(v) => {
                                headers.push(((&**name).to_string(), v.to_string()))
                            }
                            other => {
                                return Err(format!(
                                    "headers.{} must be a string, got {}",
                                    &**name,
                                    other.type_name()
                                ));
                            }
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "headers must be a table, got {}",
                        other.type_name()
                    ));
                }
            }
            timeout = timeout_arg(&[options.get_str("timeout")], 0)?;
        }
        Some(other) => {
            return Err(format!(
                "options must be a table, got {}",
                other.type_name()
            ));
        }
    }
    crate::vm::libs::background::run(
        vm,
        move || ws_connect(&url, headers, timeout),
        |_vm, conn| Ok(vec![websocket_table(register(conn))]),
    )
}

/// `ws:send(data)`: a string goes as a text message, a buffer as a binary
/// one.
fn ws_send(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = self_id(args)?;
    let (data, binary) = data_arg(args, 1, "data")?;
    let (shared, writer, _) = parts(id)?;
    {
        let state = lock(&shared.state);
        if state.closed || state.eof {
            return Err("connection is closed".to_string());
        }
    }
    let frame = ws_frame(if binary { 0x2 } else { 0x1 }, &data);
    write_all(vm, writer, frame)
}

/// `ws:receive()`: the next message, suspending the task until one arrives;
/// a string for text and a buffer for binary, then `"text"` or `"binary"`.
/// nil once the connection has closed and everything has been read.
fn ws_receive(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = self_id(args)?;
    let (shared, _, _) = parts(id)?;
    {
        let mut state = lock(&shared.state);
        if state.closed || state.eof || !state.messages.is_empty() {
            let message = take_message(&mut state)?;
            drop(state);
            return message_values(vm, message);
        }
    }
    crate::vm::libs::background::run(
        vm,
        move || {
            let mut state = shared.wait_for(|s| !s.messages.is_empty());
            take_message(&mut state)
        },
        message_values,
    )
}

fn take_message(state: &mut State) -> Result<Option<(bool, Vec<u8>)>, String> {
    if let Some(message) = state.messages.pop_front() {
        return Ok(Some(message));
    }
    if state.closed {
        return Err("connection is closed".to_string());
    }
    match state.error.take() {
        Some(error) => Err(error),
        None => Ok(None),
    }
}

fn message_values(vm: &mut VM, message: Option<(bool, Vec<u8>)>) -> Result<Vec<Value>, String> {
    match message {
        None => Ok(vec![Value::Nil]),
        Some((true, data)) => Ok(vec![
            Value::string(String::from_utf8_lossy(&data).into_owned()),
            Value::str("text"),
        ]),
        Some((false, data)) => {
            let mut values = crate::vm::libs::buffer::new_buffer(vm, data)?;
            values.push(Value::str("binary"));
            Ok(values)
        }
    }
}

// --- os.spawn ---------------------------------------------------------------

fn process_arg(args: &[Value]) -> Result<Arc<Process>, String> {
    let id = self_id(args)?;
    PROCESSES.with(|processes| {
        processes
            .borrow()
            .get(&id)
            .cloned()
            .ok_or_else(|| "process is gone".to_string())
    })
}

fn stdio_mode(options: &VmTable, name: &str) -> Result<Stdio, String> {
    match options.get_str(name) {
        Value::Nil => Ok(Stdio::inherit()),
        Value::String(mode) => match &*mode {
            "pipe" => Ok(Stdio::piped()),
            "inherit" => Ok(Stdio::inherit()),
            "null" => Ok(Stdio::null()),
            other => Err(format!(
                "{} must be \"pipe\", \"inherit\" or \"null\", got \"{}\"",
                name, other
            )),
        },
        other => Err(format!(
            "{} must be a string, got {}",
            name,
            other.type_name()
        )),
    }
}

/// `os.spawn(command, options?)`: starts a program without waiting for it.
/// `command` is a string for the shell or a list of strings; `options` has
/// `cwd`, `env` and `stdin`, `stdout`, `stderr` (each `"pipe"`, `"inherit"`
/// (default) or `"null"`). Returns `{ pid, stdin, stdout, stderr, wait,
/// kill, isRunning }`; the piped streams are connections.
pub(crate) fn os_spawn(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut command = match args.first() {
        Some(Value::String(line)) => {
            let mut command = Command::new(if cfg!(windows) { "cmd" } else { "sh" });
            command
                .arg(if cfg!(windows) { "/C" } else { "-c" })
                .arg(&**line);
            command
        }
        Some(Value::Table(list)) => {
            let list = list.borrow();
            let mut parts = Vec::with_capacity(list.array.len());
            for (index, part) in list.array.iter().enumerate() {
                match part {
                    Value::String(s) => parts.push(s.to_string()),
                    other => {
                        return Err(format!(
                            "command[{}] must be a string, got {}",
                            index + 1,
                            other.type_name()
                        ));
                    }
                }
            }
            let (program, rest) = parts.split_first().ok_or("command must not be empty")?;
            let mut command = Command::new(program);
            command.args(rest);
            command
        }
        None | Some(Value::Nil) => return Err("command must be provided".to_string()),
        Some(other) => {
            return Err(format!(
                "command must be a string or a list of strings, got {}",
                other.type_name()
            ));
        }
    };
    let (mut stdin, mut stdout, mut stderr) =
        (Stdio::inherit(), Stdio::inherit(), Stdio::inherit());
    let mut show_window = false;
    match args.get(1) {
        None | Some(Value::Nil) => {}
        Some(Value::Table(options)) => {
            let options = options.borrow();
            match options.get_str("cwd") {
                Value::Nil => {}
                Value::String(cwd) => {
                    command.current_dir(&*cwd);
                }
                other => return Err(format!("cwd must be a string, got {}", other.type_name())),
            }
            match options.get_str("env") {
                Value::Nil => {}
                Value::Table(env) => {
                    for (key, value) in env.borrow().fields.iter() {
                        match value {
                            Value::String(v) => {
                                command.env(&**key, &**v);
                            }
                            other => {
                                return Err(format!(
                                    "env.{} must be a string, got {}",
                                    &**key,
                                    other.type_name()
                                ));
                            }
                        }
                    }
                }
                other => return Err(format!("env must be a table, got {}", other.type_name())),
            }
            stdin = stdio_mode(&options, "stdin")?;
            stdout = stdio_mode(&options, "stdout")?;
            stderr = stdio_mode(&options, "stderr")?;
            show_window = matches!(options.get_str("showWindow"), Value::Bool(true));
        }
        Some(other) => {
            return Err(format!(
                "options must be a table, got {}",
                other.type_name()
            ));
        }
    }
    command.stdin(stdin).stdout(stdout).stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if !show_window {
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
    }
    #[cfg(not(windows))]
    let _ = show_window;

    let mut child = command
        .spawn()
        .map_err(|err| format!("cannot start process: {}", err))?;
    let pid = child.id();
    let mut table = VmTable::new();
    table.set_str("pid", Value::Int(pid as i64));
    if let Some(pipe) = child.stdin.take() {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                eof: true,
                ..State::default()
            }),
            wake: Condvar::new(),
            write_only: true,
        });
        let conn = Conn {
            shared,
            writer: Arc::new(Mutex::new(Some(Box::new(pipe)))),
            websocket: false,
            closer: None,
        };
        table.set_str("stdin", stream_table(register(conn)));
    }
    if let Some(pipe) = child.stdout.take() {
        table.set_str(
            "stdout",
            stream_table(register(output_conn(Box::new(pipe))?)),
        );
    }
    if let Some(pipe) = child.stderr.take() {
        table.set_str(
            "stderr",
            stream_table(register(output_conn(Box::new(pipe))?)),
        );
    }
    let id = next_id();
    PROCESSES.with(|processes| {
        processes.borrow_mut().insert(
            id,
            Arc::new(Process {
                child: Mutex::new(child),
                status: Mutex::new(None),
            }),
        )
    });
    table.set_str("_id", Value::Int(id as i64));
    table.set_str("wait", crate::native!("process.wait", process_wait));
    table.set_str("kill", crate::native!("process.kill", process_kill));
    table.set_str(
        "isRunning",
        crate::native!("process.isRunning", process_is_running),
    );
    Ok(vec![frozen(table)])
}

fn output_conn(reader: Box<dyn Read + Send>) -> Result<Conn, String> {
    stream_conn(reader, None, None)
}

/// `p:wait()`: suspends the task until the process ends; returns its exit
/// code (-1 if a signal ended it).
fn process_wait(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let process = process_arg(args)?;
    crate::vm::libs::background::run(
        vm,
        move || {
            let start = Instant::now();
            loop {
                if let Some(code) = process.poll()? {
                    return Ok(code);
                }
                // Poll gently at first so short commands return quickly.
                std::thread::sleep(if start.elapsed() < Duration::from_secs(1) {
                    Duration::from_millis(5)
                } else {
                    Duration::from_millis(25)
                });
            }
        },
        |_vm, code| Ok(vec![Value::Int(code)]),
    )
}

/// `p:kill()`: ends the process; killing one that already ended is fine.
fn process_kill(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let process = process_arg(args)?;
    if process.poll()?.is_none() {
        let _ = lock(&process.child).kill();
    }
    Ok(vec![])
}

fn process_is_running(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let process = process_arg(args)?;
    Ok(vec![Value::Bool(process.poll()?.is_none())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_accept_matches_the_rfc_example() {
        assert_eq!(
            ws_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn test_parse_ws_url() {
        let url = parse_ws_url("ws://127.0.0.1:9222/devtools?x=1").unwrap();
        assert_eq!((url.host.as_str(), url.port), ("127.0.0.1", 9222));
        assert_eq!(url.target, "/devtools?x=1");
        let url = parse_ws_url("wss://example.com").unwrap();
        assert_eq!((url.port, url.tls, url.target.as_str()), (443, true, "/"));
        assert!(parse_ws_url("http://example.com").is_err());
    }

    #[test]
    fn test_frame_round_trip() {
        let mut bytes = ws_frame(0x1, b"hello");
        let (fin, opcode, payload) = next_frame(&mut bytes).unwrap().unwrap();
        assert!(fin && bytes.is_empty());
        assert_eq!((opcode, payload.as_slice()), (0x1, &b"hello"[..]));
        let mut long = ws_frame(0x2, &[7u8; 300]);
        let half = long.split_off(100);
        assert!(next_frame(&mut long).unwrap().is_none());
        long.extend(half);
        assert_eq!(next_frame(&mut long).unwrap().unwrap().2.len(), 300);
    }
}
