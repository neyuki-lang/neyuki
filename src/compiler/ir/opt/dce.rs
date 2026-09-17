// Dead Code Elimination (DCE) and unreachable code removal pass on IR.

use std::collections::HashSet;

use crate::compiler::ir::block::IrModule;
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::IrVar;

pub fn dead_code_elimination(module: &mut IrModule) {
    let mut used_vars: HashSet<IrVar> = HashSet::new();

    for inst in &module.main.instructions {
        match inst {
            IrInst::Move { src, .. } => {
                used_vars.insert(*src);
            }
            IrInst::BinOp { lhs, rhs, .. } => {
                used_vars.insert(*lhs);
                used_vars.insert(*rhs);
            }
            IrInst::UnOp { src, .. } => {
                used_vars.insert(*src);
            }
            IrInst::GetTable { table, key, .. } => {
                used_vars.insert(*table);
                used_vars.insert(*key);
            }
            IrInst::SetTable { table, key, val } => {
                used_vars.insert(*table);
                used_vars.insert(*key);
                used_vars.insert(*val);
            }
            IrInst::AppendArray { table, src } => {
                used_vars.insert(*table);
                used_vars.insert(*src);
            }
            IrInst::SetGlobal { src, .. } => {
                used_vars.insert(*src);
            }
            IrInst::Call { callee, args, .. } => {
                used_vars.insert(*callee);
                for arg in args {
                    used_vars.insert(*arg);
                }
            }
            IrInst::Return(vars) => {
                for v in vars {
                    used_vars.insert(*v);
                }
            }
            IrInst::JumpIfFalse { cond, .. } => {
                used_vars.insert(*cond);
            }
            _ => {}
        }
    }

    let mut filtered_insts = Vec::new();
    let mut unreachable = false;

    for inst in module.main.instructions.drain(..) {
        if unreachable {
            if let IrInst::Label(_) = inst {
                unreachable = false;
                filtered_insts.push(inst);
            }
            continue;
        }

        match &inst {
            IrInst::LoadConst { dst, .. } | IrInst::LoadNil { dst } => {
                if used_vars.contains(dst) {
                    filtered_insts.push(inst);
                }
            }
            IrInst::BinOp { dst, .. } | IrInst::UnOp { dst, .. } => {
                if used_vars.contains(dst) {
                    filtered_insts.push(inst);
                }
            }
            IrInst::Return(_) => {
                filtered_insts.push(inst);
                unreachable = true;
            }
            IrInst::Jump(_) => {
                filtered_insts.push(inst);
                unreachable = true;
            }
            _ => {
                filtered_insts.push(inst);
            }
        }
    }

    module.main.instructions = filtered_insts;
}
