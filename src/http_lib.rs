//! Native HTTP primitives backing `@neyuki/http`.
//!
//! The client side is a single `__http_request` native built on `ureq`. The
//! server side is pull-based because natives cannot call back into Neyuki
//! functions: `__http_listen` opens a listener, `__http_accept` hands back the
//! next request as a table (keeping the connection parked here under an id)
//! and `__http_respond` answers it. `lib/http.nyk` builds the `Response`,
//! `Request` and `Server` objects and the `serve` loop on top of these.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Duration;

use num_bigint::BigInt;
use ureq::ResponseExt;

use crate::runtime::{Value, new_table};

pub(crate) const NATIVES: &[(&str, crate::runtime::Native)] = &[
    ("__http_request", builtin_request),
    ("__http_listen", builtin_listen),
    ("__http_accept", builtin_accept),
    ("__http_respond", builtin_respond),
    ("__http_close", builtin_close),
    ("__http_url_encode", builtin_url_encode),
    ("__http_url_decode", builtin_url_decode),
];

thread_local! {
    static SERVERS: RefCell<HashMap<u64, tiny_http::Server>> = RefCell::new(HashMap::new());
    static PENDING: RefCell<HashMap<u64, tiny_http::Request>> = RefCell::new(HashMap::new());
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
    Value::Integer(BigInt::from(value))
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
    request
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(|err| format!("cannot read request body: {}", err))?;
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
        ("body", Value::String(bytes_to_string(bytes))),
        ("remote", remote),
    ]);
    PENDING.with(|pending| pending.borrow_mut().insert(request_id, request));
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

    let request = PENDING
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
