//! Language Server Protocol (LSP) server for `neyuki lsp`, running over stdio.

use std::collections::HashMap;
use std::io::{self, BufReader, Write};

use crate::dap_json::{self, Json};
use crate::diagnostics::severity::Severity;

pub struct LspServer {
    pub documents: HashMap<String, String>,
    pub is_shutdown: bool,
    pub is_exit: bool,
    pub exit_code: Option<i32>,
}

impl Default for LspServer {
    fn default() -> Self {
        Self::new()
    }
}

impl LspServer {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            is_shutdown: false,
            is_exit: false,
            exit_code: None,
        }
    }

    pub fn handle_message(&mut self, msg_bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let json = match dap_json::parse_json(msg_bytes) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("LSP parse error: {}", e);
                let err_resp = json_error(Json::Null, -32700, "Parse error");
                let bytes = dap_json::render_json(&err_resp).into_bytes();
                return Ok(Some(bytes));
            }
        };

        let method = json.get("method").and_then(|m| m.as_str());
        let id = json.get("id");

        if let Some(method_name) = method {
            // Check for requests arriving after shutdown
            if self.is_shutdown && method_name != "exit" {
                if let Some(req_id) = id {
                    let err_resp = json_error(
                        req_id.clone(),
                        -32600,
                        "Invalid Request: server is shutting down",
                    );
                    let bytes = dap_json::render_json(&err_resp).into_bytes();
                    return Ok(Some(bytes));
                }
                return Ok(None);
            }

            match method_name {
                "initialize" => {
                    let req_id = id.cloned().unwrap_or(Json::Num(1.0));
                    let resp = json_response(
                        req_id,
                        Json::Obj(vec![(
                            "capabilities".to_string(),
                            Json::Obj(vec![
                                ("textDocumentSync".to_string(), Json::Num(1.0)),
                                ("documentFormattingProvider".to_string(), Json::Bool(true)),
                                ("hoverProvider".to_string(), Json::Bool(true)),
                                (
                                    "completionProvider".to_string(),
                                    Json::Obj(vec![(
                                        "triggerCharacters".to_string(),
                                        Json::Arr(vec![
                                            Json::Str(".".to_string()),
                                            Json::Str(":".to_string()),
                                        ]),
                                    )]),
                                ),
                                ("definitionProvider".to_string(), Json::Bool(true)),
                                ("documentSymbolProvider".to_string(), Json::Bool(true)),
                            ]),
                        )]),
                    );
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                "initialized" => {
                    // Notification, no response needed
                    return Ok(None);
                }
                "shutdown" => {
                    self.is_shutdown = true;
                    let req_id = id.cloned().unwrap_or(Json::Null);
                    let resp = json_response(req_id, Json::Null);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                "exit" => {
                    self.is_exit = true;
                    self.exit_code = Some(if self.is_shutdown { 0 } else { 1 });
                    return Ok(None);
                }
                "textDocument/didOpen" => {
                    if let Some(params) = json.get("params")
                        && let Some(doc) = params.get("textDocument")
                    {
                        let uri = doc
                            .get("uri")
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string();
                        let content = doc
                            .get("text")
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string();
                        self.documents.insert(uri.clone(), content.clone());
                        let notif = create_publish_diagnostics(&uri, &content);
                        let bytes = dap_json::render_json(&notif).into_bytes();
                        return Ok(Some(bytes));
                    }
                    return Ok(None);
                }
                "textDocument/didChange" => {
                    if let Some(params) = json.get("params") {
                        let uri = params
                            .get("textDocument")
                            .and_then(|d| d.get("uri"))
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string();
                        // With TextDocumentSyncKind::Full (1), the client sends the entire document in contentChanges[0].
                        if let Some(Json::Arr(changes)) = params.get("contentChanges")
                            && let Some(first) = changes.first()
                            && let Some(new_text) = first.get("text").and_then(|t| t.as_str())
                        {
                            self.documents.insert(uri.clone(), new_text.to_string());
                            let notif = create_publish_diagnostics(&uri, new_text);
                            let bytes = dap_json::render_json(&notif).into_bytes();
                            return Ok(Some(bytes));
                        }
                    }
                    return Ok(None);
                }
                "textDocument/didClose" => {
                    if let Some(params) = json.get("params")
                        && let Some(doc) = params.get("textDocument")
                        && let Some(uri) = doc.get("uri").and_then(|u| u.as_str())
                    {
                        self.documents.remove(uri);
                        let notif = create_empty_publish_diagnostics(uri);
                        let bytes = dap_json::render_json(&notif).into_bytes();
                        return Ok(Some(bytes));
                    }
                    return Ok(None);
                }
                "textDocument/didSave" => {
                    if let Some(params) = json.get("params")
                        && let Some(doc) = params.get("textDocument")
                        && let Some(uri) = doc.get("uri").and_then(|u| u.as_str())
                    {
                        if let Some(text) = params.get("text").and_then(|t| t.as_str()) {
                            self.documents.insert(uri.to_string(), text.to_string());
                        }
                        if let Some(content) = self.documents.get(uri) {
                            let notif = create_publish_diagnostics(uri, content);
                            let bytes = dap_json::render_json(&notif).into_bytes();
                            return Ok(Some(bytes));
                        }
                    }
                    return Ok(None);
                }
                "textDocument/formatting" => {
                    let req_id = id.cloned().unwrap_or(Json::Null);
                    let uri = json
                        .get("params")
                        .and_then(|p| p.get("textDocument"))
                        .and_then(|d| d.get("uri"))
                        .and_then(|u| u.as_str())
                        .unwrap_or("");
                    let edits = if let Some(content) = self.documents.get(uri) {
                        if let Ok(formatted) = crate::fmt::format_source(content) {
                            let line_count = content.lines().count();
                            let (end_line, end_char) = if content.ends_with('\n')
                                || content.ends_with("\r\n")
                            {
                                (line_count as f64, 0.0)
                            } else {
                                let last_line = content.lines().last().unwrap_or("");
                                let last_len =
                                    last_line.chars().map(|c| c.len_utf16()).sum::<usize>() as f64;
                                (line_count.saturating_sub(1) as f64, last_len)
                            };

                            Json::Arr(vec![Json::Obj(vec![
                                (
                                    "range".to_string(),
                                    Json::Obj(vec![
                                        (
                                            "start".to_string(),
                                            Json::Obj(vec![
                                                ("line".to_string(), Json::Num(0.0)),
                                                ("character".to_string(), Json::Num(0.0)),
                                            ]),
                                        ),
                                        (
                                            "end".to_string(),
                                            Json::Obj(vec![
                                                ("line".to_string(), Json::Num(end_line)),
                                                ("character".to_string(), Json::Num(end_char)),
                                            ]),
                                        ),
                                    ]),
                                ),
                                ("newText".to_string(), Json::Str(formatted)),
                            ])])
                        } else {
                            Json::Arr(Vec::new())
                        }
                    } else {
                        Json::Arr(Vec::new())
                    };

                    let resp = json_response(req_id, edits);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                "textDocument/hover" => {
                    let req_id = id.cloned().unwrap_or(Json::Null);
                    let uri = json
                        .get("params")
                        .and_then(|p| p.get("textDocument"))
                        .and_then(|d| d.get("uri"))
                        .and_then(|u| u.as_str())
                        .unwrap_or("");
                    let line = json
                        .get("params")
                        .and_then(|p| p.get("position"))
                        .and_then(|pos| pos.get("line"))
                        .and_then(|l| l.as_usize())
                        .unwrap_or(0);
                    let character = json
                        .get("params")
                        .and_then(|p| p.get("position"))
                        .and_then(|pos| pos.get("character"))
                        .and_then(|c| c.as_usize())
                        .unwrap_or(0);

                    let result = if let Some(content) = self.documents.get(uri) {
                        handle_hover(content, line, character)
                    } else {
                        Json::Null
                    };

                    let resp = json_response(req_id, result);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                "textDocument/completion" => {
                    let req_id = id.cloned().unwrap_or(Json::Null);
                    let uri = json
                        .get("params")
                        .and_then(|p| p.get("textDocument"))
                        .and_then(|d| d.get("uri"))
                        .and_then(|u| u.as_str())
                        .unwrap_or("");
                    let line = json
                        .get("params")
                        .and_then(|p| p.get("position"))
                        .and_then(|pos| pos.get("line"))
                        .and_then(|l| l.as_usize())
                        .unwrap_or(0);
                    let character = json
                        .get("params")
                        .and_then(|p| p.get("position"))
                        .and_then(|pos| pos.get("character"))
                        .and_then(|c| c.as_usize())
                        .unwrap_or(0);

                    let items = if let Some(content) = self.documents.get(uri) {
                        handle_completion(content, line, character)
                    } else {
                        Json::Arr(Vec::new())
                    };

                    let resp = json_response(req_id, items);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                "textDocument/definition" => {
                    let req_id = id.cloned().unwrap_or(Json::Null);
                    let uri = json
                        .get("params")
                        .and_then(|p| p.get("textDocument"))
                        .and_then(|d| d.get("uri"))
                        .and_then(|u| u.as_str())
                        .unwrap_or("");
                    let line = json
                        .get("params")
                        .and_then(|p| p.get("position"))
                        .and_then(|pos| pos.get("line"))
                        .and_then(|l| l.as_usize())
                        .unwrap_or(0);
                    let character = json
                        .get("params")
                        .and_then(|p| p.get("position"))
                        .and_then(|pos| pos.get("character"))
                        .and_then(|c| c.as_usize())
                        .unwrap_or(0);

                    let result = if let Some(content) = self.documents.get(uri) {
                        handle_definition(uri, content, line, character)
                    } else {
                        Json::Null
                    };

                    let resp = json_response(req_id, result);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                "textDocument/documentSymbol" => {
                    let req_id = id.cloned().unwrap_or(Json::Null);
                    let uri = json
                        .get("params")
                        .and_then(|p| p.get("textDocument"))
                        .and_then(|d| d.get("uri"))
                        .and_then(|u| u.as_str())
                        .unwrap_or("");

                    let result = if let Some(content) = self.documents.get(uri) {
                        handle_document_symbols(content)
                    } else {
                        Json::Arr(Vec::new())
                    };

                    let resp = json_response(req_id, result);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                _ => {
                    if let Some(req_id) = id {
                        eprintln!("LSP unknown method: {}", method_name);
                        let resp = json_error(
                            req_id.clone(),
                            -32601,
                            &format!("Method '{}' not found", method_name),
                        );
                        let bytes = dap_json::render_json(&resp).into_bytes();
                        return Ok(Some(bytes));
                    }
                }
            }
        } else if let Some(req_id) = id {
            let resp = json_error(req_id.clone(), -32600, "Invalid Request");
            let bytes = dap_json::render_json(&resp).into_bytes();
            return Ok(Some(bytes));
        }

        Ok(None)
    }
}

fn json_response(id: Json, result: Json) -> Json {
    Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        ("id".to_string(), id),
        ("result".to_string(), result),
    ])
}

fn json_error(id: Json, code: i64, message: &str) -> Json {
    Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        ("id".to_string(), id),
        (
            "error".to_string(),
            Json::Obj(vec![
                ("code".to_string(), Json::Num(code as f64)),
                ("message".to_string(), Json::Str(message.to_string())),
            ]),
        ),
    ])
}

fn json_notification(method: &str, params: Json) -> Json {
    Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        ("method".to_string(), Json::Str(method.to_string())),
        ("params".to_string(), params),
    ])
}

