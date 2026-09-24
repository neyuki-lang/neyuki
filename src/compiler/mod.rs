// Bytecode compiler module.

pub mod builtins_folding;
pub mod codegen;
pub mod constant_fold;
pub mod cost_model;
pub mod ir;
pub mod table_shape;
pub mod value_tracking;

use crate::bytecode::proto::Proto;
use crate::bytecode::serialize::serialize;
use crate::parser::{Param, Parser, Stmt};
use std::fs;

pub use codegen::{CompileError, Compiler};
pub use constant_fold::fold_program;
pub use value_tracking::{TrackedValue, ValueTracker, value_tracking_cfg};

// Parse Neyuki source code into AST statements
pub fn compile_source(source: &str) -> Result<Vec<Stmt>, String> {
    let mut stmts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut parser = Parser::new(source);
        let (stmts, _pool) = parser.parse_program()?;
        Ok::<Vec<Stmt>, String>(stmts)
    }))
    .map_err(|payload| {
        if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "syntax error".to_string()
        }
    })??;

    crate::ast::visitor::check_ast_sanity(&stmts)?;
    crate::ast::visitor_mut::normalize_ast(&mut stmts);
    Ok(stmts)
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
    let diags = crate::sema::analyze(statements, "");
    if let Some(err) = diags
        .iter()
        .find(|d| d.severity == crate::diagnostics::severity::Severity::Error)
    {
        return Err(format!("semantic error: {}", err.message));
    }
    codegen_proto(statements)
}

// Compile the statements of a bundled `lib/*.nyk` module. The standard library
// is vetted at build time, and the tree-walking engine runs it unanalyzed too,
// so it skips the type-checking pass that user code goes through.
pub fn compile_bundled_to_proto(statements: &[Stmt]) -> Result<Proto, String> {
    compile_to_proto_via_ir_unchecked(statements)
}

fn codegen_proto(statements: &[Stmt]) -> Result<Proto, String> {
    let cost_model = crate::compiler::cost_model::CostModel::default();
    let _ast_cost = cost_model.program_cost(statements);
    let optimized_stmts = fold_program(statements.to_vec());
    let mut compiler = Compiler::new(Some("main".to_string()), 0, false);
    compiler.compile_program(&optimized_stmts);
    let proto = compiler.finish().map_err(|errors| {
        errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    let _proto_cost = cost_model.proto_cost(&proto);
    let _ = cost_model.should_inline(&proto);
    Ok(proto)
}

// Compile AST statements through full IR pipeline: AST -> IR -> CFG -> optimize_cfg -> bytecode Proto
pub fn try_compile_to_proto_via_ir(statements: &[Stmt]) -> Result<Proto, String> {
    let diags = crate::sema::analyze(statements, "");
    if let Some(err) = diags
        .iter()
        .find(|d| d.severity == crate::diagnostics::severity::Severity::Error)
    {
        return Err(format!("semantic error: {}", err.message));
    }
    compile_to_proto_via_ir_unchecked(statements)
}

// The IR pipeline without the semantic-analysis gate, for sources that have
// already been checked (or are trusted, like the bundled standard library).
fn compile_to_proto_via_ir_unchecked(statements: &[Stmt]) -> Result<Proto, String> {
    let optimized_stmts = fold_program(statements.to_vec());
    let mut ir_module = ir::ast_to_ir(&optimized_stmts);
    ir::inline_functions(&mut ir_module);
    optimize_ir_function(&mut ir_module.main);
    ir::verify_module(&ir_module)
        .map_err(|errs| format!("IR verification error: {}", errs.join(", ")))?;
    ir::ir_to_bytecode(&ir_module)
}

// The IR of a program before and after optimization, for debugging the
// pipeline.
pub fn dump_ir(statements: &[Stmt]) -> String {
    let optimized_stmts = fold_program(statements.to_vec());
    let mut ir_module = ir::ast_to_ir(&optimized_stmts);
    let mut out = String::from("=== before optimization ===\n");
    out.push_str(&ir::pretty::print_module(&ir_module));
    if let Err(errs) = ir::verify_module(&ir_module) {
        out.push_str(&format!("\n[verification warnings: {}]\n", errs.join(", ")));
    }
    ir::inline_functions(&mut ir_module);
    optimize_ir_function(&mut ir_module.main);
    out.push_str("\n=== after optimization ===\n");
    out.push_str(&ir::pretty::print_module(&ir_module));
    if let Err(errs) = ir::verify_module(&ir_module) {
        out.push_str(&format!("\n[verification warnings: {}]\n", errs.join(", ")));
    }
    out
}

// Runs the CFG optimizer over a function and every function nested in it.
fn optimize_ir_function(func: &mut ir::IrFunction) {
    let mut cfg = ir::build_cfg(&func.instructions);
    ir::optimize_cfg(&mut cfg);
    func.instructions = cfg.to_flat_instructions();
    func.cfg = Some(cfg);
    for child in &mut func.protos {
        optimize_ir_function(child);
    }
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
            p.emit(
                crate::bytecode::instruction::Instruction::LoadNil { dst: 0 },
                0,
            );
            p.emit(
                crate::bytecode::instruction::Instruction::Return { base: 0, count: 1 },
                0,
            );
            p.max_registers = 1;
            p
        }
    }
}

