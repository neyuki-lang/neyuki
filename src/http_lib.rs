//! Native HTTP primitives backing `@neyuki/http`.
//!
//! The client side is a single `__http_request` native built on `ureq`. The
//! server side is pull-based because natives cannot call back into Neyuki
//! functions: `__http_listen` opens a listener, `__http_accept` hands back the
//! next request as a table (keeping the connection parked here under an id)
//! and `__http_respond` answers it. `lib/http.nyk` builds the `Response`,
//! `Request` and `Server` objects and the `serve` loop on top of these.
//!
//! Two natives forward traffic elsewhere. `__http_proxy` answers a parked
//! request with whatever an upstream server replies, and `__http_forward`
//! is a plain TCP port forward whose accept loop and per-connection copies
//! run on their own threads, so the interpreter never sees the bytes.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use num_bigint::BigInt;
use ureq::ResponseExt;

use crate::runtime::{Int, Value, new_table};

pub(crate) const NATIVES: &[(&str, crate::runtime::Native)] = &[
    ("__http_request", builtin_request),
    ("__http_listen", builtin_listen),
    ("__http_accept", builtin_accept),
    ("__http_respond", builtin_respond),
    ("__http_close", builtin_close),
    ("__http_proxy", builtin_proxy),
    ("__http_forward", builtin_forward),
    ("__http_forward_wait", builtin_forward_wait),
    ("__http_unforward", builtin_unforward),
    ("__http_url_encode", builtin_url_encode),
    ("__http_url_decode", builtin_url_decode),
];

/// A running TCP port forward. Only the accept thread touches the listener;
/// `stop` plus one wake-up connection make it exit, and joining it is what
/// releases the port.
struct Forward {
    stop: Arc<AtomicBool>,
    address: SocketAddr,
    thread: Option<JoinHandle<()>>,
}

thread_local! {
    static SERVERS: RefCell<HashMap<u64, tiny_http::Server>> = RefCell::new(HashMap::new());
    // Each parked request keeps its raw body so a proxy can pass it on
    // byte-for-byte; the table Neyuki sees holds the (lossy) string copy.
    static PENDING: RefCell<HashMap<u64, (tiny_http::Request, Vec<u8>)>> = RefCell::new(HashMap::new());
    static FORWARDS: RefCell<HashMap<u64, Forward>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

fn next_id() -> u64 {
    NEXT_ID.with(|id| {
        let value = id.get();
        id.set(value + 1);
        value
    })
}

fn string_arg(args: &[Value], index: usize, name: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(value) => Err(format!(
            "{} must be a string, got {}",
            name,
            value.type_name()
        )),
        None => Err(format!("{} must be provided", name)),
    }
}

fn optional_string_arg(args: &[Value], index: usize, name: &str) -> Result<Option<String>, String> {
    match args.get(index) {
        None | Some(Value::Nil) => Ok(None),
        _ => string_arg(args, index, name).map(Some),
    }
}

fn integer_arg(args: &[Value], index: usize, name: &str) -> Result<u64, String> {
    let value = args.get(index).cloned().unwrap_or(Value::Nil);
    let number = crate::runtime::number(value).map_err(|_| format!("{} must be a number", name))?;
    if number < 0.0 || number.fract() != 0.0 {
        return Err(format!("{} must be a non-negative integer", name));
    }
    Ok(number as u64)
}

/// A timeout in seconds; nil, absent or zero means "no timeout".
fn timeout_arg(args: &[Value], index: usize, name: &str) -> Result<Option<Duration>, String> {
    let value = args.get(index).cloned().unwrap_or(Value::Nil);
    if matches!(value, Value::Nil) {
        return Ok(None);
    }
    let seconds =
        crate::runtime::number(value).map_err(|_| format!("{} must be a number", name))?;
    if seconds < 0.0 || !seconds.is_finite() {
        return Err(format!("{} must be a non-negative number of seconds", name));
    }
    if seconds == 0.0 {
        return Ok(None);
    }
    Ok(Some(Duration::from_secs_f64(seconds)))
}

