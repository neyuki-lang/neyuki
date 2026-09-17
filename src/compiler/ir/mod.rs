// Intermediate Representation (IR) module for Neyuki compiler.
// Provides 3-Address Code / CFG-based intermediate representation between AST and Bytecode.

#![allow(dead_code)]
#![allow(unused_imports)]

pub mod block;
pub mod builder;
pub mod codegen;
pub mod inst;
pub mod opt;
pub mod types;

#[allow(unused_imports)]
pub use block::{BasicBlock, ControlFlowGraph, IrFunction, IrModule};
#[allow(unused_imports)]
pub use builder::{IrBuilder, ast_to_ir};
pub use codegen::ir_to_bytecode;
pub use inst::IrInst;
pub use opt::{constant_propagation, dead_code_elimination};
#[allow(unused_imports)]
pub use types::{IrBinaryOp, IrConstant, IrLabel, IrUnaryOp, IrVar};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile_source;
    use crate::vm::machine::VM;
    use num_bigint::BigInt;

    #[test]
    fn test_ir_basic() {
        let stmts = vec![];
        let mut module = ast_to_ir(&stmts);
        dead_code_elimination(&mut module);
        constant_propagation(&mut module);
        let proto = ir_to_bytecode(&module).expect("ir lowering failed");
        assert_eq!(proto.name, Some("main".to_string()));
    }

    #[test]
    fn test_ast_to_ir_and_constant_folding() {
        let code = "local a = 10 + 20\nreturn a";
        let stmts = compile_source(code).expect("syntax error");
        let mut ir_module = ast_to_ir(&stmts);

        let has_binop_before = ir_module
            .main
            .instructions
            .iter()
            .any(|i| matches!(i, IrInst::BinOp { .. }));
        assert!(has_binop_before);

        constant_propagation(&mut ir_module);

        let has_folded_30 = ir_module.main.instructions.iter().any(|i| {
            if let IrInst::LoadConst {
                val: IrConstant::Int(bi),
                ..
            } = i
            {
                bi == &BigInt::from(30)
            } else {
                false
            }
        });
        assert!(has_folded_30);

        dead_code_elimination(&mut ir_module);

        let proto = ir_to_bytecode(&ir_module).expect("ir lowering failed");
        let mut vm = VM::new();
        let res = vm.execute(proto).expect("execution failed");
        assert_eq!(res.to_string(), "30");
    }

    #[test]
    fn test_ir_if_control_flow() {
        let code =
            "local x = 5\nlocal res = 0\nif x == 5 then res = 100 else res = 200 end\nreturn res";
        let stmts = compile_source(code).expect("syntax error");
        let mut ir_module = ast_to_ir(&stmts);
        constant_propagation(&mut ir_module);
        dead_code_elimination(&mut ir_module);
        let proto = ir_to_bytecode(&ir_module).expect("ir lowering failed");

        let mut vm = VM::new();
        let val = vm.execute(proto).expect("exec failed");
        assert_eq!(val.to_string(), "100");
    }

    #[test]
    fn test_ir_while_loop() {
        let code =
            "local sum = 0\nlocal i = 1\nwhile i <= 4 do sum = sum + i; i = i + 1 end\nreturn sum";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let proto = ir_to_bytecode(&ir_module).expect("ir lowering failed");

        let mut vm = VM::new();
        let val = vm.execute(proto).expect("exec failed");
        assert_eq!(val.to_string(), "10");
    }

    #[test]
    fn test_ir_repeat_and_interpolation() {
        let code = "local a = 0\nrepeat a += 1 until a == 3\nlocal msg = `val: {a}`\nreturn msg";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let proto = ir_to_bytecode(&ir_module).expect("ir lowering failed");

        let mut vm = VM::new();
        let val = vm.execute(proto).expect("exec failed");
        assert_eq!(val.to_string(), "val: 3");
    }

    #[test]
    fn test_ir_closure_and_sub_function() {
        let code = "local function make_adder(x)\n  return x + 5\nend\nreturn make_adder(10)";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let proto = ir_to_bytecode(&ir_module).expect("ir lowering failed");

        let mut vm = VM::new();
        let val = vm.execute(proto).expect("exec failed");
        assert_eq!(val.to_string(), "15");
    }

    #[test]
    fn test_ir_depth_limit_rejected() {
        // Construct an IrModule with 65 levels of nested functions
        let mut curr = IrFunction::new(Some("level_65".to_string()), 0, false);
        for lvl in (0..65).rev() {
            let mut parent = IrFunction::new(Some(format!("level_{}", lvl)), 0, false);
            parent.protos.push(curr);
            curr = parent;
        }
        let module = IrModule { main: curr };
        let result = ir_to_bytecode(&module);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("depth limit (64) exceeded"));
    }

    #[test]
    fn test_ir_and_or_short_circuit() {
        let code = "local a = false and 10 or 20\nreturn a";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let proto = ir_to_bytecode(&ir_module).expect("ir lowering failed");

        let mut vm = VM::new();
        let val = vm.execute(proto).expect("exec failed");
        assert_eq!(val.to_string(), "20");
    }

    #[test]
    fn test_ir_numeric_for() {
        let code = "local sum = 0\nfor i = 1, 5 do\n  sum = sum + i\nend\nreturn sum";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let proto = ir_to_bytecode(&ir_module).expect("ir lowering failed");

        let mut vm = VM::new();
        let val = vm.execute(proto).expect("exec failed");
        assert_eq!(val.to_string(), "15");
    }
}
