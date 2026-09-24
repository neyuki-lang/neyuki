// Static Single Assignment (SSA) form construction and destruction.

use std::collections::{HashMap, HashSet};

use crate::compiler::ir::block::ControlFlowGraph;
use crate::compiler::ir::dom::DominatorTree;
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::{IrLabel, IrVar};

pub struct SsaContext {
    var_counter: u32,
}

impl SsaContext {
    pub fn new(max_existing_var: u32) -> Self {
        Self {
            var_counter: max_existing_var + 1,
        }
    }

    pub fn fresh_var(&mut self) -> IrVar {
        let v = IrVar(self.var_counter);
        self.var_counter += 1;
        v
    }
}

pub fn construct_ssa(cfg: &mut ControlFlowGraph, dom: &DominatorTree) {
    // 1. Find all variables defined in CFG and their defining blocks
    let mut max_var_id = 0u32;
    let mut def_blocks: HashMap<IrVar, HashSet<IrLabel>> = HashMap::new();

    for block in &cfg.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.def_var() {
                max_var_id = max_var_id.max(d.0);
                def_blocks.entry(d).or_default().insert(block.label);
            }
            for u in inst.use_vars() {
                max_var_id = max_var_id.max(u.0);
            }
        }
    }

    let mut ctx = SsaContext::new(max_var_id);

    // 2. Insert Phi nodes at dominance frontiers
    for (var, blocks) in def_blocks {
        let mut worklist: Vec<IrLabel> = blocks.iter().copied().collect();
        let mut has_phi: HashSet<IrLabel> = HashSet::new();

        while let Some(blk) = worklist.pop() {
            if let Some(frontier) = dom.frontiers.get(&blk) {
                for &f_label in frontier {
                    if has_phi.insert(f_label) {
                        if let Some(target_block) = cfg.find_block_mut(f_label) {
                            let phi = IrInst::Phi {
                                dst: var,
                                incoming: Vec::new(),
                            };
                            target_block.instructions.insert(0, phi);
                        }
                        worklist.push(f_label);
                    }
                }
            }
        }
    }

    // 3. Rename variables via dominator tree traversal
    let mut var_stacks: HashMap<IrVar, Vec<IrVar>> = HashMap::new();
    rename_block(cfg.entry_label, cfg, dom, &mut var_stacks, &mut ctx);
}

fn rename_block(
    block_label: IrLabel,
    cfg: &mut ControlFlowGraph,
    dom: &DominatorTree,
    stacks: &mut HashMap<IrVar, Vec<IrVar>>,
    ctx: &mut SsaContext,
) {
    let mut defs_in_block: Vec<IrVar> = Vec::new();

    // Process instructions in this block
    if let Some(block) = cfg.find_block_mut(block_label) {
        for inst in &mut block.instructions {
            match inst {
                IrInst::Phi { dst, .. } => {
                    let old_var = *dst;
                    let new_var = ctx.fresh_var();
                    *dst = new_var;
                    stacks.entry(old_var).or_default().push(new_var);
                    defs_in_block.push(old_var);
                }
                _ => {
                    // Rename uses
                    rename_inst_uses(inst, stacks);

                    // Rename def
                    if let Some(old_dst) = inst.def_var() {
                        let new_dst = ctx.fresh_var();
                        set_inst_def(inst, new_dst);
                        stacks.entry(old_dst).or_default().push(new_dst);
                        defs_in_block.push(old_dst);
                    }
                }
            }
        }
    }

    // Fill Phi nodes in successors
    let succs = cfg
        .find_block(block_label)
        .map(|b| b.successors.clone())
        .unwrap_or_default();

    for succ_label in succs {
        if let Some(succ_block) = cfg.find_block_mut(succ_label) {
            for inst in &mut succ_block.instructions {
                if let IrInst::Phi { incoming, .. } = inst {
                    // Look up current value for the phi's variable
                    if let Some(&top) = defs_in_block
                        .last()
                        .and_then(|last_def| stacks.get(last_def))
                        .and_then(|stack| stack.last())
                    {
                        incoming.push((block_label, top));
                    }
                }
            }
        }
    }

    // Traverse dominator tree children
    if let Some(children) = dom.children.get(&block_label).cloned() {
        for child in children {
            rename_block(child, cfg, dom, stacks, ctx);
        }
    }

    // Pop definitions made in this block
    for var in defs_in_block {
        if let Some(stack) = stacks.get_mut(&var) {
            stack.pop();
        }
    }
}

