// Common Subexpression Elimination (CSE) pass on IR.
// Replaces repeated identical pure computations with Move from the first computed variable.

use std::collections::HashMap;

use crate::compiler::ir::block::{ControlFlowGraph, IrModule};
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::{IrBinaryOp, IrUnaryOp, IrVar};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ExpressionKey {
    BinOp(IrBinaryOp, IrVar, IrVar),
    UnOp(IrUnaryOp, IrVar),
}

fn canonical_binop_key(op: IrBinaryOp, lhs: IrVar, rhs: IrVar) -> ExpressionKey {
    match op {
        IrBinaryOp::Add
        | IrBinaryOp::Mul
        | IrBinaryOp::Eq
        | IrBinaryOp::Ne
        | IrBinaryOp::BitAnd
        | IrBinaryOp::BitOr
        | IrBinaryOp::BitXor => {
            if lhs.0 <= rhs.0 {
                ExpressionKey::BinOp(op, lhs, rhs)
            } else {
                ExpressionKey::BinOp(op, rhs, lhs)
            }
        }
        _ => ExpressionKey::BinOp(op, lhs, rhs),
    }
}

pub fn common_subexpression_elimination(module: &mut IrModule) {
    cse_slice(&mut module.main.instructions);
}

pub fn common_subexpression_elimination_cfg(cfg: &mut ControlFlowGraph) -> bool {
    let mut changed = false;
    for block in &mut cfg.blocks {
        if cse_slice(&mut block.instructions) {
            changed = true;
        }
    }
    changed
}

fn cse_slice(instructions: &mut [IrInst]) -> bool {
    let mut expr_map: HashMap<ExpressionKey, IrVar> = HashMap::new();
    let mut modified = false;

    for inst in instructions.iter_mut() {
        match inst {
            IrInst::BinOp { dst, op, lhs, rhs } => {
                let key = canonical_binop_key(*op, *lhs, *rhs);
                if let Some(&existing_var) = expr_map.get(&key) {
                    if existing_var != *dst {
                        *inst = IrInst::Move {
                            dst: *dst,
                            src: existing_var,
                        };
                        modified = true;
                    }
                } else {
                    let d = *dst;
                    invalidate_expressions(&mut expr_map, d);
                    expr_map.insert(key, d);
                    continue;
                }
            }
            IrInst::UnOp { dst, op, src } => {
                let key = ExpressionKey::UnOp(*op, *src);
                if let Some(&existing_var) = expr_map.get(&key) {
                    if existing_var != *dst {
                        *inst = IrInst::Move {
                            dst: *dst,
                            src: existing_var,
                        };
                        modified = true;
                    }
                } else {
                    let d = *dst;
                    invalidate_expressions(&mut expr_map, d);
                    expr_map.insert(key, d);
                    continue;
                }
            }
            _ => {}
        }

        // Any instruction that modifies a variable invalidates expressions using or defining it
        if let Some(def) = inst.def_var() {
            invalidate_expressions(&mut expr_map, def);
        }
    }

    modified
}

fn invalidate_expressions(map: &mut HashMap<ExpressionKey, IrVar>, modified_var: IrVar) {
    map.retain(|key, &mut target_var| {
        if target_var == modified_var {
            return false;
        }
        match key {
            ExpressionKey::BinOp(_, lhs, rhs) => *lhs != modified_var && *rhs != modified_var,
            ExpressionKey::UnOp(_, src) => *src != modified_var,
        }
    });
}
