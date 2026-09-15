// Bytecode compiler module.

pub mod codegen;
pub mod constant_fold;

use std::fs;
use crate::bytecode::proto::Proto;
use crate::bytecode::serialize::serialize;
use crate::parser::{Parser, Stmt};

pub use codegen::Compiler;
pub use constant_fold::fold_program;

// Parse Neyuki source code into AST statements
pub fn compile_source(source: &str) -> Result<Vec<Stmt>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut parser = Parser::new(source);
        parser.parse_program()
    }))
    .map_err(|_| "syntax error".to_string())
}

// Read and parse source file into AST statements
pub fn compile_file(path: &str) -> Result<Vec<Stmt>, String> {
    let source =
        fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;
    compile_source(&source)
}

// Compile AST statements into a register-based Proto with constant folding optimization
pub fn compile_to_proto(statements: &[Stmt]) -> Proto {
    let optimized_stmts = fold_program(statements.to_vec());
    let mut compiler = Compiler::new(Some("main".to_string()), 0, false);
    compiler.compile_program(&optimized_stmts);
    compiler.finish()
}

// Compile source directly into binary bytecode with magic bytes 'neyuki!'
pub fn compile_source_to_bytecode(source: &str) -> Result<Vec<u8>, String> {
    let stmts = compile_source(source)?;
    let proto = compile_to_proto(&stmts);
    Ok(serialize(&proto))
}

// Compile file to bytecode binary
pub fn compile_file_to_bytecode(path: &str) -> Result<Vec<u8>, String> {
    let source =
        fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;
    compile_source_to_bytecode(&source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::MAGIC;

    #[test]
    fn test_compile_source_to_bytecode_magic() {
        let code = "local a = 10\nlocal b = 20\nreturn a + b";
        let bc = compile_source_to_bytecode(code).expect("compilation failed");
        assert!(bc.starts_with(MAGIC));
    }

    #[test]
    fn test_compile_registers_and_proto() {
        let code = "local x = 5\nreturn x";
        let stmts = compile_source(code).expect("syntax error");
        let proto = compile_to_proto(&stmts);
        assert!(proto.max_registers >= 1);
        assert!(!proto.instructions.is_empty());
    }
}
