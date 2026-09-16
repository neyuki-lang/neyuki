// Linter rule checking for variable shadowing across scopes.

use crate::ast::stmt::Stmt;
use crate::diagnostics::diagnostic::Diagnostic;
use crate::diagnostics::error_code::ErrorCode;
use crate::sema::analyze;

pub fn check(statements: &[Stmt], source: &str) -> Vec<Diagnostic> {
    let diags = analyze(statements, source);
    diags
        .into_iter()
        .filter(|d| d.code == Some(ErrorCode::W0003))
        .collect()
}
