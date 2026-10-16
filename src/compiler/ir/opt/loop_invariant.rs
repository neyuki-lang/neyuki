//! Loop invariant code motion: pure expressions that do not change inside
//! a loop move in front of it.

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

                if n != h {
                    let mut stack = vec![n];
                    while let Some(current) = stack.pop() {
                        if let Some(blk) = cfg.find_block(current) {
                            for &pred in &blk.predecessors {
                                if dom.dominates(h, pred) && body.insert(pred) {
                                    stack.push(pred);
                                }
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

/// Whether an instruction in a loop body may run earlier without observable effect.
fn is_hoistable_body(inst: &IrInst) -> bool {
    matches!(
        inst,
        IrInst::LoadConst { .. } | IrInst::LoadNil { .. } | IrInst::Move { .. }
    )
}

/// In the loop header, instructions before the exit test run unconditionally on loop entry.
fn is_hoistable_header(inst: &IrInst) -> bool {
    match inst {
        IrInst::LoadConst { .. } | IrInst::LoadNil { .. } | IrInst::Move { .. } => true,
        IrInst::BinOp { op, .. } => matches!(
            op,
            crate::compiler::ir::types::IrBinaryOp::Add
                | crate::compiler::ir::types::IrBinaryOp::Sub
                | crate::compiler::ir::types::IrBinaryOp::Mul
                | crate::compiler::ir::types::IrBinaryOp::BitAnd
                | crate::compiler::ir::types::IrBinaryOp::BitOr
                | crate::compiler::ir::types::IrBinaryOp::BitXor
                | crate::compiler::ir::types::IrBinaryOp::Eq
                | crate::compiler::ir::types::IrBinaryOp::Ne
                | crate::compiler::ir::types::IrBinaryOp::Lt
                | crate::compiler::ir::types::IrBinaryOp::Le
                | crate::compiler::ir::types::IrBinaryOp::Gt
                | crate::compiler::ir::types::IrBinaryOp::Ge
        ),
        IrInst::UnOp { op, .. } => matches!(
            op,
            crate::compiler::ir::types::IrUnaryOp::Neg
                | crate::compiler::ir::types::IrUnaryOp::Not
                | crate::compiler::ir::types::IrUnaryOp::BitNot
        ),
        _ => false,
    }
}

/// Ensures the loop header has a dedicated pre-header block outside the loop.
/// If not already present, creates and inserts one, redirecting outside predecessors.
fn ensure_preheader(
    cfg: &mut ControlFlowGraph,
    lp: &NaturalLoop,
    next_label_id: &mut usize,
) -> (IrLabel, bool) {
    let outside_preds: Vec<IrLabel> = match cfg.find_block(lp.header) {
        Some(h) => h
            .predecessors
            .iter()
            .filter(|p| !lp.body.contains(p))
            .copied()
            .collect(),
        None => return (lp.header, false),
    };

    if outside_preds.len() == 1 {
        let p = outside_preds[0];
        if p != lp.header
            && let Some(blk) = cfg.find_block(p)
            && blk.successors.len() == 1
            && blk.successors[0] == lp.header
        {
            return (p, false);
        }
    }

    *next_label_id += 1;
    let preheader_label = IrLabel(*next_label_id);

    let mut preheader_block = crate::compiler::ir::BasicBlock::new(preheader_label);
    preheader_block.instructions.push(IrInst::Jump(lp.header));
    preheader_block.successors.push(lp.header);
    preheader_block.predecessors = outside_preds.clone();

    for &pred_lbl in &outside_preds {
        if let Some(pred_blk) = cfg.find_block_mut(pred_lbl) {
            for inst in &mut pred_blk.instructions {
                match inst {
                    IrInst::Jump(target) if *target == lp.header => {
                        *target = preheader_label;
                    }
                    IrInst::JumpIfFalse { target, .. } if *target == lp.header => {
                        *target = preheader_label;
                    }
                    IrInst::ForPrep { jump, .. }
                    | IrInst::ForLoop { jump, .. }
                    | IrInst::TForLoop { jump, .. }
                        if *jump == lp.header =>
                    {
                        *jump = preheader_label;
                    }
                    _ => {}
                }
            }
            for succ in &mut pred_blk.successors {
                if *succ == lp.header {
                    *succ = preheader_label;
                }
            }
        }
    }

    if let Some(header_blk) = cfg.find_block_mut(lp.header) {
        header_blk.predecessors.retain(|p| lp.body.contains(p));
        header_blk.predecessors.push(preheader_label);
    }

    if cfg.entry_label == lp.header {
        cfg.entry_label = preheader_label;
    }

    let header_pos = cfg
        .blocks
        .iter()
        .position(|b| b.label == lp.header)
        .unwrap_or(0);
    cfg.blocks.insert(header_pos, preheader_block);

    (preheader_label, true)
}

// Hoists loop-invariant pure instructions out of natural loops
pub fn loop_invariant_code_motion(cfg: &mut ControlFlowGraph, dom: &DominatorTree) -> bool {
    let captured = super::captured_vars(cfg);
    let loops = find_natural_loops(cfg, dom);
    if loops.is_empty() {
        return false;
    }
    let mut changed = false;

    let mut next_label_id = cfg.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);

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

        let mut invariant_defs = HashSet::new();
        let mut hoisted_insts = Vec::new();

        // In layout order
        let body_in_layout_order: Vec<IrLabel> = cfg
            .blocks
            .iter()
            .map(|blk| blk.label)
            .filter(|lbl| lp.body.contains(lbl))
            .collect();

        // 1. Scan loop header for invariant instructions before the first branch/terminator
        if let Some(header_blk) = cfg.find_block_mut(lp.header) {
            let mut remaining = Vec::new();
            let mut past_branch = false;

            for inst in header_blk.instructions.drain(..) {
                if !past_branch
                    && (inst.is_terminator() || matches!(inst, IrInst::JumpIfFalse { .. }))
                {
                    past_branch = true;
                }

                if !past_branch
                    && is_hoistable_header(&inst)
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
            header_blk.instructions = remaining;
        }

        // 2. Scan remaining loop blocks for invariant instructions
        for block_lbl in body_in_layout_order {
            if block_lbl == lp.header {
                continue;
            }

            let Some(blk) = cfg.find_block_mut(block_lbl) else {
                continue;
            };

            let mut remaining = Vec::new();
            for inst in blk.instructions.drain(..) {
                if is_hoistable_body(&inst)
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

        let (preheader_label, _) = ensure_preheader(cfg, &lp, &mut next_label_id);
        changed = true;

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
