// Bytecode compiler module.

pub mod builtins_folding;
pub mod codegen;
pub mod constant_fold;
pub mod cost_model;
pub mod ir;
pub mod table_shape;

use std::fs;
use crate::bytecode::proto::Proto;
use crate::bytecode::serialize::serialize;
use crate::parser::{Param, Parser, Stmt};

#[allow(unused_imports)]
pub use codegen::{Compiler, CompileError};
pub use constant_fold::fold_program;

// Parse Neyuki source code into AST statements
pub fn compile_source(source: &str) -> Result<Vec<Stmt>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut parser = Parser::new(source);
        parser.parse_program()
    }))
    .map_err(|payload| {
        if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "syntax error".to_string()
        }
    })?
}

// Read and parse source file into AST statements
pub fn compile_file(path: &str) -> Result<Vec<Stmt>, String> {
    let source =
        fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;
    compile_source(&source)
}

// Compile AST statements into a register-based Proto with constant folding optimization,
// returning detailed errors on compilation failure.
pub fn try_compile_to_proto(statements: &[Stmt]) -> Result<Proto, String> {
    let optimized_stmts = fold_program(statements.to_vec());
    let mut compiler = Compiler::new(Some("main".to_string()), 0, false);
    compiler.compile_program(&optimized_stmts);
    compiler.finish().map_err(|errors| {
        errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

// Compile AST function (with parameters and body statements) into a register-based Proto
pub fn try_compile_function_to_proto(
    name: Option<String>,
    params: &[Param],
    statements: &[Stmt],
) -> Result<Proto, String> {
    let is_vararg = params.iter().any(|p| p.variadic);
    let num_params = params.iter().filter(|p| !p.variadic).count() as u8;
    let optimized_stmts = fold_program(statements.to_vec());
    let mut compiler = Compiler::new(name, num_params, is_vararg);
    for param in params {
        if !param.variadic {
            let reg = compiler.alloc_reg();
            compiler.current_mut().add_local(param.name.clone(), reg);
        }
    }
    compiler.compile_program(&optimized_stmts);
    compiler.finish().map_err(|errors| {
        errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

// Compile AST statements into a register-based Proto with fallback on error
pub fn compile_to_proto(statements: &[Stmt]) -> Proto {
    match try_compile_to_proto(statements) {
        Ok(proto) => proto,
        Err(err) => {
            eprintln!("{}", err);
            // Return a minimal proto that immediately returns nil
            let mut p = Proto::new(Some("error".to_string()), 0, false);
            p.emit(crate::bytecode::instruction::Instruction::LoadNil { dst: 0 }, 0);
            p.emit(crate::bytecode::instruction::Instruction::Return { base: 0, count: 1 }, 0);
            p.max_registers = 1;
            p
        }
    }
}

// Compile source directly into binary bytecode with magic bytes 'neyuki!'
pub fn compile_source_to_bytecode(source: &str) -> Result<Vec<u8>, String> {
    let stmts = compile_source(source)?;
    let proto = try_compile_to_proto(&stmts)?;
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

    #[test]
    fn test_compile_error_on_invalid_interpolation() {
        // Unfinished interpolation must fail compilation
        let code = "local s = `hello {name`";
        let res = compile_source(code);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_lowercase().contains("unfinished"));

        // Multi-statement in interpolation must fail compilation
        let code2 = "local s = `hello {a; b}`";
        let res2 = compile_source(code2);
        assert!(res2.is_err());
        assert!(res2.unwrap_err().contains("exactly one expression"));
    }
}
