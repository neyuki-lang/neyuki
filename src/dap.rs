// Debug Adapter Protocol server (v1 subset) over stdio.
//
// Speaks DAP with function breakpoints only: `initialize`, `launch`,
// `setFunctionBreakpoints`, `configurationDone`, `threads`, `stackTrace`,
// `scopes`, `variables`, `continue`, `disconnect`. Stepping commands,
// `evaluate` and line breakpoints are rejected with clear errors (line
// numbers are not plumbed through the compilers yet).
//
// Execution model: the session owns one VM. The program runs when
// `configurationDone` arrives; a breakpoint hit (checked at every closure
// entry) sends a `stopped` event and serves requests from a nested loop
// until `continue`/`disconnect`. No threads are involved (the VM is
// `!Send`), so pausing is purely re-entrant through the hook-style check.
//
// Two honest limitations, both documented to clients by behavior:
// script `print()` output shares stdout with the DAP stream (keep debug
// sessions print-free), and yielding inside a paused inspection is
// unsupported.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use super::dap_json::{self, Json};
use crate::vm::machine::VM;
use crate::vm::value::Value;

/// Debug session state, owned by the VM (`None` = no session, zero cost).
pub struct DapSession {
    pub reader: Box<dyn BufRead>,
    pub writer: Box<dyn Write>,
    /// Function names that pause execution on entry.
    pub func_breakpoints: Vec<String>,
    /// Program given by `launch`, run at `configurationDone`.
    pub launch_program: Option<String>,
    /// variablesReference registry for compound values.
    pub variables: HashMap<usize, Value>,
    pub next_var_ref: usize,
    pub next_seq: u64,
    pub terminated: bool,
}

impl DapSession {
    pub fn stdio() -> Self {
        Self {
            reader: Box::new(std::io::BufReader::new(std::io::stdin())),
            writer: Box::new(io::stdout()),
            func_breakpoints: Vec::new(),
            launch_program: None,
            variables: HashMap::new(),
            next_var_ref: 1,
            next_seq: 1,
            terminated: false,
        }
    }

    /// In-memory session for tests. Returns the session plus a handle to
    /// everything written to it.
    #[cfg(test)]
    pub fn memory(input: Vec<u8>) -> (Self, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        use std::io::Cursor;
        let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        struct SharedOut(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl Write for SharedOut {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let session = Self {
            reader: Box::new(Cursor::new(input)),
            writer: Box::new(SharedOut(out.clone())),
            func_breakpoints: Vec::new(),
            launch_program: None,
            variables: HashMap::new(),
            next_var_ref: 1,
            next_seq: 1,
            terminated: false,
        };
        (session, out)
    }
}

fn ok_response(req_seq: f64, command: &str, server_seq: u64, body: Json) -> Json {
    Json::Obj(vec![
        ("seq".to_string(), Json::Num(server_seq as f64)),
        ("type".to_string(), Json::Str("response".to_string())),
        ("request_seq".to_string(), Json::Num(req_seq)),
        ("success".to_string(), Json::Bool(true)),
        ("command".to_string(), Json::Str(command.to_string())),
        ("body".to_string(), body),
    ])
}

fn err_response(req_seq: f64, command: &str, server_seq: u64, message: String) -> Json {
    Json::Obj(vec![
        ("seq".to_string(), Json::Num(server_seq as f64)),
        ("type".to_string(), Json::Str("response".to_string())),
        ("request_seq".to_string(), Json::Num(req_seq)),
        ("success".to_string(), Json::Bool(false)),
        ("command".to_string(), Json::Str(command.to_string())),
        ("message".to_string(), Json::Str(message)),
    ])
}

fn event_envelope(server_seq: u64, event: &str, body: Json) -> Json {
    Json::Obj(vec![
        ("seq".to_string(), Json::Num(server_seq as f64)),
        ("type".to_string(), Json::Str("event".to_string())),
        ("event".to_string(), Json::Str(event.to_string())),
        ("body".to_string(), body),
    ])
}

fn value_preview(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::BigInt(b) => b.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.to_string(),
        Value::Table(t) => format!("table({})", t.borrow().array.len()),
        Value::Closure(c) => format!(
            "function({})",
            c.proto.name.as_deref().unwrap_or("anonymous")
        ),
        Value::Native(def) => format!("native({})", def.name),
        Value::Buffer(b) => format!("buffer({})", b.borrow().len()),
    }
}

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Nil => "nil",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::BigInt(_) | Value::Float(_) => "number",
        Value::String(_) => "string",
        Value::Table(_) => "table",
        Value::Closure(_) | Value::Native(_) => "function",
        Value::Buffer(_) => "buffer",
    }
}