/// Converts a 0-based character column to a 0-based UTF-16 code unit offset.
fn char_col_to_utf16_col(line: &str, char_col_0_based: usize) -> usize {
    line.chars()
        .take(char_col_0_based)
        .map(|c| c.len_utf16())
        .sum()
}

fn create_publish_diagnostics(uri: &str, content: &str) -> Json {
    let diags = crate::lint::lint_source(content);
    let mut diag_items = Vec::new();
    let doc_lines: Vec<&str> = content.lines().collect();

    for d in diags {
        let (start_line, start_char, end_line, end_char) = if let Some(ref l) = d.primary_label {
            let s_line = l.span.start.line.saturating_sub(1) as usize;
            let s_col = l.span.start.column.saturating_sub(1) as usize;
            let s_char = doc_lines
                .get(s_line)
                .map(|line| char_col_to_utf16_col(line, s_col))
                .unwrap_or(s_col);

            let e_line = l.span.end.line.saturating_sub(1) as usize;
            let e_col = l.span.end.column.saturating_sub(1) as usize;
            let e_char = doc_lines
                .get(e_line)
                .map(|line| char_col_to_utf16_col(line, e_col))
                .unwrap_or(e_col);

            (s_line as f64, s_char as f64, e_line as f64, e_char as f64)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };

        let severity = if d.severity == Severity::Error {
            1.0 // DiagnosticSeverity.Error
        } else {
            2.0 // DiagnosticSeverity.Warning
        };

        let mut fields = vec![
            (
                "range".to_string(),
                Json::Obj(vec![
                    (
                        "start".to_string(),
                        Json::Obj(vec![
                            ("line".to_string(), Json::Num(start_line)),
                            ("character".to_string(), Json::Num(start_char)),
                        ]),
                    ),
                    (
                        "end".to_string(),
                        Json::Obj(vec![
                            ("line".to_string(), Json::Num(end_line.max(start_line))),
                            (
                                "character".to_string(),
                                Json::Num(end_char.max(start_char + 1.0)),
                            ),
                        ]),
                    ),
                ]),
            ),
            ("severity".to_string(), Json::Num(severity)),
            ("message".to_string(), Json::Str(d.message)),
            ("source".to_string(), Json::Str("neyuki".to_string())),
        ];

        if let Some(code) = d.code {
            fields.push(("code".to_string(), Json::Str(code.as_str().to_string())));
        }

        diag_items.push(Json::Obj(fields));
    }

    json_notification(
        "textDocument/publishDiagnostics",
        Json::Obj(vec![
            ("uri".to_string(), Json::Str(uri.to_string())),
            ("diagnostics".to_string(), Json::Arr(diag_items)),
        ]),
    )
}