fn bool_arg(args: &[Value], index: usize, name: &str, default: bool) -> Result<bool, String> {
    match args.get(index) {
        Some(Value::Bool(value)) => Ok(*value),
        None | Some(Value::Nil) => Ok(default),
        Some(value) => Err(format!(
            "{} must be a boolean, got {}",
            name,
            value.type_name()
        )),
    }
}

/// Reads a `{ name = value }` table of headers; nil is an empty set. Values
/// may be strings or numbers.
fn headers_arg(args: &[Value], index: usize, name: &str) -> Result<Vec<(String, String)>, String> {
    let table = match args.get(index) {
        None | Some(Value::Nil) => return Ok(Vec::new()),
        Some(Value::Table(table)) => table.clone(),
        Some(value) => {
            return Err(format!(
                "{} must be a table, got {}",
                name,
                value.type_name()
            ));
        }
    };
    let table = table.borrow();
    let mut headers: Vec<(String, String)> = table
        .fields
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Value::String(text) => text.clone(),
                Value::Integer(_) | Value::Number(_) => value.to_string(),
                other => {
                    return Err(format!(
                        "header `{}` must be a string or a number, got {}",
                        key,
                        other.type_name()
                    ));
                }
            };
            Ok((key.clone(), value))
        })
        .collect::<Result<_, _>>()?;
    // Deterministic order makes requests reproducible regardless of hashing.
    headers.sort();
    Ok(headers)
}

/// Builds a `{ name = value }` table with lower-cased names. Repeated headers
/// are joined with `", "` as RFC 9110 allows.
fn headers_table<'a>(headers: impl Iterator<Item = (&'a str, &'a [u8])>) -> Value {
    let mut fields: HashMap<String, Value> = HashMap::new();
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        let value = String::from_utf8_lossy(value).into_owned();
        match fields.get_mut(&name) {
            Some(Value::String(existing)) => {
                existing.push_str(", ");
                existing.push_str(&value);
            }
            _ => {
                fields.insert(name, Value::String(value));
            }
        }
    }
    let table = new_table(Vec::new());
    if let Value::Table(inner) = &table {
        inner.borrow_mut().fields = fields;
    }
    table
}

/// Neyuki strings are UTF-8, so bodies that are not valid UTF-8 have their
/// offending bytes replaced with U+FFFD rather than failing the request.
fn bytes_to_string(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
}

fn record(fields: Vec<(&str, Value)>) -> Value {
    let table = new_table(Vec::new());
    if let Value::Table(inner) = &table {
        let mut inner = inner.borrow_mut();
        for (name, value) in fields {
            inner.fields.insert(name.to_string(), value);
        }
    }
    table
}

fn integer(value: u64) -> Value {
    Value::Integer(Int::from_bigint(BigInt::from(value)))
}

/// `__http_request(method, url, headers?, body?, timeout?, follow?)` performs
/// one request and returns `{ status, headers, body, url }`. Only transport
/// failures (DNS, connection, TLS, timeout, too many redirects) are errors.
fn builtin_request(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let method = string_arg(&args, 0, "method")?.to_ascii_uppercase();
    let url = string_arg(&args, 1, "url")?;
    let headers = headers_arg(&args, 2, "headers")?;
    let body = optional_string_arg(&args, 3, "body")?;
    let timeout = timeout_arg(&args, 4, "timeout")?;
    let follow = bool_arg(&args, 5, "follow", true)?;

    // The crate is built with only the platform TLS backend (see Cargo.toml),
    // so it must be selected explicitly.
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .build();
    let config = ureq::config::Config::builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .timeout_global(timeout)
        .max_redirects(if follow { 10 } else { 0 })
        .build();
    let agent = config.new_agent();

    let mut builder = ureq::http::Request::builder()
        .method(method.as_str())
        .uri(url.as_str());
    for (name, value) in &headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let request = builder
        .body(body.map(String::into_bytes).unwrap_or_default())
        .map_err(|err| format!("cannot build request for `{}`: {}", url, err))?;

    let mut response = agent
        .run(request)
        .map_err(|err| format!("cannot {} `{}`: {}", method, url, err))?;

    let status = u64::from(response.status().as_u16());
    let final_url = response.get_uri().to_string();
    let headers = headers_table(
        response
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_bytes())),
    );
    let bytes = response
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_to_vec()
        .map_err(|err| format!("cannot read response from `{}`: {}", url, err))?;

    Ok(vec![record(vec![
        ("status", integer(status)),
        ("headers", headers),
        ("body", Value::String(bytes_to_string(bytes))),
        ("url", Value::String(final_url)),
    ])])
}