/// Current source line of a frame, best effort (line tables may be stubs).
fn frame_line(vm: &VM, frame_idx: usize) -> u32 {
    vm.frames
        .get(frame_idx)
        .and_then(|f| f.closure.proto.lines.get(f.ip).copied())
        .unwrap_or(1)
}

/// Handle one parsed request. Pure dispatch over explicit borrows so the
/// same function serves the outer loop, the pause loop, and tests.
pub fn handle_request(vm: &mut VM, req: &Json) -> Control {
    let req_seq = req.get("seq").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let command = req
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let args = req.get("arguments").cloned().unwrap_or(Json::Null);

    // `launch` needs the whole VM (it executes code): handle first.
    if command == "launch" {
        return do_launch(vm, req_seq, &args);
    }
    if command == "disconnect" {
        let seq = next_server_seq(vm);
        if let Some(session) = vm.dap.as_mut() {
            session.terminated = true;
        }
        return Control::Respond(ok_response(req_seq, &command, seq, Json::Obj(vec![])));
    }

    // Everything below only touches disjoint VM fields.
    let has_session = vm.dap.is_some();
    if !has_session {
        return Control::Respond(err_response(
            req_seq,
            &command,
            0,
            "no debug session".to_string(),
        ));
    }
    match command.as_str() {
        "initialize" => {
            let seq = next_server_seq(vm);
            Control::Respond(ok_response(
                req_seq,
                &command,
                seq,
                Json::Obj(vec![(
                    "capabilities".to_string(),
                    Json::Obj(vec![
                        ("supportsFunctionBreakpoints".to_string(), Json::Bool(true)),
                        (
                            "supportsConfigurationDoneRequest".to_string(),
                            Json::Bool(true),
                        ),
                    ]),
                )]),
            ))
        }
        "setFunctionBreakpoints" => {
            let mut names = Vec::new();
            if let Json::Arr(items) = args
                .get("breakpoints")
                .cloned()
                .unwrap_or(Json::Arr(vec![]))
            {
                for item in items {
                    if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                        names.push(name.to_string());
                    }
                }
            }
            if let Some(session) = vm.dap.as_mut() {
                session.func_breakpoints = names.clone();
            }
            let seq = next_server_seq(vm);
            let list: Vec<Json> = names
                .iter()
                .map(|_| Json::Obj(vec![("verified".to_string(), Json::Bool(true))]))
                .collect();
            Control::Respond(ok_response(
                req_seq,
                &command,
                seq,
                Json::Obj(vec![("breakpoints".to_string(), Json::Arr(list))]),
            ))
        }
        "configurationDone" => {
            // Start the program (already validated by launch).
            let prog = vm.dap.as_ref().and_then(|s| s.launch_program.clone());
            match prog {
                Some(path) => {
                    let seq = next_server_seq(vm);
                    let out = ok_response(req_seq, &command, seq, Json::Obj(vec![]));
                    // Run now; breakpoints pause from inside.
                    if let Err(e) = run_program(vm, &path) {
                        if e == "terminated by debugger" {
                            return Control::Respond(out);
                        }
                        let eseq = next_server_seq(vm);
                        return Control::Respond(err_response(req_seq, &command, eseq, e));
                    }
                    let tseq = next_server_seq(vm);
                    emit_event(vm, "terminated", Json::Obj(vec![]), tseq);
                    Control::Respond(out)
                }
                None => {
                    let seq = next_server_seq(vm);
                    Control::Respond(err_response(
                        req_seq,
                        &command,
                        seq,
                        "no program launched".to_string(),
                    ))
                }
            }
        }
        "threads" => {
            let seq = next_server_seq(vm);
            Control::Respond(ok_response(
                req_seq,
                &command,
                seq,
                Json::Obj(vec![(
                    "threads".to_string(),
                    Json::Arr(vec![Json::Obj(vec![
                        ("id".to_string(), Json::Num(1.0)),
                        ("name".to_string(), Json::Str("main".to_string())),
                    ])]),
                )]),
            ))
        }
        "stackTrace" => {
            let thread_ok = args.get("threadId").and_then(|v| v.as_usize()).unwrap_or(1) == 1;
            if !thread_ok {
                let seq = next_server_seq(vm);
                return Control::Respond(err_response(
                    req_seq,
                    &command,
                    seq,
                    "unknown thread".to_string(),
                ));
            }
            let start = args
                .get("startFrame")
                .and_then(|v| v.as_usize())
                .unwrap_or(0);
            let levels = args.get("levels").and_then(|v| v.as_usize());
            // Top frame first, absolute ids.
            let total = vm.frames.len();
            let mut ids: Vec<usize> = (0..total).rev().collect();
            if start < ids.len() {
                ids = ids.split_off(start);
            } else {
                ids.clear();
            }
            if let Some(n) = levels {
                ids.truncate(n);
            }
            let mut out_frames = Vec::new();
            for id in ids {
                let (name, line) = match vm.frames.get(id) {
                    Some(f) => (
                        f.closure
                            .proto
                            .name
                            .clone()
                            .unwrap_or_else(|| "<anonymous>".to_string()),
                        frame_line(vm, id),
                    ),
                    None => ("<gone>".to_string(), 1),
                };
                out_frames.push(Json::Obj(vec![
                    ("id".to_string(), Json::Num(id as f64)),
                    ("name".to_string(), Json::Str(name)),
                    ("line".to_string(), Json::Num(line as f64)),
                    ("column".to_string(), Json::Num(1.0)),
                ]));
            }
            let seq = next_server_seq(vm);
            Control::Respond(ok_response(
                req_seq,
                &command,
                seq,
                Json::Obj(vec![
                    ("stackFrames".to_string(), Json::Arr(out_frames)),
                    ("totalFrames".to_string(), Json::Num(total as f64)),
                ]),
            ))
        }
        "scopes" => {
            let frame_id = args.get("frameId").and_then(|v| v.as_usize());
            let frame = frame_id.and_then(|id| vm.frames.get(id));
            match frame {
                None => {
                    let seq = next_server_seq(vm);
                    Control::Respond(err_response(
                        req_seq,
                        &command,
                        seq,
                        "unknown frame".to_string(),
                    ))
                }
                Some(_) => {
                    let seq = next_server_seq(vm);
                    // Locals ref is even, globals ref odd would collide;
                    // allocate two fresh refs per scopes call instead.
                    let locals_ref = alloc_scope_ref(vm, ScopeKind::Locals(frame_id.unwrap()));
                    let globals_ref = alloc_scope_ref(vm, ScopeKind::Globals);
                    Control::Respond(ok_response(
                        req_seq,
                        &command,
                        seq,
                        Json::Obj(vec![(
                            "scopes".to_string(),
                            Json::Arr(vec![
                                Json::Obj(vec![
                                    ("name".to_string(), Json::Str("Locals".to_string())),
                                    (
                                        "variablesReference".to_string(),
                                        Json::Num(locals_ref as f64),
                                    ),
                                ]),
                                Json::Obj(vec![
                                    ("name".to_string(), Json::Str("Globals".to_string())),
                                    (
                                        "variablesReference".to_string(),
                                        Json::Num(globals_ref as f64),
                                    ),
                                ]),
                            ]),
                        )]),
                    ))
                }
            }
        }
        "variables" => {
            let var_ref = args
                .get("variablesReference")
                .and_then(|v| v.as_usize())
                .unwrap_or(0);
            let start = args.get("start").and_then(|v| v.as_usize()).unwrap_or(0);
            let count = args.get("count").and_then(|v| v.as_usize());
            let body = expand_variables(vm, var_ref, start, count);
            let seq = next_server_seq(vm);
            Control::Respond(ok_response(req_seq, &command, seq, body))
        }
        "continue" => Control::Continue,
        "pause" | "next" | "stepIn" | "stepOut" => {
            let seq = next_server_seq(vm);
            Control::Respond(err_response(
                req_seq,
                &command,
                seq,
                "stepping is not supported (needs line hooks)".to_string(),
            ))
        }
        "evaluate" => {
            // Global-scope evaluation (locals of a paused frame are NOT
            // visible; breakpoints stay silent inside). Errors become
            // error responses; the paused program is unaffected.
            let expr = args
                .get("expression")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match do_evaluate(vm, expr) {
                Ok(body) => {
                    let seq = next_server_seq(vm);
                    Control::Respond(ok_response(req_seq, &command, seq, body))
                }
                Err(message) => {
                    let seq = next_server_seq(vm);
                    Control::Respond(err_response(req_seq, &command, seq, message))
                }
            }
        }
        other => {
            let seq = next_server_seq(vm);
            Control::Respond(err_response(
                req_seq,
                &command,
                seq,
                format!("unknown command '{}'", other),
            ))
        }
    }
}

