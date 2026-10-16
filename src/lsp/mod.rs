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
                    let resp = json_response(req_id, Json::Null);
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
        // Ensure fake hoverProvider is not advertised
        assert!(!resp_str.contains("\"hoverProvider\""));
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
        // Ensure range does not use 100_000 hack
        assert!(!resp_str.contains("100000"));
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
        // 🦀 takes 2 UTF-16 code units
        let col_chars = 7; // after 🦀
        let col_utf16 = char_col_to_utf16_col(s, col_chars);
        assert_eq!(col_utf16, 8); // 'local ' (6) + '🦀' (2) = 8
    }
}