fn create_empty_publish_diagnostics(uri: &str) -> Json {
    json_notification(
        "textDocument/publishDiagnostics",
        Json::Obj(vec![
            ("uri".to_string(), Json::Str(uri.to_string())),
            ("diagnostics".to_string(), Json::Arr(Vec::new())),
        ]),
    )
}

fn get_word_at_position(content: &str, line_idx: usize, char_col: usize) -> Option<String> {
    let line = content.lines().nth(line_idx)?;
    let mut current_utf16_col = 0;
    let mut char_indices = Vec::new();

    for (byte_idx, ch) in line.char_indices() {
        let ch_utf16_len = ch.len_utf16();
        char_indices.push((
            byte_idx,
            ch,
            current_utf16_col,
            current_utf16_col + ch_utf16_len,
        ));
        current_utf16_col += ch_utf16_len;
    }

    let target = char_indices
        .iter()
        .find(|(_, _, start, end)| char_col >= *start && char_col <= *end)?;

    if !target.1.is_ascii_alphanumeric() && target.1 != '_' {
        return None;
    }

    let byte_pos = target.0;
    let bytes = line.as_bytes();
    let mut start = byte_pos;
    while start > 0
        && (bytes[start - 1].is_ascii_alphanumeric()
            || bytes[start - 1] == b'_'
            || bytes[start - 1] == b'.')
    {
        start -= 1;
    }
    let mut end = byte_pos + target.1.len_utf8();
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'.')
    {
        end += 1;
    }

    let word = std::str::from_utf8(&bytes[start..end])
        .ok()?
        .trim_matches('.');
    if word.is_empty() {
        None
    } else {
        Some(word.to_string())
    }
}

fn handle_hover(content: &str, line: usize, character: usize) -> Json {
    if let Some(word) = get_word_at_position(content, line, character) {
        if let Some((sig, desc)) = lookup_docs(&word) {
            let markdown = format!("```neyuki\n{}\n```\n{}", sig, desc);
            return Json::Obj(vec![(
                "contents".to_string(),
                Json::Obj(vec![
                    ("kind".to_string(), Json::Str("markdown".to_string())),
                    ("value".to_string(), Json::Str(markdown)),
                ]),
            )]);
        }

        let symbols = extract_document_symbols(content);
        if let Some(sym) = symbols.iter().find(|s| s.name == word) {
            let markdown = format!("```neyuki\n{}\n```\n*Defined in this document*", sym.detail);
            return Json::Obj(vec![(
                "contents".to_string(),
                Json::Obj(vec![
                    ("kind".to_string(), Json::Str("markdown".to_string())),
                    ("value".to_string(), Json::Str(markdown)),
                ]),
            )]);
        }
    }
    Json::Null
}