/// What the dispatcher decided: answer now, or resume execution.
pub enum Control {
    Respond(Json),
    Continue,
}

fn next_server_seq(vm: &mut VM) -> u64 {
    match vm.dap.as_mut() {
        Some(s) => {
            let n = s.next_seq;
            s.next_seq += 1;
            n
        }
        None => 0,
    }
}

fn emit_event(vm: &mut VM, event: &str, body: Json, server_seq: u64) {
    let msg = event_envelope(server_seq, event, body);
    let bytes = dap_json::render_json(&msg).into_bytes();
    if let Some(session) = vm.dap.as_mut() {
        let _ = dap_json::write_message(&mut session.writer, &bytes);
    }
}

/// `launch` needs the whole VM: read, compile (same pipeline as `run`),
/// but execute on the session VM so breakpoints and hooks apply.
fn do_launch(vm: &mut VM, req_seq: f64, args: &Json) -> Control {
    let path = args
        .get("program")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if path.is_empty() {
        let seq = next_server_seq(vm);
        return Control::Respond(err_response(
            req_seq,
            "launch",
            seq,
            "launch needs a program path".to_string(),
        ));
    }
    if let Some(session) = vm.dap.as_mut() {
        session.launch_program = Some(path);
    }
    let seq = next_server_seq(vm);
    Control::Respond(ok_response(req_seq, "launch", seq, Json::Obj(vec![])))
}