/// `__http_listen(host, port)` binds a server and returns its id followed by
/// the bound port (port 0 asks the OS for a free one).
fn builtin_listen(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let host = string_arg(&args, 0, "host")?;
    let port = integer_arg(&args, 1, "port")?;
    if port > u64::from(u16::MAX) {
        return Err(format!("port {} is out of range (0-65535)", port));
    }
    let address = format!("{}:{}", host, port);
    let server = tiny_http::Server::http(&address)
        .map_err(|err| format!("cannot listen on `{}`: {}", address, err))?;
    let bound_port = server
        .server_addr()
        .to_ip()
        .map(|addr| u64::from(addr.port()))
        .unwrap_or(port);
    let id = next_id();
    SERVERS.with(|servers| servers.borrow_mut().insert(id, server));
    Ok(vec![integer(id), integer(bound_port)])
}

/// Whether a request asks to switch protocols (`Connection: upgrade` or an
/// `Upgrade` header).
fn is_upgrade(request: &tiny_http::Request) -> bool {
    request.headers().iter().any(|header| {
        header.field.equiv("Upgrade")
            || (header.field.equiv("Connection")
                && header
                    .value
                    .as_str()
                    .to_ascii_lowercase()
                    .contains("upgrade"))
    })
}

/// `__http_accept(server, timeout?)` waits for the next request and returns
/// `{ id, method, path, headers, body, remote }`, or nil once `timeout`
/// seconds pass without one.
fn builtin_accept(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "server")?;
    let timeout = timeout_arg(&args, 1, "timeout")?;

    let received = SERVERS.with(|servers| {
        let servers = servers.borrow();
        let server = servers
            .get(&id)
            .ok_or_else(|| "server is closed".to_string())?;
        match timeout {
            Some(duration) => server.recv_timeout(duration),
            None => server.recv().map(Some),
        }
        .map_err(|err| format!("cannot accept request: {}", err))
    })?;
    let Some(mut request) = received else {
        return Ok(vec![Value::Nil]);
    };

    let mut bytes = Vec::new();
    if is_upgrade(&request) {
        // tiny_http hands over the raw socket as the body of an upgrade
        // request (a WebSocket handshake, say), so reading to EOF would block
        // until the client hangs up. Take only what Content-Length promises.
        if let Some(length) = request.body_length() {
            request
                .as_reader()
                .take(length as u64)
                .read_to_end(&mut bytes)
                .map_err(|err| format!("cannot read request body: {}", err))?;
        }
    } else {
        request
            .as_reader()
            .read_to_end(&mut bytes)
            .map_err(|err| format!("cannot read request body: {}", err))?;
    }
    let headers = headers_table(
        request
            .headers()
            .iter()
            .map(|header| (header.field.as_str().as_str(), header.value.as_bytes())),
    );
    let remote = match request.remote_addr() {
        Some(addr) => Value::String(addr.to_string()),
        None => Value::Nil,
    };
    let request_id = next_id();
    let table = record(vec![
        ("id", integer(request_id)),
        (
            "method",
            Value::String(request.method().as_str().to_string()),
        ),
        ("path", Value::String(request.url().to_string())),
        ("headers", headers),
        ("body", Value::String(bytes_to_string(bytes.clone()))),
        ("remote", remote),
    ]);
    PENDING.with(|pending| pending.borrow_mut().insert(request_id, (request, bytes)));
    Ok(vec![table])
}

