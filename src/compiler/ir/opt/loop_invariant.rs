// Loop Invariant Code Motion (LICM) pass on CFG.
// Detects natural loops using dominator analysis and hoists invariant pure expressions.

use std::collections::{HashMap, HashSet};

use crate::compiler::ir::block::ControlFlowGraph;
use crate::compiler::ir::dom::DominatorTree;
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::{IrLabel, IrVar};

#[derive(Debug, Clone)]
pub struct NaturalLoop {
    pub header: IrLabel,
    pub back_edge_source: IrLabel,
    pub body: HashSet<IrLabel>,
}

// Identifies all natural loops in the CFG using dominance information
pub fn find_natural_loops(cfg: &ControlFlowGraph, dom: &DominatorTree) -> Vec<NaturalLoop> {
    let mut loops = Vec::new();

    for block in &cfg.blocks {
        let n = block.label;
        for &h in &block.successors {
            // If h dominates n, then (n -> h) is a back-edge
            if dom.dominates(h, n) {
                let mut body = HashSet::new();
                body.insert(h);
                body.insert(n);

                let mut stack = vec![n];
                while let Some(current) = stack.pop() {
                    if let Some(blk) = cfg.find_block(current) {
                        for &pred in &blk.predecessors {
                            if body.insert(pred) {
                                stack.push(pred);
                            }
                        }
                    }
                }

                loops.push(NaturalLoop {
                    header: h,
                    back_edge_source: n,
                    body,
                });
            }
        }
    }

    loops
}

/// Whether an instruction may run earlier, and possibly more or fewer times,
/// than written. Values are dynamically typed, so even `a + b` can raise an
/// error or call a metamethod; only loading a constant or copying a variable
/// is free of any observable effect. A table constructor is pure for dead
/// code purposes but must still run once per iteration.
fn is_hoistable(inst: &IrInst) -> bool {
    matches!(
        inst,
        IrInst::LoadConst { .. } | IrInst::LoadNil { .. } | IrInst::Move { .. }
    )
}

// Hoists loop-invariant pure instructions out of natural loops
pub fn loop_invariant_code_motion(cfg: &mut ControlFlowGraph, dom: &DominatorTree) -> bool {
    let captured = super::captured_vars(cfg);
    let loops = find_natural_loops(cfg, dom);
    let mut changed = false;

    // The IR is not in SSA form here: a variable assigned more than once
    // anywhere in the function is a real local whose value at a given point
    // depends on which assignment ran last, so none of its assignments can
    // move. Only single-assignment temporaries are candidates.
    let mut def_counts: HashMap<IrVar, usize> = HashMap::new();
    for block in &cfg.blocks {
        for inst in &block.instructions {
            for def in inst.def_vars() {
                *def_counts.entry(def).or_insert(0) += 1;
            }
        }
    }

    for lp in loops {
        // Collect all variables defined inside the loop
        let mut loop_defs = HashSet::new();
        for &block_lbl in &lp.body {
            if let Some(blk) = cfg.find_block(block_lbl) {
                for inst in &blk.instructions {
                    for def in inst.def_vars() {
                        loop_defs.insert(def);
                    }
                }
            }
        }

        // Find loop preheader: a predecessor of header outside the loop
        let preheader_opt = cfg.find_block(lp.header).and_then(|h| {
            h.predecessors
                .iter()
                .find(|p| !lp.body.contains(p))
                .copied()
        });

        let Some(preheader_label) = preheader_opt else {
            continue;
        };

        let mut invariant_defs = HashSet::new();
        let mut hoisted_insts = Vec::new();

        // Scan loop blocks for invariant instructions
        for &block_lbl in &lp.body {
            if block_lbl == lp.header {
                continue;
            }

            let Some(blk) = cfg.find_block_mut(block_lbl) else {
                continue;
            };

            let mut remaining = Vec::new();
            for inst in blk.instructions.drain(..) {
                if is_hoistable(&inst)
                    && !inst.def_vars().iter().any(|d| captured.contains(d))
                    && inst
                        .def_vars()
                        .iter()
                        .all(|d| def_counts.get(d).copied().unwrap_or(0) == 1)
                {
                    let uses = inst.use_vars();
                    let is_invariant = uses
                        .iter()
                        .all(|u| !loop_defs.contains(u) || invariant_defs.contains(u));

                    if is_invariant {
                        if let Some(d) = inst.def_var() {
                            invariant_defs.insert(d);
                        }
                        hoisted_insts.push(inst);
                        changed = true;
                        continue;
                    }
                }
                remaining.push(inst);
            }
            blk.instructions = remaining;
        }

        // Insert hoisted instructions into preheader before terminator
        if hoisted_insts.is_empty() {
            continue;
        }

        if let Some(preheader) = cfg.find_block_mut(preheader_label) {
            let term_pos = preheader
                .instructions
                .iter()
                .position(|i| i.is_terminator())
                .unwrap_or(preheader.instructions.len());

            for (offset, inst) in hoisted_insts.into_iter().enumerate() {
                preheader.instructions.insert(term_pos + offset, inst);
            }
        }
    }

    changed
}
