// Intermediate Representation (IR) module for Neyuki compiler.
// Provides 3-Address Code / CFG-based intermediate representation between AST and Bytecode.

#![allow(dead_code)]
#![allow(unused_imports)]

pub mod block;
pub mod builder;
pub mod cfg_builder;
pub mod codegen;
pub mod dom;
pub mod inst;
pub mod liveness;
pub mod opt;
pub mod pretty;
pub mod regalloc;
pub mod ssa;
pub mod types;
pub mod verify;

pub use block::{BasicBlock, ControlFlowGraph, IrFunction, IrModule};
pub use builder::{IrBuilder, ast_to_ir};
pub use cfg_builder::build_cfg;
pub use codegen::ir_to_bytecode;
pub use dom::DominatorTree;
pub use inst::IrInst;
pub use liveness::{LiveInterval, LivenessInfo};
pub use opt::{
    common_subexpression_elimination, common_subexpression_elimination_cfg, constant_propagation,
    copy_propagation, copy_propagation_cfg, dead_code_elimination, dead_code_elimination_cfg,
    inline_functions, loop_invariant_code_motion, optimize_cfg, simplify_cfg,
};
pub use pretty::{format_inst, print_cfg, print_function, print_module};
pub use regalloc::{RegisterAllocation, allocate_registers};
pub use ssa::{construct_ssa, destruct_ssa, ssa_constant_propagation};
pub use types::{IrBinaryOp, IrConstant, IrLabel, IrUnaryOp, IrVar};
pub use verify::{verify_cfg, verify_function, verify_module};

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

    #[test]
    fn test_cfg_construction_and_dom_tree() {
        let code = "local x = 10\nif x > 5 then x = 20 else x = 30 end\nreturn x";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let cfg = build_cfg(&ir_module.main.instructions);

        assert!(cfg.blocks.len() >= 3);
        let dom = DominatorTree::build(&cfg);
        // Entry dominates entry
        assert!(dom.dominates(cfg.entry_label, cfg.entry_label));

        let liveness = LivenessInfo::compute(&cfg);
        let intervals = liveness.compute_live_intervals(&cfg);
        assert!(!intervals.is_empty());

        let reg_alloc = allocate_registers(&cfg, 0).expect("regalloc should succeed");
        assert!(reg_alloc.max_registers < 255);
    }

    #[test]
    fn test_ssa_construction_and_destruction() {
        let code = "local x = 1\nif x == 1 then x = 2 else x = 3 end\nreturn x";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let mut cfg = build_cfg(&ir_module.main.instructions);
        let dom = DominatorTree::build(&cfg);

        construct_ssa(&mut cfg, &dom);
        destruct_ssa(&mut cfg);
        let verify_result = verify_cfg(&cfg, 0);
        assert!(
            verify_result.is_ok(),
            "CFG verification failed: {:?}",
            verify_result
        );
    }

    #[test]
    fn test_cfg_optimizations() {
        let code = "local a = 10\nlocal b = 20\nlocal c = a + b\nlocal d = a + b\nreturn c";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let mut cfg = build_cfg(&ir_module.main.instructions);

        common_subexpression_elimination_cfg(&mut cfg);
        copy_propagation_cfg(&mut cfg);
        dead_code_elimination_cfg(&mut cfg);
        simplify_cfg(&mut cfg);

        let pretty_out = {
            let mut out = String::new();
            print_cfg(&cfg, 0, &mut out);
            out
        };
        assert!(!pretty_out.is_empty());
    }

    #[test]
    fn test_ssa_constant_propagation() {
        let code = "local x = 1\nif x == 1 then x = 42 else x = 42 end\nreturn x";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let mut cfg = build_cfg(&ir_module.main.instructions);
        let dom = DominatorTree::build(&cfg);

        construct_ssa(&mut cfg, &dom);
        let folded = ssa_constant_propagation(&mut cfg);
        assert!(
            folded,
            "SSA constant propagation should have folded the phi"
        );
        destruct_ssa(&mut cfg);

        let proto = ir_to_bytecode(&IrModule {
            main: IrFunction {
                name: Some("main".to_string()),
                params: Vec::new(),
                num_params: 0,
                is_vararg: false,
                instructions: cfg.to_flat_instructions(),
                protos: Vec::new(),
                upvalues: Vec::new(),
                cfg: Some(cfg),
            },
        })
        .expect("codegen failed");

        let mut vm = VM::new();
        let val = vm.execute(proto).expect("execution failed");
        assert_eq!(val.to_string(), "42");
    }

    #[test]
    fn test_cfg_fixpoint_optimization_driver() {
        let code = "local a = 10\nlocal b = 20\nlocal c = a + b\nlocal d = a + b\nlocal e = d\nreturn c + e";
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let mut cfg = build_cfg(&ir_module.main.instructions);

        optimize_cfg(&mut cfg);
        let verify_result = verify_cfg(&cfg, 0);
        assert!(verify_result.is_ok());
    }

    #[test]
    fn test_complex_control_flow_3_level_loops_and_closures() {
        let code = r#"
            local total = 0
            for i = 1, 3 do
                for j = 1, 3 do
                    local k = 1
                    while k <= 2 do
                        if k == 2 then
                            break
                        end
                        local function add_k(val)
                            return val + k + i + j
                        end
                        total = add_k(total)
                        k = k + 1
                    end
                end
            end
            return total
        "#;
        let stmts = compile_source(code).expect("syntax error");
        let ir_module = ast_to_ir(&stmts);
        let mut cfg = build_cfg(&ir_module.main.instructions);

        optimize_cfg(&mut cfg);
        let verify_result = verify_cfg(&cfg, 0);
        assert!(
            verify_result.is_ok(),
            "CFG verification failed: {:?}",
            verify_result.err()
        );

        let proto = ir_to_bytecode(&ir_module).expect("ir codegen failed");
        let mut vm = VM::new();
        let val = vm.execute(proto).expect("execution failed");
        // i: 1..3, j: 1..3 -> 9 iterations. In each: k=1, add_k(val) = val + 1 + i + j.
        // sum(1 + i + j) for i in 1..3, j in 1..3 = 9*1 + 3*(1+2+3) + 3*(1+2+3) = 9 + 18 + 18 = 45.
        assert_eq!(val.to_string(), "45");
    }
}
