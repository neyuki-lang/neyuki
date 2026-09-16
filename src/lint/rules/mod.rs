// Submodules for individual lint rules.

pub mod const_mutation;
pub mod shadowing;
pub mod type_mismatch;
pub mod unreachable_code;
pub mod unused_parameter;
pub mod unused_variable;

use crate::ast::stmt::Stmt;
use crate::diagnostics::diagnostic::Diagnostic;

pub fn run_all_rules(statements: &[Stmt], source: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    diags.extend(unused_variable::check(statements, source));
    diags.extend(unused_parameter::check(statements, source));
    diags.extend(shadowing::check(statements, source));
    diags.extend(unreachable_code::check(statements, source));
    diags.extend(const_mutation::check(statements, source));
    diags.extend(type_mismatch::check(statements, source));
    diags
}