/// `__http_respond(request, status, headers?, body?)` answers a request that
/// `__http_accept` returned. Each request can be answered once.
fn builtin_respond(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "request")?;
    let status = integer_arg(&args, 1, "status")?;
    if !(100..=999).contains(&status) {
        return Err(format!("invalid status code {}", status));
    }
    let headers = headers_arg(&args, 2, "headers")?;
    let body = optional_string_arg(&args, 3, "body")?.unwrap_or_default();

    let (request, _) = PENDING
        .with(|pending| pending.borrow_mut().remove(&id))
        .ok_or_else(|| "request was already answered".to_string())?;

    let mut response = tiny_http::Response::from_string(body)
        .with_status_code(tiny_http::StatusCode(status as u16));
    for (name, value) in headers {
        let header = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes())
            .map_err(|_| format!("invalid header `{}: {}`", name, value))?;
        response.add_header(header);
    }
    request
        .respond(response)
        .map_err(|err| format!("cannot send response: {}", err))?;
    Ok(vec![Value::Nil])
}

/// `__http_close(server)` stops listening and returns whether the server was
/// still open. Requests accepted but not yet answered are dropped.
fn builtin_close(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "server")?;
    let Some(server) = SERVERS.with(|servers| servers.borrow_mut().remove(&id)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let address = server.server_addr().to_ip();
    drop(server);
    // Dropping only signals tiny_http's accept thread; the socket is released
    // when that thread exits. Wait (briefly) until the port can be bound again
    // so a `listen` right after `close` does not race it.
    if let Some(address) = address {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::net::TcpListener::bind(address).is_err() && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(vec![Value::Bool(true)])
}

/// Headers that describe one hop rather than the message, which a proxy must
/// not copy (RFC 9110 §7.6.1), plus the ones the client library regenerates:
/// `host` and `content-length` from the outgoing request, `accept-encoding`
/// because `ureq` negotiates (and transparently decodes) gzip on its own.
const HOP_BY_HOP_REQUEST: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
    "accept-encoding",
];

/// The same for the upstream response. `content-encoding` goes because the
/// body has already been decoded, `content-length` because `tiny_http`
/// computes it from the buffered body.
const HOP_BY_HOP_RESPONSE: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "content-encoding",
];

/// `__http_proxy(request, base, timeout?)` answers a parked request with the
/// reply of `base .. path` (so `http://127.0.0.1:5173` plus `/app.js?v=1`),
/// forwarding the method, headers and raw body and copying the status,
/// headers and raw body back. Redirects are passed through rather than
/// followed. If the upstream cannot be reached the request is answered with
/// a 502 and the error is raised, so the caller learns what went wrong.
fn builtin_proxy(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "request")?;
    let base = string_arg(&args, 1, "target")?;
    let timeout = timeout_arg(&args, 2, "timeout")?;

    let (request, body) = PENDING
        .with(|pending| pending.borrow_mut().remove(&id))
        .ok_or_else(|| "request was already answered".to_string())?;
    let method = request.method().as_str().to_ascii_uppercase();
    let url = format!("{}{}", base.trim_end_matches('/'), request.url());

    match proxy_upstream(&request, &method, &url, body, timeout) {
        Ok(response) => {
            request
                .respond(response)
                .map_err(|err| format!("cannot send response: {}", err))?;
            Ok(vec![Value::Nil])
        }
        Err(err) => {
            let message = format!(
                "cannot forward {} {} to `{}`: {}",
                method,
                request.url(),
                url,
                err
            );
            let reply = tiny_http::Response::from_string(format!("Bad Gateway: {}\n", message))
                .with_status_code(tiny_http::StatusCode(502))
                .with_header(
                    tiny_http::Header::from_bytes("Content-Type", "text/plain; charset=utf-8")
                        .expect("static header is valid"),
                );
            let _ = request.respond(reply);
            Err(message)
        }
    }
}

