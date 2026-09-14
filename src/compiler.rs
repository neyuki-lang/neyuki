use std::fs;

use crate::parser::{Parser, Stmt};

pub fn compile_source(source: &str) -> Result<Vec<Stmt>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut parser = Parser::new(source);
        parser.parse_program()
    }))
        .map_err(|_| "syntax error".to_string())
}

pub fn compile_file(path: &str) -> Result<Vec<Stmt>, String> {
    let source = fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;
    compile_source(&source)
}