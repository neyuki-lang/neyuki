// Dead Code Elimination (DCE) and unreachable code removal pass on IR.

use std::collections::HashSet;

use crate::compiler::ir::block::{ControlFlowGraph, IrModule};
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::liveness::LivenessInfo;
use crate::compiler::ir::types::IrVar;

pub fn is_pure_instruction(inst: &IrInst) -> bool {
    matches!(
        inst,
        IrInst::LoadConst { .. }
            | IrInst::LoadNil { .. }
            | IrInst::Move { .. }
            | IrInst::BinOp { .. }
            | IrInst::UnOp { .. }
            | IrInst::NewTable { .. }
            | IrInst::Phi { .. }
    )
}

// CFG-aware Dead Code Elimination using backward dataflow liveness analysis
pub fn dead_code_elimination_cfg(cfg: &mut ControlFlowGraph) -> bool {
    let captured = super::captured_vars(cfg);
    let mut any_changed = false;
    let mut changed = true;
    let mut iterations = 0;
    const MAX_PASSES: usize = 20;

    while changed && iterations < MAX_PASSES {
        changed = false;
        iterations += 1;

        let liveness = LivenessInfo::compute(cfg);

        for block in &mut cfg.blocks {
            // Forward pass: indices of stores with statically-nil values.
            // Clearing a register is GC-observable (it releases whatever
            // reference the register may still hold), so a nil-store is
            // never dead code — even when its target is never read again
            // (the explicit-release idiom `t = nil`). Chains like
            // `v10 = nil; v1 = v10` propagate nil-ness forward.
            let mut nil_known: HashSet<IrVar> = HashSet::new();
            let mut nil_stores: HashSet<usize> = HashSet::new();
            for (idx, inst) in block.instructions.iter().enumerate() {
                match inst {
                    IrInst::LoadNil { dst } => {
                        nil_known.insert(*dst);
                        nil_stores.insert(idx);
                    }
                    IrInst::Move { dst, src } => {
                        nil_known.remove(dst);
                        if nil_known.contains(src) {
                            nil_known.insert(*dst);
                            nil_stores.insert(idx);
                        }
                    }
                    _ => {
                        for d in inst.def_vars() {
                            nil_known.remove(&d);
                        }
                    }
                }
            }

            let mut live_now = liveness
                .blocks
                .get(&block.label)
                .map(|b| b.live_out.clone())
                .unwrap_or_default();

            let mut retained = Vec::new();

            let mut pos = block.instructions.len();
            for inst in block.instructions.drain(..).rev() {
                pos -= 1;
                let defs = inst.def_vars();
                let is_dead = is_pure_instruction(&inst)
                    && !nil_stores.contains(&pos)
                    && !defs.is_empty()
                    && defs
                        .iter()
                        .all(|dst| !live_now.contains(dst) && !captured.contains(dst));
                if is_dead {
                    // Dead instruction, eliminate it
                    changed = true;
                    any_changed = true;
                    continue;
                }

                // Update live_now backwards: remove def, add uses
                for dst in defs {
                    live_now.remove(&dst);
                }
                for u in inst.use_vars() {
                    live_now.insert(u);
                }

                retained.push(inst);
            }

            retained.reverse();
            block.instructions = retained;
        }
    }

    any_changed
}

pub fn dead_code_elimination(module: &mut IrModule) {
    let mut used_vars: HashSet<IrVar> = HashSet::new();

    for inst in &module.main.instructions {
        for u in inst.use_vars() {
            used_vars.insert(u);
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