fn rename_inst_uses(inst: &mut IrInst, stacks: &HashMap<IrVar, Vec<IrVar>>) {
    let rename = |v: &mut IrVar| {
        if let Some(&top) = stacks.get(v).and_then(|s| s.last()) {
            *v = top;
        }
    };

    match inst {
        IrInst::Move { src, .. } | IrInst::UnOp { src, .. } | IrInst::AppendArray { src, .. } => {
            rename(src);
        }
        IrInst::BinOp { lhs, rhs, .. } => {
            rename(lhs);
            rename(rhs);
        }
        IrInst::GetTable { table, key, .. } => {
            rename(table);
            rename(key);
        }
        IrInst::SetTable { table, key, val } => {
            rename(table);
            rename(key);
            rename(val);
        }
        IrInst::SetGlobal { src, .. } | IrInst::SetUpval { src, .. } => {
            rename(src);
        }
        IrInst::Call { callee, args, .. } => {
            rename(callee);
            for arg in args {
                rename(arg);
            }
        }
        IrInst::Return(vars) => {
            for v in vars {
                rename(v);
            }
        }
        IrInst::JumpIfFalse { cond, .. } => {
            rename(cond);
        }
        IrInst::ForPrep {
            base, limit, step, ..
        } => {
            rename(base);
            rename(limit);
            rename(step);
        }
        IrInst::ForLoop { base, .. } | IrInst::TForLoop { base, .. } => {
            rename(base);
        }
        IrInst::TForCall {
            base, state, ctrl, ..
        } => {
            rename(base);
            rename(state);
            rename(ctrl);
        }
        _ => {}
    }
}

fn set_inst_def(inst: &mut IrInst, new_def: IrVar) {
    match inst {
        IrInst::LoadConst { dst, .. }
        | IrInst::LoadNil { dst }
        | IrInst::Move { dst, .. }
        | IrInst::BinOp { dst, .. }
        | IrInst::UnOp { dst, .. }
        | IrInst::NewTable { dst }
        | IrInst::GetTable { dst, .. }
        | IrInst::GetImport { dst, .. }
        | IrInst::GetGlobal { dst, .. }
        | IrInst::GetUpval { dst, .. }
        | IrInst::Closure { dst, .. }
        | IrInst::Vararg { dst, .. }
        | IrInst::Phi { dst, .. } => {
            *dst = new_def;
        }
        IrInst::Call { dsts, .. } => {
            // Only the first result is renamed; SSA does not model the rest.
            if let Some(first) = dsts.first_mut() {
                *first = new_def;
            }
        }
        _ => {}
    }
}

// Lowers SSA form back to conventional 3-Address Code by converting Phi nodes to parallel copies
pub fn destruct_ssa(cfg: &mut ControlFlowGraph) {
    let mut copies: Vec<(IrLabel, IrInst)> = Vec::new();

    for block in &mut cfg.blocks {
        let mut non_phi = Vec::new();

        for inst in block.instructions.drain(..) {
            if let IrInst::Phi { dst, incoming } = inst {
                for (pred_label, src_var) in incoming {
                    copies.push((pred_label, IrInst::Move { dst, src: src_var }));
                }
            } else {
                non_phi.push(inst);
            }
        }

        block.instructions = non_phi;
    }

    // Insert copies into predecessor blocks before their terminators
    for (pred_label, copy_inst) in copies {
        if let Some(pred_block) = cfg.find_block_mut(pred_label) {
            let term_pos = pred_block
                .instructions
                .iter()
                .position(|i| i.is_terminator());

            if let Some(pos) = term_pos {
                pred_block.instructions.insert(pos, copy_inst);
            } else {
                pred_block.instructions.push(copy_inst);
            }
        }
    }
}