fn run_program(vm: &mut VM, path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read {}: {}", path, e))?;
    if bytes.starts_with(crate::bytecode::MAGIC) {
        return crate::vm::execute_bytecode(&bytes).map(|_| ());
    }
    let source = String::from_utf8(bytes)
        .map_err(|_| format!("failed to read {}: file is not valid UTF-8", path))?;
    // Deliberately the direct compiler, not the IR pipeline: optimization
    // passes such as inlining remove the very call sites breakpoints trap
    // on. Debug sessions trade speed for faithful execution.
    let stmts = crate::compiler::compile_source(&source)?;
    let proto = crate::compiler::try_compile_to_proto(&stmts)?;
    // Fresh frames/stack like a new run, but keep globals, hook and
    // session state on this VM.
    vm.execute(proto).map(|_| ())
}

/// Evaluates one expression in global scope and describes the first
/// result for a DAP `evaluate` response. Tables get a variablesReference
/// so the client can expand them with `variables`.
fn do_evaluate(vm: &mut VM, expr: &str) -> Result<Json, String> {
    if expr.trim().is_empty() {
        return Err("empty expression".to_string());
    }
    let stmts = crate::compiler::compile_source(&format!("return ({})", expr))
        .map_err(|e| e.to_string())?;
    // Unchecked pipeline, like the REPL: runtime names resolve
    // dynamically instead of failing whole-program analysis.
    let proto = crate::compiler::compile_bundled_to_proto(&stmts)?;
    let func = Value::Closure(std::rc::Rc::new(crate::vm::value::VmClosure {
        proto: std::rc::Rc::new(proto),
        upvalues: Vec::new(),
    }));
    vm.in_eval = true;
    let res = vm.call_function(func, &[]);
    vm.in_eval = false;
    let values = res?;
    let first = values.into_iter().next().unwrap_or(Value::Nil);
    let (type_name, var_ref) = match &first {
        Value::Table(t) => {
            let id = match vm.dap.as_mut() {
                Some(session) => {
                    let id = session.next_var_ref;
                    session.next_var_ref += 1;
                    session.variables.insert(id, Value::Table(t.clone()));
                    id
                }
                None => 0,
            };
            ("table", id)
        }
        _ => (value_type_name(&first), 0),
    };
    Ok(Json::Obj(vec![
        ("result".to_string(), Json::Str(value_preview(&first))),
        ("type".to_string(), Json::Str(type_name.to_string())),
        ("variablesReference".to_string(), Json::Num(var_ref as f64)),
    ]))
}

