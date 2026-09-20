// Copy propagation optimization pass for IR.
// Eliminates redundant Move instructions by forwarding source variables.

use std::collections::HashMap;

use crate::compiler::ir::block::{ControlFlowGraph, IrModule};
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::IrVar;

pub fn copy_propagation(module: &mut IrModule) {
    let captured = std::collections::HashSet::new();
    propagate_copies_slice(&mut module.main.instructions, &captured);
}

pub fn copy_propagation_cfg(cfg: &mut ControlFlowGraph) -> bool {
    let captured = super::captured_vars(cfg);
    let mut changed = false;
    for block in &mut cfg.blocks {
        if propagate_copies_slice(&mut block.instructions, &captured) {
            changed = true;
        }
    }
    changed
}

fn propagate_copies_slice(
    instructions: &mut [IrInst],
    captured: &std::collections::HashSet<IrVar>,
) -> bool {
    let mut copies: HashMap<IrVar, IrVar> = HashMap::new();
    let mut modified = false;

    for inst in instructions.iter_mut() {
        // Resolve uses using current copy map
        let replace_var = |v: &mut IrVar, copies: &HashMap<IrVar, IrVar>| -> bool {
            let mut cur = *v;
            let mut rep = false;
            while let Some(&src) = copies.get(&cur) {
                cur = src;
                rep = true;
            }
            if rep {
                *v = cur;
            }
            rep
        };

        match inst {
            IrInst::Move { src, .. }
            | IrInst::UnOp { src, .. }
            | IrInst::AppendArray { src, .. } => {
                if replace_var(src, &copies) {
                    modified = true;
                }
            }
            IrInst::BinOp { lhs, rhs, .. } => {
                if replace_var(lhs, &copies) {
                    modified = true;
                }
                if replace_var(rhs, &copies) {
                    modified = true;
                }
            }
            IrInst::GetTable { table, key, .. } => {
                if replace_var(table, &copies) {
                    modified = true;
                }
                if replace_var(key, &copies) {
                    modified = true;
                }
            }
            IrInst::SetTable { table, key, val } => {
                if replace_var(table, &copies) {
                    modified = true;
                }
                if replace_var(key, &copies) {
                    modified = true;
                }
                if replace_var(val, &copies) {
                    modified = true;
                }
            }
            IrInst::SetGlobal { src, .. } | IrInst::SetUpval { src, .. } => {
                if replace_var(src, &copies) {
                    modified = true;
                }
            }
            IrInst::Call { callee, args, .. } => {
                if replace_var(callee, &copies) {
                    modified = true;
                }
                for arg in args {
                    if replace_var(arg, &copies) {
                        modified = true;
                    }
                }
            }
            IrInst::Return(vars) => {
                for v in vars {
                    if replace_var(v, &copies) {
                        modified = true;
                    }
                }
            }
            IrInst::JumpIfFalse { cond, .. } => {
                if replace_var(cond, &copies) {
                    modified = true;
                }
            }
            _ => {}
        }

        // Invalidate copies if destination or source is overwritten
        for def in inst.def_vars() {
            copies.remove(&def);
            copies.retain(|_, src| *src != def);
        }

        // Register new copy
        match inst {
            // A captured variable's register is what the closure reads, so
            // neither side of such a copy can be forwarded away.
            IrInst::Move { dst, src }
                if *dst != *src && !captured.contains(dst) && !captured.contains(src) =>
            {
                copies.insert(*dst, *src);
            }
            _ => {}
        }
    }

    modified
}