fn handle_definition(uri: &str, content: &str, line: usize, character: usize) -> Json {
    if let Some(word) = get_word_at_position(content, line, character) {
        let symbols = extract_document_symbols(content);
        if let Some(sym) = symbols.iter().find(|s| s.name == word) {
            let line_0 = sym.line_1_based.saturating_sub(1) as f64;
            let doc_lines: Vec<&str> = content.lines().collect();
            let line_text = doc_lines
                .get(sym.line_1_based.saturating_sub(1) as usize)
                .unwrap_or(&"");
            let char_offset = line_text.find(&word).unwrap_or(0);
            let char_utf16 = char_col_to_utf16_col(line_text, char_offset) as f64;
            let end_utf16 = char_utf16 + word.chars().map(|c| c.len_utf16()).sum::<usize>() as f64;

            return Json::Obj(vec![
                ("uri".to_string(), Json::Str(uri.to_string())),
                (
                    "range".to_string(),
                    Json::Obj(vec![
                        (
                            "start".to_string(),
                            Json::Obj(vec![
                                ("line".to_string(), Json::Num(line_0)),
                                ("character".to_string(), Json::Num(char_utf16)),
                            ]),
                        ),
                        (
                            "end".to_string(),
                            Json::Obj(vec![
                                ("line".to_string(), Json::Num(line_0)),
                                ("character".to_string(), Json::Num(end_utf16)),
                            ]),
                        ),
                    ]),
                ),
            ]);
        }
    }
    Json::Null
}

fn handle_document_symbols(content: &str) -> Json {
    let symbols = extract_document_symbols(content);
    let mut symbol_objs = Vec::new();

    for sym in symbols {
        let line_0 = sym.line_1_based.saturating_sub(1) as f64;
        let range = Json::Obj(vec![
            (
                "start".to_string(),
                Json::Obj(vec![
                    ("line".to_string(), Json::Num(line_0)),
                    ("character".to_string(), Json::Num(0.0)),
                ]),
            ),
            (
                "end".to_string(),
                Json::Obj(vec![
                    ("line".to_string(), Json::Num(line_0)),
                    ("character".to_string(), Json::Num(40.0)),
                ]),
            ),
        ]);

        symbol_objs.push(Json::Obj(vec![
            ("name".to_string(), Json::Str(sym.name)),
            ("detail".to_string(), Json::Str(sym.detail)),
            ("kind".to_string(), Json::Num(sym.kind as f64)),
            ("range".to_string(), range.clone()),
            ("selectionRange".to_string(), range),
        ]));
    }

    Json::Arr(symbol_objs)
}

fn handle_completion(content: &str, line: usize, character: usize) -> Json {
    let doc_lines: Vec<&str> = content.lines().collect();
    let line_str = doc_lines.get(line).unwrap_or(&"");
    let prefix = if character <= line_str.len() {
        &line_str[..character]
    } else {
        line_str
    };

    let mut items = Vec::new();

    if prefix.ends_with("os.") {
        for (name, detail) in &[
            ("run", "os.run(command, options)"),
            ("exit", "os.exit(code)"),
            ("getenv", "os.getenv(name)"),
            ("time", "os.time()"),
            ("clock", "os.clock()"),
        ] {
            items.push(make_completion_item(name, detail, 3)); // Function
        }
    } else if prefix.ends_with("string.") {
        for (name, detail) in &[
            ("sub", "string.sub(s, i, j)"),
            ("format", "string.format(fmt, ...)"),
            ("find", "string.find(s, pattern)"),
            ("match", "string.match(s, pattern)"),
            ("gmatch", "string.gmatch(s, pattern)"),
            ("gsub", "string.gsub(s, pattern, repl)"),
            ("len", "string.len(s)"),
            ("lower", "string.lower(s)"),
            ("upper", "string.upper(s)"),
            ("trim", "string.trim(s)"),
            ("split", "string.split(s, sep)"),
            ("rep", "string.rep(s, n)"),
        ] {
            items.push(make_completion_item(name, detail, 3));
        }
    } else if prefix.ends_with("table.") {
        for (name, detail) in &[
            ("insert", "table.insert(list, value)"),
            ("remove", "table.remove(list, pos)"),
            ("sort", "table.sort(list, comp)"),
            ("concat", "table.concat(list, sep)"),
            ("freeze", "table.freeze(t)"),
            ("isfrozen", "table.isfrozen(t)"),
            ("clone", "table.clone(t)"),
            ("clear", "table.clear(t)"),
        ] {
            items.push(make_completion_item(name, detail, 3));
        }
    } else if prefix.ends_with("math.") {
        for (name, detail) in &[
            ("floor", "math.floor(x)"),
            ("ceil", "math.ceil(x)"),
            ("sqrt", "math.sqrt(x)"),
            ("abs", "math.abs(x)"),
            ("min", "math.min(...)"),
            ("max", "math.max(...)"),
            ("random", "math.random(m, n)"),
            ("pi", "math.pi"),
            ("huge", "math.huge"),
        ] {
            items.push(make_completion_item(name, detail, 3));
        }
    } else if prefix.ends_with("coroutine.") {
        for (name, detail) in &[
            ("create", "coroutine.create(f)"),
            ("resume", "coroutine.resume(co, ...)"),
            ("yield", "coroutine.yield(...)"),
            ("status", "coroutine.status(co)"),
        ] {
            items.push(make_completion_item(name, detail, 3));
        }
    } else if prefix.ends_with("json.") {
        for (name, detail) in &[
            ("parse", "json.parse(str)"),
            ("stringify", "json.stringify(val)"),
        ] {
            items.push(make_completion_item(name, detail, 3));
        }
    } else if prefix.ends_with("fs.") {
        for (name, detail) in &[
            ("read", "fs.read(path)"),
            ("write", "fs.write(path, content)"),
            ("exists", "fs.exists(path)"),
        ] {
            items.push(make_completion_item(name, detail, 3));
        }
    } else if prefix.ends_with("crypto.") {
        for (name, detail) in &[
            ("hash", "crypto.hash(algo, data)"),
            ("randomBytes", "crypto.randomBytes(n)"),
        ] {
            items.push(make_completion_item(name, detail, 3));
        }
    } else {
        // Top-level globals, keywords, and local document symbols
        for (name, detail) in &[
            ("print", "print(...)"),
            ("assert", "assert(cond, msg)"),
            ("type", "type(val)"),
            ("checktype", "checktype(val, ty)"),
            ("asserttype", "asserttype(val, ty)"),
            ("pcall", "pcall(fn, ...)"),
            ("xpcall", "xpcall(fn, msgh, ...)"),
            ("error", "error(msg)"),
            ("require", "require(mod)"),
            ("os", "module os"),
            ("string", "module string"),
            ("table", "module table"),
            ("math", "module math"),
            ("coroutine", "module coroutine"),
            ("json", "module json"),
            ("fs", "module fs"),
            ("crypto", "module crypto"),
        ] {
            items.push(make_completion_item(name, detail, 3));
        }
        for kw in &[
            "function", "local", "if", "then", "else", "elseif", "for", "while", "do", "repeat",
            "until", "return", "break", "continue", "match", "case", "default", "enum", "end",
        ] {
            items.push(make_completion_item(kw, "keyword", 14)); // Keyword
        }
        for sym in extract_document_symbols(content) {
            items.push(make_completion_item(&sym.name, &sym.detail, 6)); // Variable
        }
    }

    Json::Arr(items)
}