#[derive(Clone, Copy)]
enum ScopeKind {
    Locals(usize),
    Globals,
}

/// Scope contents are materialized as synthetic tables so the variables
/// path stays uniform; the ref itself is registered as a scope marker.
fn alloc_scope_ref(vm: &mut VM, kind: ScopeKind) -> usize {
    let session = match vm.dap.as_mut() {
        Some(s) => s,
        None => return 0,
    };
    let id = session.next_var_ref;
    session.next_var_ref += 1;
    let marker = match kind {
        ScopeKind::Locals(frame_id) => Value::Int(-(frame_id as i64) - 1),
        ScopeKind::Globals => Value::Int(-999_999),
    };
    session.variables.insert(id, marker);
    id
}

fn expand_variables(vm: &mut VM, var_ref: usize, start: usize, count: Option<usize>) -> Json {
    // Resolve scope markers without holding borrows across registration.
    enum Resolved {
        Locals(usize),
        Globals,
        Table(std::rc::Rc<std::cell::RefCell<crate::vm::value::VmTable>>),
        Missing,
    }
    let resolved = match vm
        .dap
        .as_ref()
        .and_then(|s| s.variables.get(&var_ref).cloned())
    {
        Some(Value::Int(n)) if n < 0 => {
            if n == -999_999 {
                Resolved::Globals
            } else {
                Resolved::Locals((-n - 1) as usize)
            }
        }
        Some(Value::Table(t)) => Resolved::Table(t),
        _ => Resolved::Missing,
    };
    // Collect (name, value) pairs.
    let mut pairs: Vec<(String, Value)> = Vec::new();
    match resolved {
        Resolved::Missing => {
            return Json::Obj(vec![("variables".to_string(), Json::Arr(vec![]))]);
        }
        Resolved::Globals => {
            let mut names: Vec<String> = vm.globals.keys().map(|k| k.to_string()).collect();
            names.sort();
            for name in names {
                if let Some(v) = vm.globals.get(name.as_str()).cloned() {
                    pairs.push((name, v));
                }
            }
        }
        Resolved::Locals(frame_id) => {
            if let Some(frame) = vm.frames.get(frame_id) {
                let current_ip = frame.ip as u32;
                for info in &frame.closure.proto.local_names {
                    if current_ip >= info.from_pc && current_ip <= info.to_pc {
                        let slot = frame.base + info.reg as usize;
                        let val = vm.stack.get(slot).cloned().unwrap_or(Value::Nil);
                        pairs.push((info.name.clone(), val));
                    }
                }
            }
        }
        Resolved::Table(t) => {
            let tbl = t.borrow();
            for (i, v) in tbl.array.iter().enumerate() {
                pairs.push(((i + 1).to_string(), v.clone()));
            }
            let mut keys: Vec<String> = tbl.fields.keys().map(|k| k.to_string()).collect();
            keys.sort();
            for k in keys {
                if let Some(v) = tbl.fields.get(k.as_str()).cloned() {
                    pairs.push((k, v));
                }
            }
        }
    }
    // Page.
    let total = pairs.len();
    let end = match count {
        Some(n) => (start + n).min(total),
        None => total,
    };
    let start = start.min(total);
    // Register compound children, then render.
    let mut out = Vec::new();
    // Borrow session once for the whole batch.
    let var_refs: Vec<(String, Value, usize)> = {
        let has_session = vm.dap.is_some();
        pairs
            .into_iter()
            .skip(start)
            .take(end - start)
            .map(|(name, v)| {
                let child_ref = match &v {
                    Value::Table(_) if has_session => {
                        let s = vm.dap.as_mut().expect("session");
                        let id = s.next_var_ref;
                        s.next_var_ref += 1;
                        s.variables.insert(id, v.clone());
                        id
                    }
                    _ => 0,
                };
                (name, v, child_ref)
            })
            .collect()
    };
    for (name, v, child_ref) in var_refs {
        out.push(Json::Obj(vec![
            ("name".to_string(), Json::Str(name)),
            ("value".to_string(), Json::Str(value_preview(&v))),
            (
                "type".to_string(),
                Json::Str(value_type_name(&v).to_string()),
            ),
            (
                "variablesReference".to_string(),
                Json::Num(child_ref as f64),
            ),
        ]));
    }
    Json::Obj(vec![("variables".to_string(), Json::Arr(out))])
}

