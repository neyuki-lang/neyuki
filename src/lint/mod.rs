// Comprehensive linting framework for Neyuki source files.

pub mod rules;
pub use rules::run_all_rules;

use std::fs;

use crate::compiler::compile_source;
use crate::diagnostics::diagnostic::Diagnostic;
use crate::diagnostics::emitter::render_report;
use crate::diagnostics::error_code::ErrorCode;
use crate::diagnostics::severity::Severity;
use crate::diagnostics::span::Span;

#[allow(clippy::result_large_err)]
pub fn lint_source(source: &str) -> Result<Vec<Diagnostic>, Diagnostic> {
    let stmts = compile_source(source).map_err(|err| {
        Diagnostic::error(err)
            .with_code(ErrorCode::E0050)
            .with_label(Span::line(1), "syntax error occurred here")
    })?;

    let mut diagnostics = run_all_rules(&stmts, source);
    diagnostics.sort_by_key(|d| {
        d.primary_label
            .as_ref()
            .map(|l| (l.span.start.line, l.span.start.column))
            .unwrap_or((0, 0))
    });
    diagnostics.dedup_by(|a, b| a.code == b.code && a.message == b.message);
    Ok(diagnostics)
}

pub fn lint_file(path: &str) -> Result<Vec<Diagnostic>, String> {
    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read '{}': {}", path, err))?;

    match lint_source(&source) {
        Ok(diags) => Ok(diags),
        Err(diag) => Ok(vec![diag]),
    }
}

// Backward-compatible CLI entry point for `neyuki lint <path>`
pub fn lint(path: &str) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read '{}': {}", path, err))?;

    let diagnostics = lint_file(path)?;

    if diagnostics.is_empty() {
        println!("No issues found in '{}'.", path);
        return Ok(());
    }

    let report = render_report(path, &source, &diagnostics);
    eprintln!("{}", report);

    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);
    if has_errors {
        Err(format!("lint check failed with errors in '{}'", path))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lint_source_clean() {
        let code = "local x = 10\nreturn x + 1";
        let diags = lint_source(code).expect("syntax error");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_lint_source_detects_unused() {
        let code = "local unused = 42\nreturn 100";
        let diags = lint_source(code).expect("syntax error");
        assert!(diags.iter().any(|d| d.code == Some(ErrorCode::W0001)));
    }
}