/// Performs the upstream half of `__http_proxy` and builds the reply for the
/// parked request from what came back.
fn proxy_upstream(
    request: &tiny_http::Request,
    method: &str,
    url: &str,
    body: Vec<u8>,
    timeout: Option<Duration>,
) -> Result<tiny_http::Response<std::io::Cursor<Vec<u8>>>, String> {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .build();
    let config = ureq::config::Config::builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .timeout_global(timeout)
        .max_redirects(0)
        .build();
    let agent = config.new_agent();

    let mut builder = ureq::http::Request::builder().method(method).uri(url);
    for header in request.headers() {
        let name = header.field.as_str().as_str();
        if HOP_BY_HOP_REQUEST.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        builder = builder.header(name, header.value.as_bytes());
    }
    let upstream = builder
        .body(body)
        .map_err(|err| format!("cannot build request: {}", err))?;
    let mut response = agent.run(upstream).map_err(|err| err.to_string())?;

    let status = response.status().as_u16();
    let mut headers = Vec::new();
    for (name, value) in response.headers() {
        if HOP_BY_HOP_RESPONSE.contains(&name.as_str()) {
            continue;
        }
        // A header the upstream produced but tiny_http cannot represent is
        // dropped rather than failing the whole reply.
        if let Ok(header) =
            tiny_http::Header::from_bytes(name.as_str().as_bytes(), value.as_bytes())
        {
            headers.push(header);
        }
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_to_vec()
        .map_err(|err| format!("cannot read response: {}", err))?;

    let length = bytes.len();
    Ok(tiny_http::Response::new(
        tiny_http::StatusCode(status),
        headers,
        std::io::Cursor::new(bytes),
        Some(length),
        None,
    ))
}

/// Copies `from` into `to` until EOF, then half-closes `to` so the far end
/// sees the same EOF.
fn pipe(from: Arc<TcpStream>, to: Arc<TcpStream>) {
    let _ = std::io::copy(&mut &*from, &mut &*to);
    let _ = (&*to).flush();
    let _ = to.shutdown(Shutdown::Write);
}

/// Serves one forwarded connection: connects to the target and copies bytes
/// both ways until both sides are done. A target that cannot be reached
/// just closes the client connection.
///
/// The two directions share each socket through an `Arc` rather than
/// `try_clone`: on Windows a clone is a duplicated handle, and closing one
/// while the other direction is still using the socket left roughly 1 in
/// 150 connections hanging until the TCP timeout.
fn relay(client: TcpStream, target: &str) {
    let Ok(addresses) = target.to_socket_addrs() else {
        return;
    };
    let Some(upstream) = addresses
        .into_iter()
        .find_map(|address| TcpStream::connect_timeout(&address, Duration::from_secs(10)).ok())
    else {
        return;
    };
    let _ = client.set_nodelay(true);
    let _ = upstream.set_nodelay(true);
    let client = Arc::new(client);
    let upstream = Arc::new(upstream);
    let (client_reader, upstream_writer) = (client.clone(), upstream.clone());
    std::thread::spawn(move || pipe(client_reader, upstream_writer));
    pipe(upstream, client);
}

/// `__http_forward(host, port, target_host, target_port)` binds `host:port`
/// and relays every TCP connection it receives to `target_host:target_port`
/// on background threads. Returns the forward's id and the bound port (0
/// asks the OS for a free one). Nothing is checked about the target until a
/// connection arrives, so a forward can be set up before the target starts.
fn builtin_forward(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let host = string_arg(&args, 0, "host")?;
    let port = integer_arg(&args, 1, "port")?;
    let target_host = string_arg(&args, 2, "target host")?;
    let target_port = integer_arg(&args, 3, "target port")?;
    if port > u64::from(u16::MAX) {
        return Err(format!("port {} is out of range (0-65535)", port));
    }
    if target_port == 0 || target_port > u64::from(u16::MAX) {
        return Err(format!(
            "target port {} is out of range (1-65535)",
            target_port
        ));
    }
    let address = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&address)
        .map_err(|err| format!("cannot listen on `{}`: {}", address, err))?;
    let bound = listener
        .local_addr()
        .map_err(|err| format!("cannot listen on `{}`: {}", address, err))?;
    let target = format!("{}:{}", target_host, target_port);

    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                match connection {
                    Ok(client) => {
                        let target = target.clone();
                        std::thread::spawn(move || relay(client, &target));
                    }
                    // Accept errors (a client resetting mid-handshake, file
                    // descriptors running out) are transient; back off a bit.
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        })
    };

    let id = next_id();
    FORWARDS.with(|forwards| {
        forwards.borrow_mut().insert(
            id,
            Forward {
                stop,
                address: bound,
                thread: Some(thread),
            },
        )
    });
    Ok(vec![integer(id), integer(u64::from(bound.port()))])
}