use crate::compiler::ir::types::IrBinaryOp;
use crate::compiler::ir::types::IrConstant;

/// Sparse Constant & Copy Propagation pass directly operating on SSA form.
/// Folds constant Phi nodes, resolves compile-time binary operations, and propagates values.
pub fn ssa_constant_propagation(cfg: &mut ControlFlowGraph) -> bool {
    let mut const_values: HashMap<IrVar, IrConstant> = HashMap::new();
    let mut copy_values: HashMap<IrVar, IrVar> = HashMap::new();
    let mut changed = false;

    let mut passes = 0;
    const MAX_PASSES: usize = 16;

    while passes < MAX_PASSES {
        passes += 1;
        let mut pass_changed = false;

        for block in &mut cfg.blocks {
            for inst in &mut block.instructions {
                match inst {
                    IrInst::LoadConst { dst, val } => {
                        if const_values.insert(*dst, val.clone()).is_none() {
                            pass_changed = true;
                        }
                    }
                    IrInst::Move { dst, src } => {
                        let resolved_src = copy_values.get(src).copied().unwrap_or(*src);
                        if let Some(c) = const_values.get(&resolved_src).cloned() {
                            if const_values.insert(*dst, c).is_none() {
                                pass_changed = true;
                            }
                        } else if copy_values.insert(*dst, resolved_src).is_none() {
                            pass_changed = true;
                        }
                    }
                    IrInst::Phi { dst, incoming } => {
                        // Check if all incoming values are identical constants or the dst itself
                        let mut common_const: Option<IrConstant> = None;
                        let mut all_same_const = !incoming.is_empty();

                        for (_, src_var) in incoming.iter() {
                            if *src_var == *dst {
                                continue;
                            }
                            let resolved = copy_values.get(src_var).copied().unwrap_or(*src_var);
                            if let Some(c) = const_values.get(&resolved) {
                                if let Some(ref prev) = common_const {
                                    if prev != c {
                                        all_same_const = false;
                                        break;
                                    }
                                } else {
                                    common_const = Some(c.clone());
                                }
                            } else {
                                all_same_const = false;
                                break;
                            }
                        }

                        if let (true, Some(c)) = (all_same_const, common_const) {
                            if const_values.insert(*dst, c.clone()).is_none() {
                                pass_changed = true;
                            }
                            *inst = IrInst::LoadConst { dst: *dst, val: c };
                            continue;
                        }

                        // Check if all incoming values are the exact same variable
                        let mut common_var: Option<IrVar> = None;
                        let mut all_same_var = !incoming.is_empty();
                        for (_, src_var) in incoming.iter() {
                            if *src_var == *dst {
                                continue;
                            }
                            let resolved = copy_values.get(src_var).copied().unwrap_or(*src_var);
                            if let Some(prev) = common_var {
                                if prev != resolved {
                                    all_same_var = false;
                                    break;
                                }
                            } else {
                                common_var = Some(resolved);
                            }
                        }

                        if let (true, Some(src)) = (all_same_var, common_var) {
                            copy_values.insert(*dst, src);
                            *inst = IrInst::Move { dst: *dst, src };
                            pass_changed = true;
                        }
                    }
                    IrInst::BinOp { dst, op, lhs, rhs } => {
                        let c_lhs = const_values.get(lhs);
                        let c_rhs = const_values.get(rhs);

                        if let (Some(IrConstant::Int(a)), Some(IrConstant::Int(b))) = (c_lhs, c_rhs)
                        {
                            let folded = match op {
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
                            };
                            if let Some(val) = folded {
                                const_values.insert(*dst, val.clone());
                                *inst = IrInst::LoadConst { dst: *dst, val };
                                pass_changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if !pass_changed {
            break;
        }
        changed = true;
    }

    changed
}