// Compile source directly into binary bytecode with magic bytes 'neyuki!'
pub fn compile_source_to_bytecode(source: &str) -> Result<Vec<u8>, String> {
    let stmts = compile_source(source)?;
    let diags = crate::sema::analyze(&stmts, source);
    if let Some(err) = diags
        .iter()
        .find(|d| d.severity == crate::diagnostics::severity::Severity::Error)
    {
        return Err(format!("semantic error: {}", err.message));
    }
    let proto = try_compile_to_proto(&stmts)?;
    Ok(serialize(&proto))
}

// Compile file to bytecode binary
pub fn compile_file_to_bytecode(path: &str) -> Result<Vec<u8>, String> {
    let source =
        fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;
    compile_source_to_bytecode(&source)
}

// Compile source directly into binary bytecode via IR pipeline
pub fn compile_source_to_bytecode_via_ir(source: &str) -> Result<Vec<u8>, String> {
    let stmts = compile_source(source)?;
    let diags = crate::sema::analyze(&stmts, source);
    if let Some(err) = diags
        .iter()
        .find(|d| d.severity == crate::diagnostics::severity::Severity::Error)
    {
        return Err(format!("semantic error: {}", err.message));
    }
    let proto = try_compile_to_proto_via_ir(&stmts)?;
    Ok(serialize(&proto))
}

// Compile file to bytecode binary via IR pipeline
pub fn compile_file_to_bytecode_via_ir(path: &str) -> Result<Vec<u8>, String> {
    let source =
        fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;
    compile_source_to_bytecode_via_ir(&source)
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

    #[test]
    fn test_compile_via_ir_parity_with_direct_compiler() {
        let code = "local a = 10\nlocal b = 20\nreturn a + b";
        let stmts = compile_source(code).expect("syntax error");
        let direct_proto = try_compile_to_proto(&stmts).expect("direct compilation failed");
        let ir_proto = try_compile_to_proto_via_ir(&stmts).expect("IR compilation failed");

        let mut vm_direct = crate::vm::machine::VM::new();
        let res_direct = vm_direct.execute(direct_proto).expect("direct exec failed");

        let mut vm_ir = crate::vm::machine::VM::new();
        let res_ir = vm_ir.execute(ir_proto).expect("IR exec failed");

        assert_eq!(res_direct.to_string(), res_ir.to_string());
    }

    #[test]
    fn test_ir_pipeline_inlines_small_leaf_call() {
        let code = "local function add(x, y) return x + y end\nlocal s = 0\nfor i = 1, 5 do s = s + add(i, i) end\nreturn s";
        let stmts = compile_source(code).expect("syntax error");
        let proto = try_compile_to_proto_via_ir(&stmts).expect("IR compilation failed");
        // The two-instruction leaf must be inlined: no Call left in main.
        assert!(
            !proto
                .instructions
                .iter()
                .any(|i| matches!(i, crate::bytecode::instruction::Instruction::Call { .. })),
            "expected inlined call, got:\n{}",
            crate::bytecode::disasm::disassemble_proto(&proto, 0)
        );

        let mut vm = crate::vm::machine::VM::new();
        let res = vm.execute(proto).expect("exec failed");
        assert_eq!(res.to_string(), "30");
    }
}