/// Entry point for `neyuki dap`: raw DAP conversation on stdio.
pub fn run_dap_session() -> Result<(), String> {
    let mut vm = VM::new();
    vm.dap = Some(DapSession::stdio());
    loop {
        let raw = read_next()?;
        let Some(bytes) = raw else { break };
        let req = match dap_json::parse_json(&bytes) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match handle_request(&mut vm, &req) {
            Control::Respond(resp) => {
                write_out(&mut vm, &resp)?;
            }
            Control::Continue => {
                // `continue` outside a pause: nothing is running.
                let seq = next_server_seq(&mut vm);
                let resp = err_response(0.0, "continue", seq, "nothing is paused".to_string());
                write_out(&mut vm, &resp)?;
            }
        }
        let done = vm.dap.as_ref().map(|s| s.terminated).unwrap_or(true);
        if done {
            break;
        }
    }
    Ok(())
}

fn read_next() -> Result<Option<Vec<u8>>, String> {
    let stdin = io::stdin();
    let mut lock = stdin.lock();
    dap_json::read_message(&mut lock).map_err(|e| e.to_string())
}

fn write_out(vm: &mut VM, resp: &Json) -> Result<(), String> {
    let bytes = dap_json::render_json(resp).into_bytes();
    match vm.dap.as_mut() {
        Some(session) => {
            dap_json::write_message(&mut session.writer, &bytes).map_err(|e| e.to_string())
        }
        None => Ok(()),
    }
}

