// Linter rule checking for reassignment or mutation of const variables.

use crate::ast::stmt::Stmt;
use crate::diagnostics::diagnostic::Diagnostic;
use crate::diagnostics::error_code::ErrorCode;
use crate::sema::analyze;

pub fn check(statements: &[Stmt], source: &str) -> Vec<Diagnostic> {
    let diags = analyze(statements, source);
    diags
        .into_iter()
        .filter(|d| d.code == Some(ErrorCode::E0002))
        .collect()
}