/// `__http_forward_wait(forward, timeout?)` blocks the calling thread for
/// `timeout` seconds, or until the forward stops on its own without one, so
/// a script that only forwards can stay alive. Returns whether the forward
/// is still running.
fn builtin_forward_wait(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "forward")?;
    let timeout = timeout_arg(&args, 1, "timeout")?;
    let deadline = timeout.map(|duration| std::time::Instant::now() + duration);
    loop {
        let running = FORWARDS.with(|forwards| {
            forwards.borrow().get(&id).map(|forward| {
                forward
                    .thread
                    .as_ref()
                    .is_some_and(|thread| !thread.is_finished())
            })
        });
        let Some(running) = running else {
            return Err("forward is closed".to_string());
        };
        if !running {
            return Ok(vec![Value::Bool(false)]);
        }
        let step = match deadline {
            Some(deadline) => {
                let now = std::time::Instant::now();
                if now >= deadline {
                    return Ok(vec![Value::Bool(true)]);
                }
                (deadline - now).min(Duration::from_millis(100))
            }
            None => Duration::from_millis(100),
        };
        std::thread::sleep(step);
    }
}

/// `__http_unforward(forward)` stops a forward and returns whether it was
/// still running. Connections already relayed keep going until they close
/// on their own; only the listening socket goes away.
fn builtin_unforward(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "forward")?;
    let Some(mut forward) = FORWARDS.with(|forwards| forwards.borrow_mut().remove(&id)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    forward.stop.store(true, Ordering::SeqCst);
    // The accept loop only notices `stop` when a connection arrives, so make
    // one. A wildcard bind is reached through the matching loopback address.
    let mut wake = forward.address;
    if wake.ip().is_unspecified() {
        wake.set_ip(if wake.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    let woken = TcpStream::connect_timeout(&wake, Duration::from_secs(1)).is_ok();
    if let Some(thread) = forward.thread.take() {
        // Without the wake-up the thread would stay parked in accept() and
        // joining would hang; then the port stays bound until exit instead.
        if woken {
            let _ = thread.join();
        }
    }
    Ok(vec![Value::Bool(true)])
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
fn builtin_url_encode(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let text = string_arg(&args, 0, "s")?;
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }
    Ok(vec![Value::String(encoded)])
}

/// Decodes percent-escapes and `+` (as a space, the form-encoding convention).
fn builtin_url_decode(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let text = string_arg(&args, 0, "s")?;
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let code = bytes
                    .get(index + 1..index + 3)
                    .and_then(|pair| std::str::from_utf8(pair).ok())
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .ok_or_else(|| format!("invalid percent-escape at byte {}", index + 1))?;
                decoded.push(code);
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    Ok(vec![Value::String(bytes_to_string(decoded))])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `native` and unwraps its single result as a number.
    fn number_result(result: Result<Vec<Value>, String>, index: usize) -> u64 {
        let values = result.expect("native should succeed");
        crate::runtime::number(values[index].clone()).expect("result should be a number") as u64
    }

    /// Reads one HTTP/1.1 message from `stream` (headers plus a
    /// `Content-Length` body) as raw bytes.
    fn read_message(stream: &mut TcpStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream.read(&mut chunk).expect("read should succeed");
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        bytes
    }

    #[test]
    fn test_forward_relays_raw_bytes_and_close_frees_the_port() {
        // An echo server standing in for the target.
        let echo = TcpListener::bind("127.0.0.1:0").unwrap();
        let echo_port = echo.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in echo.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut data = Vec::new();
                let _ = stream.read_to_end(&mut data);
                let _ = stream.write_all(&data);
            }
        });

        let result = builtin_forward(vec![
            Value::String("127.0.0.1".into()),
            integer(0),
            Value::String("127.0.0.1".into()),
            integer(u64::from(echo_port)),
        ])
        .expect("forward should bind");
        let id = number_result(Ok(result.clone()), 0);
        let port = number_result(Ok(result), 1) as u16;
        assert_ne!(port, 0, "port 0 should be replaced by the bound port");

        // Every byte value must survive the trip, in both directions.
        let payload: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.write_all(&payload).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut echoed = Vec::new();
        client.read_to_end(&mut echoed).unwrap();
        assert_eq!(echoed, payload);

        // Waiting with a timeout returns once it passes, reporting "running".
        let waited = builtin_forward_wait(vec![integer(id), Value::Number(0.05)]).unwrap();
        assert!(matches!(waited[0], Value::Bool(true)));

        let closed = builtin_unforward(vec![integer(id)]).unwrap();
        assert!(matches!(closed[0], Value::Bool(true)));
        assert!(
            TcpListener::bind(("127.0.0.1", port)).is_ok(),
            "closing should release the port"
        );
        let again = builtin_unforward(vec![integer(id)]).unwrap();
        assert!(matches!(again[0], Value::Bool(false)));
        assert!(builtin_forward_wait(vec![integer(id)]).is_err());
    }

    #[test]
    fn test_forward_rejects_bad_ports() {
        let bind = |port: u64, target: u64| {
            builtin_forward(vec![
                Value::String("127.0.0.1".into()),
                integer(port),
                Value::String("127.0.0.1".into()),
                integer(target),
            ])
        };
        assert!(bind(70_000, 80).is_err());
        assert!(bind(0, 0).is_err());
        assert!(bind(0, 70_000).is_err());
    }

    #[test]
    fn test_proxy_passes_binary_bodies_and_strips_hop_headers() {
        // A hand-rolled upstream, so the reply is exactly known: binary body,
        // a gzip header that must vanish and a custom header that must stay.
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let seen = std::thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let request = read_message(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 201 Created\r\n\
                      Content-Type: application/octet-stream\r\n\
                      X-Upstream: yes\r\n\
                      Connection: close\r\n\
                      Content-Length: 4\r\n\r\n\xff\x00\x80\xfe",
                )
                .unwrap();
            request
        });

        let listen = builtin_listen(vec![Value::String("127.0.0.1".into()), integer(0)]).unwrap();
        let server = number_result(Ok(listen.clone()), 0);
        let port = number_result(Ok(listen), 1) as u16;

        // The client speaks raw HTTP too, so the reply bytes can be checked.
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .write_all(
                    b"POST /upload?x=1 HTTP/1.1\r\n\
                      Host: 127.0.0.1\r\n\
                      X-Test: 1\r\n\
                      Connection: close\r\n\
                      Content-Length: 3\r\n\r\n\x00\xff\x01",
                )
                .unwrap();
            read_message(&mut stream)
        });

        let accepted = builtin_accept(vec![integer(server)]).unwrap();
        let Value::Table(table) = &accepted[0] else {
            panic!("accept should return a table");
        };
        let request_id = table.borrow().fields.get("id").cloned().unwrap();
        builtin_proxy(vec![
            request_id,
            Value::String(format!("http://127.0.0.1:{}", upstream_port)),
        ])
        .expect("proxy should reach the upstream");

        let forwarded = seen.join().unwrap();
        let head = String::from_utf8_lossy(&forwarded).to_string();
        assert!(
            head.starts_with("POST /upload?x=1 HTTP/1.1\r\n"),
            "{}",
            head
        );
        let lower = head.to_ascii_lowercase();
        assert!(
            lower.contains("x-test: 1\r\n"),
            "custom headers should be forwarded"
        );
        assert!(
            lower.contains(&format!("host: 127.0.0.1:{}\r\n", upstream_port)),
            "host should name the upstream: {}",
            head
        );
        assert!(
            forwarded.ends_with(b"\x00\xff\x01"),
            "the raw body should be forwarded"
        );

        let reply = client.join().unwrap();
        let head = String::from_utf8_lossy(&reply).to_ascii_lowercase();
        assert!(head.starts_with("http/1.1 201 "), "{}", head);
        assert!(head.contains("x-upstream: yes\r\n"));
        assert!(head.contains("content-type: application/octet-stream\r\n"));
        assert!(
            reply.ends_with(b"\xff\x00\x80\xfe"),
            "the raw body should come back"
        );

        builtin_close(vec![integer(server)]).unwrap();
    }

    #[test]
    fn test_accept_does_not_block_on_an_upgrade_request() {
        let listen = builtin_listen(vec![Value::String("127.0.0.1".into()), integer(0)]).unwrap();
        let server = number_result(Ok(listen.clone()), 0);
        let port = number_result(Ok(listen), 1) as u16;
        // A WebSocket handshake: the client keeps the connection open,
        // waiting for a 101 that this server never sends.
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .write_all(
                    b"GET /?token=abc HTTP/1.1\r\n\
                      Host: 127.0.0.1\r\n\
                      Connection: Upgrade\r\n\
                      Upgrade: websocket\r\n\
                      Sec-WebSocket-Version: 13\r\n\
                      Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
                )
                .unwrap();
            read_message(&mut stream)
        });
        let started = std::time::Instant::now();
        let accepted = builtin_accept(vec![integer(server)]).unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "accept must not wait for the socket to close"
        );
        let Value::Table(table) = &accepted[0] else {
            panic!("accept should return a table");
        };
        let request_id = table.borrow().fields.get("id").cloned().unwrap();
        builtin_respond(vec![request_id, integer(426)]).unwrap();
        let reply = String::from_utf8_lossy(&client.join().unwrap()).to_string();
        assert!(reply.starts_with("HTTP/1.1 426 "), "{}", reply);
        builtin_close(vec![integer(server)]).unwrap();
    }

    #[test]
    fn test_proxy_answers_502_when_the_upstream_is_down() {
        let listen = builtin_listen(vec![Value::String("127.0.0.1".into()), integer(0)]).unwrap();
        let server = number_result(Ok(listen.clone()), 0);
        let port = number_result(Ok(listen), 1) as u16;
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .write_all(b"GET /x HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .unwrap();
            read_message(&mut stream)
        });
        let accepted = builtin_accept(vec![integer(server)]).unwrap();
        let Value::Table(table) = &accepted[0] else {
            panic!("accept should return a table");
        };
        let request_id = table.borrow().fields.get("id").cloned().unwrap();

        // Port 1 has nothing listening on it.
        let Err(err) = builtin_proxy(vec![
            request_id.clone(),
            Value::String("http://127.0.0.1:1".into()),
        ]) else {
            panic!("an unreachable upstream should be an error");
        };
        assert!(
            err.contains("cannot forward GET /x to `http://127.0.0.1:1/x`"),
            "{}",
            err
        );

        let reply = String::from_utf8_lossy(&client.join().unwrap()).to_string();
        assert!(reply.starts_with("HTTP/1.1 502 "), "{}", reply);
        assert!(reply.contains("Bad Gateway"));
        // The request is gone either way.
        assert!(builtin_respond(vec![request_id, integer(200)]).is_err());
        builtin_close(vec![integer(server)]).unwrap();
    }
}