/// Called from closure-call sites when breakpoints are armed. Sends
/// `stopped(reason: function breakpoint)` and serves requests from a
/// nested loop until `continue` (resume) or `disconnect` (abort with an
/// error that unwinds execution).
pub(crate) fn check_break(vm: &mut VM, func_name: &str) -> Result<(), String> {
    if vm.in_eval {
        return Ok(());
    }
    let armed = match &vm.dap {
        Some(s) if !s.func_breakpoints.is_empty() => {
            s.func_breakpoints.iter().any(|b| b == func_name)
        }
        _ => false,
    };
    if !armed {
        return Ok(());
    }
    let seq = next_server_seq(vm);
    emit_event(
        vm,
        "stopped",
        Json::Obj(vec![
            (
                "reason".to_string(),
                Json::Str("function breakpoint".to_string()),
            ),
            ("threadId".to_string(), Json::Num(1.0)),
        ]),
        seq,
    );
    loop {
        // Read the next request from the SESSION stream (never global
        // stdin: tests and embeddings supply their own).
        let raw = {
            let session = match vm.dap.as_mut() {
                Some(s) => s,
                None => return Err("debugging terminated".to_string()),
            };
            match dap_json::read_message(&mut session.reader) {
                Ok(v) => v,
                Err(e) => return Err(e.to_string()),
            }
        };
        let Some(bytes) = raw else {
            if let Some(s) = vm.dap.as_mut() {
                s.terminated = true;
            }
            return Err("debugging terminated".to_string());
        };
        let req = match dap_json::parse_json(&bytes) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // `continue` resumes; `disconnect` aborts the run.
        let is_continue = req.get("command").and_then(|v| v.as_str()) == Some("continue");
        let is_disconnect = req.get("command").and_then(|v| v.as_str()) == Some("disconnect");
        match handle_request(vm, &req) {
            Control::Respond(resp) => {
                write_out(vm, &resp)?;
            }
            Control::Continue => {
                // The dispatcher stays silent for `continue` by design, but
                // DAP requires every request to be answered: respond here.
                let rseq = req.get("seq").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let sseq = next_server_seq(vm);
                write_out(vm, &ok_response(rseq, "continue", sseq, Json::Obj(vec![])))?;
            }
        }
        if is_disconnect {
            if let Some(s) = vm.dap.as_mut() {
                s.terminated = true;
            }
            return Err("debugging terminated".to_string());
        }
        if is_continue {
            let cseq = next_server_seq(vm);
            emit_event(
                vm,
                "continued",
                Json::Obj(vec![("threadId".to_string(), Json::Num(1.0))]),
                cseq,
            );
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp_script(name: &str, src: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("neyuki_dap_{}_{}.nyk", name, std::process::id()));
        std::fs::write(&p, src).expect("temp script");
        p
    }

    #[test]
    fn dap_initialize_reports_capabilities() {
        let mut vm = VM::new();
        vm.dap = Some(DapSession {
            reader: Box::new(io::Cursor::new(Vec::new())),
            writer: Box::new(io::Cursor::new(Vec::new())),
            func_breakpoints: Vec::new(),
            variables: HashMap::new(),
            next_var_ref: 1,
            next_seq: 1,
            terminated: false,
            launch_program: None,
        });
        let req = dap_json::parse_json(
            br#"{"seq":1,"type":"request","command":"initialize","arguments":{"adapterID":"neyuki"}}"#,
        )
        .expect("req");
        let resp = match handle_request(&mut vm, &req) {
            Control::Respond(r) => r,
            Control::Continue => panic!("unexpected continue"),
        };
        assert_eq!(resp.get("success"), Some(&Json::Bool(true)));
        let caps = resp
            .get("body")
            .and_then(|b| b.get("capabilities"))
            .expect("capabilities");
        assert_eq!(
            caps.get("supportsFunctionBreakpoints"),
            Some(&Json::Bool(true))
        );
    }

    fn eval_body(vm: &mut VM, setup: &str, expr: &str) -> Vec<(String, Json)> {
        if !setup.is_empty() {
            let stmts = crate::compiler::compile_source(setup).expect("setup parse");
            let proto = crate::compiler::compile_bundled_to_proto(&stmts).expect("setup compile");
            vm.execute(proto).expect("setup exec");
        }
        match do_evaluate(vm, expr) {
            Ok(Json::Obj(pairs)) => pairs,
            other => panic!("evaluate failed: {:?}", other),
        }
    }

    fn pair_value(pairs: &[(String, Json)], key: &str) -> String {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| dap_json::render_json(v))
            .unwrap_or_else(|| panic!("missing key {}", key))
    }

    #[test]
    fn dap_evaluate_arithmetic() {
        let mut vm = VM::new();
        let pairs = eval_body(&mut vm, "", "40 + 2");
        assert_eq!(pair_value(&pairs, "result"), "\"42\"");
        assert_eq!(pair_value(&pairs, "type"), "\"number\"");
    }

    #[test]
    fn dap_evaluate_global() {
        let mut vm = VM::new();
        let pairs = eval_body(&mut vm, "eval_xyz = 41", "eval_xyz + 1");
        assert_eq!(pair_value(&pairs, "result"), "\"42\"");
    }

    #[test]
    fn dap_evaluate_error_keeps_session() {
        let mut vm = VM::new();
        assert!(do_evaluate(&mut vm, "1 +").is_err());
        // Session still functional afterwards.
        let pairs = eval_body(&mut vm, "", "2 * 3");
        assert_eq!(pair_value(&pairs, "result"), "\"6\"");
    }

    #[test]
    fn dap_evaluate_skips_breakpoints() {
        // A breakpoint on `f` must NOT pause evaluation itself: without
        // the in_eval guard this would read an empty stream and abort.
        let mut vm = VM::new();
        let (session, _out) = DapSession::memory(Vec::new());
        vm.dap = Some(session);
        if let Some(s) = vm.dap.as_mut() {
            s.func_breakpoints.push("f".to_string());
        }
        let pairs = eval_body(&mut vm, "f = function(a) return a * 2 end", "f(21)");
        assert_eq!(pair_value(&pairs, "result"), "\"42\"");
    }

    #[test]
    fn dap_breakpoint_pause_flow() {
        let path = write_temp_script(
            "pause",
            "local function add(x, y)\n  return x + y\nend\nlocal s = add(20, 22)\nreturn s\n",
        );
        let launch = format!(
            "{{\"seq\":2,\"type\":\"request\",\"command\":\"launch\",\"arguments\":{{\"program\":{}}}}}",
            dap_json::render_json(&Json::Str(path.to_string_lossy().to_string()))
        );
        let convo = vec![
            r#"{"seq":1,"type":"request","command":"initialize","arguments":{}}"#,
            launch.as_str(),
            r#"{"seq":3,"type":"request","command":"setFunctionBreakpoints","arguments":{"breakpoints":[{"name":"add"}]}}"#,
            r#"{"seq":4,"type":"request","command":"configurationDone"}"#,
            r#"{"seq":5,"type":"request","command":"threads"}"#,
            r#"{"seq":6,"type":"request","command":"stackTrace","arguments":{"threadId":1}}"#,
            r#"{"seq":7,"type":"request","command":"scopes","arguments":{"frameId":1}}"#,
            // Locals is the first allocated ref (id 1): nothing else
            // allocates variable references before the scopes call.
            r#"{"seq":8,"type":"request","command":"variables","arguments":{"variablesReference":1}}"#,
            r#"{"seq":9,"type":"request","command":"continue","arguments":{"threadId":1}}"#,
        ];
        let mut framed_in = Vec::new();
        for m in &convo {
            framed_in.extend_from_slice(format!("Content-Length: {}\r\n\r\n", m.len()).as_bytes());
            framed_in.extend_from_slice(m.as_bytes());
        }
        let mut vm = VM::new();
        let (session, out_buf) = DapSession::memory(framed_in);
        vm.dap = Some(session);
        // Drive the whole conversation, including the nested pause loop
        // that configurationDone triggers via the breakpoint.
        loop {
            let raw = {
                let s = vm.dap.as_mut().expect("session");
                match dap_json::read_message(&mut s.reader) {
                    Ok(v) => v,
                    Err(e) => panic!("read: {}", e),
                }
            };
            let Some(bytes) = raw else { break };
            let req = dap_json::parse_json(&bytes).expect("parse");
            match handle_request(&mut vm, &req) {
                Control::Respond(resp) => {
                    let out = dap_json::render_json(&resp).into_bytes();
                    let s = vm.dap.as_mut().expect("session");
                    dap_json::write_message(&mut s.writer, &out).expect("write");
                }
                Control::Continue => {}
            }
            if vm.dap.as_ref().map(|s| s.terminated).unwrap_or(true) {
                break;
            }
        }
        // Split the output stream back into messages.
        let raw_out = out_buf.lock().unwrap().clone();
        let mut cursor = io::Cursor::new(raw_out);
        let mut messages = Vec::new();
        while let Some(body) = dap_json::read_message(&mut cursor).expect("read out") {
            messages.push(dap_json::parse_json(&body).expect("parse out"));
        }
        let find_response = |seq: f64| {
            messages
                .iter()
                .find(|m| {
                    m.get("type").and_then(|v| v.as_str()) == Some("response")
                        && m.get("request_seq").and_then(|v| v.as_f64()) == Some(seq)
                })
                .unwrap_or_else(|| panic!("missing response {}", seq))
                .clone()
        };
        // Paused at `add`: a stopped event fired.
        assert!(
            messages.iter().any(|m| {
                m.get("type").and_then(|v| v.as_str()) == Some("event")
                    && m.get("event").and_then(|v| v.as_str()) == Some("stopped")
            }),
            "expected a stopped event"
        );
        // Innermost frame is `add`.
        let st = find_response(6.0);
        let frames = st
            .get("body")
            .and_then(|b| b.get("stackFrames"))
            .expect("stackFrames");
        let first_name = match frames {
            Json::Arr(items) => items
                .first()
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            _ => panic!("stackFrames not an array"),
        };
        assert_eq!(first_name, "add");
        // Locals scope exposes x = 20 (and the ref it reported is 1).
        let scopes = find_response(7.0);
        let locals_ref = match scopes.get("body").and_then(|b| b.get("scopes")) {
            Some(Json::Arr(items)) => items
                .first()
                .and_then(|s| s.get("variablesReference"))
                .and_then(|v| v.as_usize())
                .expect("locals ref"),
            _ => panic!("scopes not an array"),
        };
        assert_eq!(locals_ref, 1);
        let vars = find_response(8.0);
        let rendered = dap_json::render_json(&vars);
        assert!(
            rendered.contains("\"x\"") && rendered.contains("20"),
            "locals must show x = 20: {}",
            rendered
        );
        // Ran to completion afterwards.
        assert!(
            messages.iter().any(|m| {
                m.get("type").and_then(|v| v.as_str()) == Some("event")
                    && m.get("event").and_then(|v| v.as_str()) == Some("terminated")
            }),
            "expected a terminated event"
        );
        // Every request gets a response, including `continue`.
        let cont = find_response(9.0);
        assert_eq!(cont.get("success"), Some(&Json::Bool(true)));
    }
}
