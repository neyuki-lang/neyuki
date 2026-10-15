//! Language Server Protocol (LSP) server for `neyuki lsp`, running over stdio.

use std::collections::HashMap;
use std::io::{self, BufReader, Write};

use crate::dap_json::{self, Json};
use crate::diagnostics::severity::Severity;

pub struct LspServer {
    documents: HashMap<String, String>,
    is_shutdown: bool,
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
        }
    }

    pub fn handle_message(&mut self, msg_bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let json = dap_json::parse_json(msg_bytes).map_err(|e| e.to_string())?;

        let method = json.get("method").and_then(|m| m.as_str());
        let id = json.get("id");

        if let Some(method_name) = method {
            match method_name {
                "initialize" => {
                    let req_id = id.cloned().unwrap_or(Json::Num(1.0));
                    let resp = Json::Obj(vec![
                        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
                        ("id".to_string(), req_id),
                        (
                            "result".to_string(),
                            Json::Obj(vec![(
                                "capabilities".to_string(),
                                Json::Obj(vec![
                                    ("textDocumentSync".to_string(), Json::Num(1.0)),
                                    ("hoverProvider".to_string(), Json::Bool(true)),
                                    ("documentFormattingProvider".to_string(), Json::Bool(true)),
                                ]),
                            )]),
                        ),
                    ]);
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
                    let resp = Json::Obj(vec![
                        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
                        ("id".to_string(), req_id),
                        ("result".to_string(), Json::Null),
                    ]);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
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
                                                ("line".to_string(), Json::Num(100_000.0)),
                                                ("character".to_string(), Json::Num(0.0)),
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

                    let resp = Json::Obj(vec![
                        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
                        ("id".to_string(), req_id),
                        ("result".to_string(), edits),
                    ]);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                "textDocument/hover" => {
                    let req_id = id.cloned().unwrap_or(Json::Null);
                    let hover_text = "Neyuki Script Symbol";
                    let resp = Json::Obj(vec![
                        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
                        ("id".to_string(), req_id),
                        (
                            "result".to_string(),
                            Json::Obj(vec![(
                                "contents".to_string(),
                                Json::Obj(vec![
                                    ("kind".to_string(), Json::Str("markdown".to_string())),
                                    ("value".to_string(), Json::Str(hover_text.to_string())),
                                ]),
                            )]),
                        ),
                    ]);
                    let bytes = dap_json::render_json(&resp).into_bytes();
                    return Ok(Some(bytes));
                }
                _ => {
                    // Unknown method, return empty or null if request
                    if let Some(req_id) = id {
                        let resp = Json::Obj(vec![
                            ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
                            ("id".to_string(), req_id.clone()),
                            ("result".to_string(), Json::Null),
                        ]);
                        let bytes = dap_json::render_json(&resp).into_bytes();
                        return Ok(Some(bytes));
                    }
                }
            }
        }

        Ok(None)
    }
}

fn create_publish_diagnostics(uri: &str, content: &str) -> Json {
    let diags = crate::lint::lint_source(content);
    let mut diag_items = Vec::new();

    for d in diags {
        let (start_line, start_col, end_line, end_col) = d
            .primary_label
            .as_ref()
            .map(|l| {
                (
                    l.span.start.line.saturating_sub(1) as f64,
                    l.span.start.column.saturating_sub(1) as f64,
                    l.span.end.line.saturating_sub(1) as f64,
                    l.span.end.column.saturating_sub(1) as f64,
                )
            })
            .unwrap_or((0.0, 0.0, 0.0, 0.0));

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
                            ("character".to_string(), Json::Num(start_col)),
                        ]),
                    ),
                    (
                        "end".to_string(),
                        Json::Obj(vec![
                            ("line".to_string(), Json::Num(end_line.max(start_line))),
                            (
                                "character".to_string(),
                                Json::Num(end_col.max(start_col + 1.0)),
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

    Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        (
            "method".to_string(),
            Json::Str("textDocument/publishDiagnostics".to_string()),
        ),
        (
            "params".to_string(),
            Json::Obj(vec![
                ("uri".to_string(), Json::Str(uri.to_string())),
                ("diagnostics".to_string(), Json::Arr(diag_items)),
            ]),
        ),
    ])
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
                if let Ok(Some(resp_bytes)) = server.handle_message(&msg_bytes) {
                    dap_json::write_message(&mut stdout, &resp_bytes)?;
                    stdout.flush()?;
                }
                if server.is_shutdown {
                    break;
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
    fn test_lsp_initialize_and_shutdown() {
        let mut server = LspServer::new();
        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = server.handle_message(init_req.as_bytes()).unwrap();
        assert!(resp.is_some());
        let resp_str = String::from_utf8(resp.unwrap()).unwrap();
        assert!(resp_str.contains("\"capabilities\""));

        let shutdown_req = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#;
        let resp = server.handle_message(shutdown_req.as_bytes()).unwrap();
        assert!(resp.is_some());
        assert!(server.is_shutdown);
    }

    #[test]
    fn test_lsp_publish_diagnostics_on_open() {
        let mut server = LspServer::new();
        let open_req = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.nyk","text":"local unused = 10\n"}}}"#;
        let resp = server.handle_message(open_req.as_bytes()).unwrap();
        assert!(resp.is_some());
        let resp_str = String::from_utf8(resp.unwrap()).unwrap();
        assert!(resp_str.contains("textDocument/publishDiagnostics"));
        assert!(resp_str.contains("W0001")); // unused variable warning
    }
}
