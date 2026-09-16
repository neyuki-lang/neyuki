// Constant propagation and folding pass on IR.

use std::collections::HashMap;

use crate::compiler::ir::block::IrModule;
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::{IrBinaryOp, IrConstant, IrVar};

pub fn constant_propagation(module: &mut IrModule) {
    let mut const_map: HashMap<IrVar, IrConstant> = HashMap::new();
    let mut optimized_insts = Vec::new();

    for inst in module.main.instructions.drain(..) {
        match inst {
            IrInst::LoadConst { dst, val } => {
                const_map.insert(dst, val.clone());
                optimized_insts.push(IrInst::LoadConst { dst, val });
            }
            IrInst::BinOp { dst, op, lhs, rhs } => {
                let folded = match (const_map.get(&lhs), const_map.get(&rhs)) {
                    (Some(IrConstant::Int(a)), Some(IrConstant::Int(b))) => match op {
                        IrBinaryOp::Add => Some(IrConstant::Int(a + b)),
                        IrBinaryOp::Sub => Some(IrConstant::Int(a - b)),
                        IrBinaryOp::Mul => Some(IrConstant::Int(a * b)),
                        IrBinaryOp::Eq => Some(IrConstant::Bool(a == b)),
                        IrBinaryOp::Ne => Some(IrConstant::Bool(a != b)),
                        IrBinaryOp::Lt => Some(IrConstant::Bool(a < b)),
                        IrBinaryOp::Le => Some(IrConstant::Bool(a <= b)),
                        IrBinaryOp::Gt => Some(IrConstant::Bool(a > b)),
                        IrBinaryOp::Ge => Some(IrConstant::Bool(a >= b)),
                        _ => None,
                    },
                    (Some(IrConstant::String(a)), Some(IrConstant::String(b))) => match op {
                        IrBinaryOp::Concat => Some(IrConstant::String(format!("{}{}", a, b))),
                        _ => None,
                    },
                    _ => None,
                };

                if let Some(folded_val) = folded {
                    const_map.insert(dst, folded_val.clone());
                    optimized_insts.push(IrInst::LoadConst { dst, val: folded_val });
                } else {
                    optimized_insts.push(IrInst::BinOp { dst, op, lhs, rhs });
                }
            }
            other => {
                optimized_insts.push(other);
            }
        }
    }

    module.main.instructions = optimized_insts;
}