fn make_completion_item(label: &str, detail: &str, kind: usize) -> Json {
    Json::Obj(vec![
        ("label".to_string(), Json::Str(label.to_string())),
        ("detail".to_string(), Json::Str(detail.to_string())),
        ("kind".to_string(), Json::Num(kind as f64)),
    ])
}

struct FoundSymbol {
    name: String,
    detail: String,
    kind: usize,
    line_1_based: u32,
}

fn extract_document_symbols(content: &str) -> Vec<FoundSymbol> {
    let mut symbols = Vec::new();
    if let Ok(parsed) = crate::compiler::parse(content) {
        for stmt in &parsed.stmts {
            match stmt {
                crate::ast::Stmt::Function {
                    name: Some(name),
                    params,
                    id,
                    ..
                } => {
                    let line = parsed.lines.get(id).copied().unwrap_or(1);
                    let param_str = params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    symbols.push(FoundSymbol {
                        name: name.clone(),
                        detail: format!("function {}({})", name, param_str),
                        kind: 12, // Function
                        line_1_based: line,
                    });
                }
                crate::ast::Stmt::Local {
                    name,
                    type_name,
                    id,
                    ..
                } => {
                    let line = parsed.lines.get(id).copied().unwrap_or(1);
                    let ty = type_name.as_deref().unwrap_or("any");
                    symbols.push(FoundSymbol {
                        name: name.clone(),
                        detail: format!("local {}: {}", name, ty),
                        kind: 13, // Variable
                        line_1_based: line,
                    });
                }
                crate::ast::Stmt::LocalMany { names, id, .. } => {
                    let line = parsed.lines.get(id).copied().unwrap_or(1);
                    for name in names {
                        symbols.push(FoundSymbol {
                            name: name.clone(),
                            detail: format!("local {}", name),
                            kind: 13,
                            line_1_based: line,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    symbols
}

fn lookup_docs(word: &str) -> Option<(&'static str, &'static str)> {
    match word {
        "print" => Some((
            "print(...: any): void",
            "Prints formatted values to standard output separated by tabs and followed by a newline.",
        )),
        "assert" => Some((
            "assert(v: any, message?: string): any",
            "Asserts that `v` is truthy (not `nil` or `false`). Raises an error if assertion fails.",
        )),
        "type" => Some((
            "type(v: any): string",
            "Returns the basic type name of `v` as a string (`\"nil\"`, `\"boolean\"`, `\"number\"`, `\"string\"`, `\"table\"`, `\"function\"`, `\"thread\"`, `\"buffer\"`).",
        )),
        "checktype" => Some((
            "checktype(val: any, expected: string): boolean",
            "Returns true if `val` conforms to the specified type expression, false otherwise.",
        )),
        "asserttype" | "assert_type" => Some((
            "asserttype(val: any, expected: string): any",
            "Asserts that `val` conforms to the specified type expression. Raises error `E0210` on mismatch.",
        )),
        "pcall" => Some((
            "pcall(f: function, ...args: any): (boolean, ...any)",
            "Calls function `f` in protected mode. Catches runtime errors and returns `true` plus results, or `false` plus error message.",
        )),
        "xpcall" => Some((
            "xpcall(f: function, msgh: function, ...args: any): (boolean, ...any)",
            "Calls function `f` in protected mode with a custom error handler `msgh`.",
        )),
        "error" => Some((
            "error(message: string, level?: int): void",
            "Terminates the last protected function called and returns `message` as the error object.",
        )),
        "setmetatable" => Some((
            "setmetatable(table: table, metatable: table | nil): table",
            "Sets the metatable for the given table. If `metatable` is `nil`, removes the metatable.",
        )),
        "getmetatable" => Some((
            "getmetatable(table: table): table | nil",
            "Returns the metatable of the given table, or `nil` if it has no metatable.",
        )),
        "rawget" => Some((
            "rawget(table: table, index: any): any",
            "Gets the real value of `table[index]` without invoking `__index` metamethods.",
        )),
        "rawset" => Some((
            "rawset(table: table, index: any, value: any): table",
            "Sets the real value of `table[index]` to `value` without invoking `__newindex` metamethods.",
        )),
        "rawequal" => Some((
            "rawequal(v1: any, v2: any): boolean",
            "Checks whether `v1` is primitive-equal to `v2` without invoking `__eq` metamethods.",
        )),
        "rawlen" => Some((
            "rawlen(v: table | string): int",
            "Returns the length of `v` without invoking `__len` metamethods.",
        )),
        "select" => Some((
            "select(index: int | \"#\", ...args: any): any",
            "If index is a number, returns all arguments after argument number `index`. If `\"#\"`, returns the count of arguments.",
        )),
        "collectgarbage" => Some((
            "collectgarbage(opt?: string, arg?: int): any",
            "Performs a garbage collection cycle or queries GC statistics.",
        )),
        "require" => Some((
            "require(modname: string): any",
            "Loads and returns the given module, caching the result in `package.loaded`.",
        )),
        "os" => Some((
            "module os",
            "Operating system interaction: processes, environment, clock, and filesystem.",
        )),
        "os.run" => Some((
            "os.run(command: string, options?: { stdin?: string, timeout?: int }): { stdout: string, stderr: string, code: int, success: boolean }",
            "Executes an external system command and captures output, status code, and execution success.",
        )),
        "os.exit" => Some((
            "os.exit(code?: int): void",
            "Terminates the host process with an exit status code.",
        )),
        "os.getenv" => Some((
            "os.getenv(varname: string): string | nil",
            "Returns the value of the environment variable, or `nil` if undefined.",
        )),
        "os.time" => Some((
            "os.time(): int",
            "Returns the current time in seconds since the UNIX epoch.",
        )),
        "os.clock" => Some((
            "os.clock(): float",
            "Returns an approximation of the amount in seconds of CPU time used by the program.",
        )),
        "string" => Some((
            "module string",
            "String manipulation, formatting, pattern matching, and encoding.",
        )),
        "string.sub" => Some((
            "string.sub(s: string, i: int, j?: int): string",
            "Returns the substring of `s` that starts at `i` and continues until `j`.",
        )),
        "string.format" => Some((
            "string.format(formatstring: string, ...args: any): string",
            "Returns a formatted version of its variable arguments following standard printf conventions.",
        )),
        "string.len" => Some((
            "string.len(s: string): int",
            "Returns length of string in bytes.",
        )),
        "string.lower" => Some((
            "string.lower(s: string): string",
            "Returns a copy of the string with all uppercase letters converted to lowercase.",
        )),
        "string.upper" => Some((
            "string.upper(s: string): string",
            "Returns a copy of the string with all lowercase letters converted to uppercase.",
        )),
        "string.rep" => Some((
            "string.rep(s: string, n: int, sep?: string): string",
            "Returns a string that is the concatenation of `n` copies of the string `s`.",
        )),
        "string.trim" => Some((
            "string.trim(s: string): string",
            "Trims leading and trailing whitespace characters.",
        )),
        "string.split" => Some((
            "string.split(s: string, sep?: string): table",
            "Splits string by separator into a table of parts.",
        )),
        "string.find" => Some((
            "string.find(s: string, pattern: string, init?: int, plain?: boolean): (int, int) | nil",
            "Searches for the first match of pattern in string.",
        )),
        "string.match" => Some((
            "string.match(s: string, pattern: string, init?: int): any",
            "Looks for the first match of pattern in string and returns captures.",
        )),
        "string.gmatch" => Some((
            "string.gmatch(s: string, pattern: string): function",
            "Returns an iterator function that returns next captures from pattern over string.",
        )),
        "string.gsub" => Some((
            "string.gsub(s: string, pattern: string, repl: any, n?: int): (string, int)",
            "Replaces occurrences of pattern with replacement.",
        )),
        "table" => Some((
            "module table",
            "Table manipulation, sorting, packing, and freezing.",
        )),
        "table.insert" => Some((
            "table.insert(list: table, [pos: int,] value: any): void",
            "Inserts element `value` at position `pos` in `list`.",
        )),
        "table.remove" => Some((
            "table.remove(list: table, pos?: int): any",
            "Removes element at `pos` from `list`, shifting elements down.",
        )),
        "table.sort" => Some((
            "table.sort(list: table, comp?: function): void",
            "Sorts list elements in place using optional comparator.",
        )),
        "table.concat" => Some((
            "table.concat(list: table, sep?: string, i?: int, j?: int): string",
            "Concatenates elements of `list` with separator `sep`.",
        )),
        "table.freeze" => Some((
            "table.freeze(t: table): table",
            "Freezes a table, making it read-only.",
        )),
        "table.isfrozen" => Some((
            "table.isfrozen(t: table): boolean",
            "Returns true if table is frozen.",
        )),
        "table.clone" => Some((
            "table.clone(t: table): table",
            "Creates a shallow copy of table.",
        )),
        "table.clear" => Some((
            "table.clear(t: table): void",
            "Clears all keys and values from table in place.",
        )),
        "math" => Some((
            "module math",
            "Standard mathematical functions, trigonometric operations, and constants.",
        )),
        "math.floor" => Some((
            "math.floor(x: number): int",
            "Returns the largest integer less than or equal to `x`.",
        )),
        "math.ceil" => Some((
            "math.ceil(x: number): int",
            "Returns the smallest integer greater than or equal to `x`.",
        )),
        "math.sqrt" => Some((
            "math.sqrt(x: number): float",
            "Returns the square root of `x`.",
        )),
        "math.abs" => Some((
            "math.abs(x: number): number",
            "Returns the absolute value of `x`.",
        )),
        "math.min" => Some((
            "math.min(...x: number): number",
            "Returns the argument with minimum value.",
        )),
        "math.max" => Some((
            "math.max(...x: number): number",
            "Returns the argument with maximum value.",
        )),
        "math.random" => Some((
            "math.random(m?: int, n?: int): number",
            "Generates pseudo-random numbers.",
        )),
        "math.pi" => Some((
            "math.pi: float",
            "The value of mathematical constant Pi (3.141592653589793).",
        )),
        "math.huge" => Some((
            "math.huge: float",
            "A value larger than any other numerical value (infinity).",
        )),
        "coroutine" => Some((
            "module coroutine",
            "Cooperative multithreading and fiber manipulation.",
        )),
        "coroutine.create" => Some((
            "coroutine.create(f: function): thread",
            "Creates a new coroutine with body function `f`.",
        )),
        "coroutine.resume" => Some((
            "coroutine.resume(co: thread, ...args: any): (boolean, ...any)",
            "Starts or continues the execution of coroutine `co`.",
        )),
        "coroutine.yield" => Some((
            "coroutine.yield(...values: any): ...any",
            "Suspends the execution of the calling coroutine.",
        )),
        "coroutine.status" => Some((
            "coroutine.status(co: thread): string",
            "Returns status: `\"running\"`, `\"suspended\"`, `\"normal\"`, or `\"dead\"`.",
        )),
        "json" => Some(("module json", "Fast JSON encoding and decoding.")),
        "json.parse" => Some((
            "json.parse(text: string): any",
            "Parses a JSON string into Neyuki tables and primitives.",
        )),
        "json.stringify" => Some((
            "json.stringify(value: any, indent?: int): string",
            "Serializes a Neyuki value into a JSON string.",
        )),
        "fs" => Some((
            "module fs",
            "Filesystem operations: reading, writing, directories, file metadata.",
        )),
        "fs.read" => Some((
            "fs.read(path: string): string",
            "Reads the entire contents of a file as a string.",
        )),
        "fs.write" => Some((
            "fs.write(path: string, content: string | buffer): void",
            "Writes content to file, creating or overwriting it.",
        )),
        "fs.exists" => Some((
            "fs.exists(path: string): boolean",
            "Returns true if path exists.",
        )),
        "crypto" => Some((
            "module crypto",
            "Cryptographic hashing, HMAC, encryption, key generation, and OS CSPRNG.",
        )),
        "crypto.hash" => Some((
            "crypto.hash(algorithm: string, data: string | buffer): string",
            "Computes cryptographic digest (`sha256`, `sha512`, `blake2b`, etc.).",
        )),
        "crypto.randomBytes" => Some((
            "crypto.randomBytes(count: int): buffer",
            "Generates cryptographically secure random bytes from OS CSPRNG.",
        )),
        "function" => Some(("keyword function", "Defines a named or anonymous function.")),
        "local" => Some((
            "keyword local",
            "Declares a lexical local variable or constant.",
        )),
        "if" => Some((
            "keyword if",
            "Conditional control flow: `if <cond> then <stmts> end`.",
        )),
        "then" => Some((
            "keyword then",
            "Delimits the condition in an `if` or `elseif` statement.",
        )),
        "else" => Some((
            "keyword else",
            "Specifies alternative branch in an `if` statement.",
        )),
        "elseif" => Some((
            "keyword elseif",
            "Chained conditional branch in an `if` statement.",
        )),
        "for" => Some((
            "keyword for",
            "Loop construct: numeric `for i = 1, 10 do` or generic `for k, v in iter do`.",
        )),
        "while" => Some((
            "keyword while",
            "While loop: executes body while condition is truthy.",
        )),
        "repeat" => Some((
            "keyword repeat",
            "Repeat-until loop: executes body until condition becomes truthy.",
        )),
        "until" => Some((
            "keyword until",
            "Terminates a `repeat` loop when condition is truthy.",
        )),
        "return" => Some((
            "keyword return",
            "Returns values from the enclosing function or chunk.",
        )),
        "break" => Some((
            "keyword break",
            "Terminates execution of the innermost loop.",
        )),
        "continue" => Some((
            "keyword continue",
            "Skips to the next iteration of the innermost loop.",
        )),
        "match" => Some((
            "keyword match",
            "Pattern-matching statement: `match <expr> case <pattern>: ... end`.",
        )),
        "case" => Some((
            "keyword case",
            "Pattern-matching clause in a `match` statement.",
        )),
        "default" => Some(("keyword default", "Fallback clause in a `match` statement.")),
        "enum" => Some((
            "keyword enum",
            "Defines an enumeration table: `enum <Name> ... end`.",
        )),
        "do" => Some((
            "keyword do",
            "Introduces an explicit lexical block scope or loop body.",
        )),
        "end" => Some((
            "keyword end",
            "Closes a function, block, loop, condition, match, or enum.",
        )),
        _ => None,
    }
}

/// Runs the LSP server listening on stdio.
pub fn run_lsp_server() -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = io::stdout().lock();
    let mut server = LspServer::new();

    loop {
        match dap_json::read_message(&mut reader) {
            Ok(Some(msg_bytes)) => {
                match server.handle_message(&msg_bytes) {
                    Ok(Some(resp_bytes)) => {
                        dap_json::write_message(&mut stdout, &resp_bytes)?;
                        stdout.flush()?;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        eprintln!("LSP error handling message: {}", err);
                    }
                }
                if server.is_exit {
                    let code = server.exit_code.unwrap_or(0);
                    std::process::exit(code);
                }
            }
            Ok(None) => break,
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    break;
                }
                eprintln!("LSP IO error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_initialize_capabilities() {
        let mut server = LspServer::new();
        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = server.handle_message(init_req.as_bytes()).unwrap().unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("\"capabilities\""));
        assert!(resp_str.contains("\"documentFormattingProvider\":true"));
        assert!(resp_str.contains("\"hoverProvider\":true"));
        assert!(resp_str.contains("\"completionProvider\""));
        assert!(resp_str.contains("\"definitionProvider\":true"));
        assert!(resp_str.contains("\"documentSymbolProvider\":true"));
    }

    #[test]
    fn test_lsp_shutdown_and_exit_sequence() {
        let mut server = LspServer::new();
        let shutdown_req = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#;
        let resp = server
            .handle_message(shutdown_req.as_bytes())
            .unwrap()
            .unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("\"result\":null"));
        assert!(server.is_shutdown);

        // Requests after shutdown should return error
        let invalid_req =
            r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/formatting","params":{}}"#;
        let err_resp = server
            .handle_message(invalid_req.as_bytes())
            .unwrap()
            .unwrap();
        let err_str = String::from_utf8(err_resp).unwrap();
        assert!(err_str.contains("\"code\":-32600"));

        // Exit notification after shutdown
        let exit_notif = r#"{"jsonrpc":"2.0","method":"exit"}"#;
        let exit_resp = server.handle_message(exit_notif.as_bytes()).unwrap();
        assert!(exit_resp.is_none());
        assert!(server.is_exit);
        assert_eq!(server.exit_code, Some(0));
    }

    #[test]
    fn test_lsp_exit_without_shutdown_sets_exit_code_1() {
        let mut server = LspServer::new();
        let exit_notif = r#"{"jsonrpc":"2.0","method":"exit"}"#;
        let exit_resp = server.handle_message(exit_notif.as_bytes()).unwrap();
        assert!(exit_resp.is_none());
        assert!(server.is_exit);
        assert_eq!(server.exit_code, Some(1));
    }

    #[test]
    fn test_lsp_publish_diagnostics_on_open() {
        let mut server = LspServer::new();
        let open_req = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.nyk","text":"local unused = 10\n"}}}"#;
        let resp = server.handle_message(open_req.as_bytes()).unwrap().unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("textDocument/publishDiagnostics"));
        assert!(resp_str.contains("W0001")); // unused variable warning
        assert!(server.documents.contains_key("file:///test.nyk"));
    }

    #[test]
    fn test_lsp_did_change() {
        let mut server = LspServer::new();
        let open_req = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.nyk","text":"local a = 1\n"}}}"#;
        let _ = server.handle_message(open_req.as_bytes()).unwrap();

        let change_req = r#"{"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":"file:///test.nyk"},"contentChanges":[{"text":"local b = 2\n"}]}}"#;
        let resp = server
            .handle_message(change_req.as_bytes())
            .unwrap()
            .unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("textDocument/publishDiagnostics"));
        assert_eq!(
            server.documents.get("file:///test.nyk"),
            Some(&"local b = 2\n".to_string())
        );
    }

    #[test]
    fn test_lsp_did_close_cleans_document_and_diagnostics() {
        let mut server = LspServer::new();
        let open_req = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.nyk","text":"local a = 1\n"}}}"#;
        let _ = server.handle_message(open_req.as_bytes()).unwrap();
        assert!(server.documents.contains_key("file:///test.nyk"));

        let close_req = r#"{"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":"file:///test.nyk"}}}"#;
        let resp = server
            .handle_message(close_req.as_bytes())
            .unwrap()
            .unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("\"diagnostics\":[]"));
        assert!(!server.documents.contains_key("file:///test.nyk"));
    }

    #[test]
    fn test_lsp_formatting() {
        let mut server = LspServer::new();
        let open_req = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.nyk","text":"function foo()\nreturn 1\nend\n"}}}"#;
        let _ = server.handle_message(open_req.as_bytes()).unwrap();

        let format_req = r#"{"jsonrpc":"2.0","id":10,"method":"textDocument/formatting","params":{"textDocument":{"uri":"file:///test.nyk"}}}"#;
        let resp = server
            .handle_message(format_req.as_bytes())
            .unwrap()
            .unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("\"newText\":\"function foo()\\n    return 1\\nend\\n\""));
        assert!(!resp_str.contains("100000"));
    }

    #[test]
    fn test_lsp_hover_standard_library_and_locals() {
        let mut server = LspServer::new();
        let open_req = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.nyk","text":"local x = 1\nprint(x)\n"}}}"#;
        let _ = server.handle_message(open_req.as_bytes()).unwrap();

        // Hover on print (line 1, col 2)
        let hover_req = r#"{"jsonrpc":"2.0","id":20,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///test.nyk"},"position":{"line":1,"character":2}}}"#;
        let resp = server
            .handle_message(hover_req.as_bytes())
            .unwrap()
            .unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("print(...: any): void"));
        assert!(resp_str.contains("Prints formatted values"));
    }

    #[test]
    fn test_lsp_completion_globals_and_modules() {
        let mut server = LspServer::new();
        let open_req = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.nyk","text":"os.\n"}}}"#;
        let _ = server.handle_message(open_req.as_bytes()).unwrap();

        let comp_req = r#"{"jsonrpc":"2.0","id":30,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///test.nyk"},"position":{"line":0,"character":3}}}"#;
        let resp = server.handle_message(comp_req.as_bytes()).unwrap().unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("\"label\":\"run\""));
        assert!(resp_str.contains("\"label\":\"exit\""));
    }

    #[test]
    fn test_lsp_document_symbols() {
        let mut server = LspServer::new();
        let open_req = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.nyk","text":"local a = 1\nfunction compute(x)\nreturn x * 2\nend\n"}}}"#;
        let _ = server.handle_message(open_req.as_bytes()).unwrap();

        let sym_req = r#"{"jsonrpc":"2.0","id":40,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":"file:///test.nyk"}}}"#;
        let resp = server.handle_message(sym_req.as_bytes()).unwrap().unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("\"name\":\"compute\""));
        assert!(resp_str.contains("\"name\":\"a\""));
    }

    #[test]
    fn test_lsp_unknown_request_returns_error() {
        let mut server = LspServer::new();
        let req = r#"{"jsonrpc":"2.0","id":42,"method":"custom/unknownMethod","params":{}}"#;
        let resp = server.handle_message(req.as_bytes()).unwrap().unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("\"code\":-32601"));
        assert!(resp_str.contains("Method 'custom/unknownMethod' not found"));
    }

    #[test]
    fn test_lsp_malformed_json_returns_error() {
        let mut server = LspServer::new();
        let bad_bytes = b"not a json";
        let resp = server.handle_message(bad_bytes).unwrap().unwrap();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.contains("\"code\":-32700"));
        assert!(resp_str.contains("Parse error"));
    }

    #[test]
    fn test_lsp_diagnostics_utf16_positions() {
        let s = "local 🦀_unused = 1\n";
        let col_chars = 7;
        let col_utf16 = char_col_to_utf16_col(s, col_chars);
        assert_eq!(col_utf16, 8);
    }
}
