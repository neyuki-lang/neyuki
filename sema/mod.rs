// Semantic analysis (sema) module for Neyuki.
// Provides scope resolution, const-checking, type-checking, arity checking, and dead-code detection.

pub mod analyzer;
pub mod scope;
pub mod symbol;
pub mod types;

pub use analyzer::SemanticAnalyzer;
pub use scope::ScopeManager;
pub use symbol::{Symbol, SymbolKind};
pub use types::NeyukiType;

use crate::ast::stmt::Stmt;
use crate::diagnostics::diagnostic::Diagnostic;

pub fn analyze(statements: &[Stmt], source: &str) -> Vec<Diagnostic> {
    let mut analyzer = SemanticAnalyzer::new(source);
    analyzer.analyze_program(statements);
    analyzer.diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile_source;
    use crate::diagnostics::error_code::ErrorCode;

    #[test]
    fn test_sema_undeclared_variable() {
        let code = "return unknown_var + 1";
        let stmts = compile_source(code).expect("syntax error");
        let diags = analyze(&stmts, code);
        assert!(diags.iter().any(|d| d.code == Some(ErrorCode::E0001)));
    }

    #[test]
    fn test_sema_reassign_const() {
        let code = "local const x = 10\nx = 20";
        let stmts = compile_source(code).expect("syntax error");
        let diags = analyze(&stmts, code);
        assert!(diags.iter().any(|d| d.code == Some(ErrorCode::E0002)));
    }

    #[test]
    fn test_sema_type_mismatch() {
        let code = "local x: number = \"hello\"\nreturn x";
        let stmts = compile_source(code).expect("syntax error");
        let diags = analyze(&stmts, code);
        assert!(diags.iter().any(|d| d.code == Some(ErrorCode::E0003)));
    }

    #[test]
    fn test_sema_function_arity() {
        let code = "local function add(a, b)\n  return a + b\nend\nadd(1)";
        let stmts = compile_source(code).expect("syntax error");
        let diags = analyze(&stmts, code);
        assert!(diags.iter().any(|d| d.code == Some(ErrorCode::E0004)));
    }

    #[test]
    fn test_sema_unused_variable() {
        let code = "local unused_val = 42\nreturn 0";
        let stmts = compile_source(code).expect("syntax error");
        let diags = analyze(&stmts, code);
        assert!(diags.iter().any(|d| d.code == Some(ErrorCode::W0001)));
    }

    #[test]
    fn test_sema_shadowing() {
        let code = "local x = 10\nif true then\n  local x = 20\n  print(x)\nend\nreturn x";
        let stmts = compile_source(code).expect("syntax error");
        let diags = analyze(&stmts, code);
        assert!(diags.iter().any(|d| d.code == Some(ErrorCode::W0003)));
    }

    #[test]
    fn test_sema_unreachable_code() {
        let code = "return 1\nlocal a = 2\nprint(a)";
        let stmts = compile_source(code).expect("syntax error");
        let diags = analyze(&stmts, code);
        assert!(diags.iter().any(|d| d.code == Some(ErrorCode::W0004)));
    }
}
