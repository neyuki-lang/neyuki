//! `neyuki lint`: reports syntax errors and everything the checker finds
//! (undefined names, const writes, type mismatches, unused and unreachable
//! code), as a readable report or as JSON for editors.

use std::fs;
use std::io::Read;

use crate::dap_json::{Json, render_json};
use crate::diagnostics::diagnostic::Diagnostic;
use crate::diagnostics::emitter::render_report;
use crate::diagnostics::error_code::ErrorCode;
use crate::diagnostics::severity::Severity;
use crate::diagnostics::span::Span;

/// Checks source and returns its diagnostics ordered by position. A syntax
/// error stops the check and is the only diagnostic.
pub fn lint_source(source: &str) -> Vec<Diagnostic> {
    let parsed = match crate::compiler::parse(source) {
        Ok(parsed) => parsed,
        Err(err) => return vec![syntax_error(&err)],
    };
    let mut diagnostics = crate::sema::analyze_with_spans(&parsed.stmts, source, &parsed.spans);
    diagnostics.sort_by_key(|d| {
        d.primary_label
            .as_ref()
            .map(|l| (l.span.start.line, l.span.start.column))
            .unwrap_or((0, 0))
    });
    diagnostics.dedup_by(|a, b| a.code == b.code && a.message == b.message);
    diagnostics
}

/// Turns a parser error such as `[line 4] expected symbol ")"` into a
/// diagnostic on that line.
fn syntax_error(err: &str) -> Diagnostic {
    let (line, message) = err
        .strip_prefix("[line ")
        .and_then(|rest| rest.split_once("] "))
        .and_then(|(line, message)| Some((line.parse::<u32>().ok()?, message)))
        .unwrap_or((1, err));
    Diagnostic::error(message)
        .with_code(ErrorCode::E0050)
        .with_label(Span::line(line.max(1)), "syntax error")
}

/// Renders diagnostics as a JSON array of objects with `severity`, `code`,
/// `message`, `line`, `column`, `endLine`, `endColumn` (1-based, end
/// exclusive) and `help` when there is one.
pub fn diagnostics_json(diagnostics: &[Diagnostic]) -> String {
    let items = diagnostics
        .iter()
        .map(|d| {
            let span = d.primary_label.as_ref().map(|l| l.span).unwrap_or_default();
            let mut fields = vec![
                (
                    "severity".to_string(),
                    Json::Str(d.severity.as_str().to_string()),
                ),
                (
                    "code".to_string(),
                    d.code
                        .map(|c| Json::Str(c.as_str().to_string()))
                        .unwrap_or(Json::Null),
                ),
                ("message".to_string(), Json::Str(d.message.clone())),
                ("line".to_string(), Json::Num(span.start.line.max(1) as f64)),
                (
                    "column".to_string(),
                    Json::Num(span.start.column.max(1) as f64),
                ),
                (
                    "endLine".to_string(),
                    Json::Num(span.end.line.max(span.start.line).max(1) as f64),
                ),
                (
                    "endColumn".to_string(),
                    Json::Num(span.end.column.max(span.start.column).max(1) as f64),
                ),
            ];
            if let Some(help) = &d.help {
                fields.push(("help".to_string(), Json::Str(help.clone())));
            }
            Json::Obj(fields)
        })
        .collect();
    render_json(&Json::Arr(items))
}

/// `neyuki lint [--json] [--stdin] <path>`. With `--stdin` the source is read
/// from standard input and `path` only names it. Fails when there are errors.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    let json = args.iter().any(|a| a == "--json");
    let stdin = args.iter().any(|a| a == "--stdin");
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .ok_or("Usage: neyuki lint [--json] [--stdin] <path>")?;
    let source = if stdin {
        let mut source = String::new();
        std::io::stdin()
            .read_to_string(&mut source)
            .map_err(|err| format!("failed to read stdin: {}", err))?;
        source
    } else {
        fs::read_to_string(path).map_err(|err| format!("failed to read '{}': {}", path, err))?
    };

    let diagnostics = lint_source(&source);
    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);
    if json {
        println!("{}", diagnostics_json(&diagnostics));
    } else if diagnostics.is_empty() {
        println!("No issues found in '{}'.", path);
    } else {
        eprintln!("{}", render_report(path, &source, &diagnostics));
    }
    if has_errors && !json {
        Err(format!("lint found errors in '{}'", path))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_source_has_no_diagnostics() {
        assert!(lint_source("local x = 10\nreturn x + 1").is_empty());
    }

    #[test]
    fn reports_unused_locals_where_they_are() {
        let diags = lint_source("local a = 1\nlocal unused = 42\nreturn a");
        let unused = diags
            .iter()
            .find(|d| d.code == Some(ErrorCode::W0001))
            .expect("unused warning");
        let span = unused.primary_label.as_ref().unwrap().span;
        assert_eq!((span.start.line, span.start.column), (2, 7));
    }

    #[test]
    fn reports_undefined_names_and_type_errors_on_their_lines() {
        let diags = lint_source(
            "local x = 1\nprint(y + x)\nlocal function f(): number\n    return \"s\"\nend\nf()",
        );
        let lines: Vec<(Option<ErrorCode>, u32)> = diags
            .iter()
            .map(|d| (d.code, d.primary_label.as_ref().unwrap().span.start.line))
            .collect();
        assert!(lines.contains(&(Some(ErrorCode::E0001), 2)), "{:?}", lines);
        assert!(lines.contains(&(Some(ErrorCode::E0003), 4)), "{:?}", lines);
    }

    #[test]
    fn syntax_errors_keep_their_line() {
        let diags = lint_source("local x = 1\nprint(x\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, Some(ErrorCode::E0050));
        assert!(diags[0].primary_label.as_ref().unwrap().span.start.line >= 2);
    }

    #[test]
    fn json_output_is_an_array_of_positions() {
        let json = diagnostics_json(&lint_source("local unused = 1"));
        assert!(
            json.starts_with('[') && json.contains("\"code\":\"W0001\""),
            "{}",
            json
        );
        assert!(json.contains("\"line\":1"), "{}", json);
    }
}
