// Liveness analysis on Control Flow Graph using backward dataflow equations.

use std::collections::{HashMap, HashSet};

use crate::compiler::ir::block::ControlFlowGraph;
use crate::compiler::ir::types::{IrLabel, IrVar};

#[derive(Debug, Clone)]
pub struct BlockLiveness {
    pub use_vars: HashSet<IrVar>,
    pub def_vars: HashSet<IrVar>,
    pub live_in: HashSet<IrVar>,
    pub live_out: HashSet<IrVar>,
}

#[derive(Debug, Clone)]
pub struct LivenessInfo {
    pub blocks: HashMap<IrLabel, BlockLiveness>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveInterval {
    pub var: IrVar,
    pub start: usize,
    pub end: usize,
}

impl LivenessInfo {
    pub fn compute(cfg: &ControlFlowGraph) -> Self {
        let mut blocks = HashMap::new();

        // 1. Calculate local use and def sets for each basic block
        for block in &cfg.blocks {
            let mut block_use = HashSet::new();
            let mut block_def = HashSet::new();

            for inst in &block.instructions {
                for u in inst.use_vars() {
                    if !block_def.contains(&u) {
                        block_use.insert(u);
                    }
                }
                for d in inst.def_vars() {
                    block_def.insert(d);
                }
            }

            blocks.insert(
                block.label,
                BlockLiveness {
                    use_vars: block_use,
                    def_vars: block_def,
                    live_in: HashSet::new(),
                    live_out: HashSet::new(),
                },
            );
        }

        // 2. Iterative backward dataflow analysis
        // out[B] = Union_{S in succ[B]} in[S]
        // in[B] = use[B] Union (out[B] \ def[B])
        //
        // Blocks are visited in postorder (successors before predecessors),
        // so straight-line dependency chains settle in one round and each
        // loop nesting level costs roughly one more round. Visiting in raw
        // layout order can propagate only one block per round and hit the
        // iteration cap on loop-heavy functions.
        let succ_map: HashMap<IrLabel, Vec<IrLabel>> = cfg
            .blocks
            .iter()
            .map(|block| (block.label, block.successors.clone()))
            .collect();
        let order = Self::reverse_postorder(cfg);
        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 1000;

        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            iterations += 1;

            for lbl in &order {
                // Compute out[B]
                let mut new_out = HashSet::new();
                if let Some(successors) = succ_map.get(lbl) {
                    for succ in successors {
                        if let Some(succ_info) = blocks.get(succ) {
                            new_out.extend(succ_info.live_in.iter().copied());
                        }
                    }
                }

                // Compute in[B] = use[B] Union (out[B] \ def[B]), borrowing
                // the stored sets instead of cloning them every round.
                let b = &blocks[lbl];
                let mut new_in = HashSet::with_capacity(b.use_vars.len() + new_out.len());
                new_in.extend(b.use_vars.iter().copied());
                for v in &new_out {
                    if !b.def_vars.contains(v) {
                        new_in.insert(*v);
                    }
                }

                let cur = blocks.get_mut(lbl).expect("block exists");
                if cur.live_out != new_out || cur.live_in != new_in {
                    cur.live_out = new_out;
                    cur.live_in = new_in;
                    changed = true;
                }
            }
        }

        Self { blocks }
    }

    /// Postorder of the CFG from the entry (successors before predecessors),
    /// with blocks unreachable from the entry appended in layout order. A
    /// backward analysis that visits blocks in this order propagates straight
    /// chains in one round and each loop level in one more, instead of one
    /// block per round.
    fn reverse_postorder(cfg: &ControlFlowGraph) -> Vec<IrLabel> {
        let mut visited: HashSet<IrLabel> = HashSet::new();
        let mut post: Vec<IrLabel> = Vec::new();
        // Seed with the entry plus every block, so unreachable blocks are covered
        // while the entry's reachable region still comes first.
        let mut seeds: Vec<IrLabel> = vec![cfg.entry_label];
        for block in &cfg.blocks {
            seeds.push(block.label);
        }
        for seed in seeds {
            if visited.contains(&seed) {
                continue;
            }
            let mut stack: Vec<(IrLabel, bool)> = vec![(seed, false)];
            while let Some((lbl, expanded)) = stack.pop() {
                if expanded {
                    post.push(lbl);
                    continue;
                }
                if !visited.insert(lbl) {
                    continue;
                }
                stack.push((lbl, true));
                if let Some(block) = cfg.find_block(lbl) {
                    for succ in &block.successors {
                        if !visited.contains(succ) {
                            stack.push((*succ, false));
                        }
                    }
                }
            }
        }
        post
    }

    pub fn is_live_in(&self, label: IrLabel, var: IrVar) -> bool {
        self.blocks
            .get(&label)
            .is_some_and(|b| b.live_in.contains(&var))
    }

    pub fn is_live_out(&self, label: IrLabel, var: IrVar) -> bool {
        self.blocks
            .get(&label)
            .is_some_and(|b| b.live_out.contains(&var))
    }

    pub fn compute_live_intervals(&self, cfg: &ControlFlowGraph) -> HashMap<IrVar, LiveInterval> {
        let mut intervals: HashMap<IrVar, (usize, usize)> = HashMap::new();
        let mut pc = 0usize;

        for block in &cfg.blocks {
            // Variables live at block entry start at or before pc
            if let Some(info) = self.blocks.get(&block.label) {
                for &v in &info.live_in {
                    intervals
                        .entry(v)
                        .and_modify(|(_, end)| *end = (*end).max(pc))
                        .or_insert((pc, pc));
                }
            }

            for inst in &block.instructions {
                for u in inst.use_vars() {
                    intervals
                        .entry(u)
                        .and_modify(|(_, end)| *end = (*end).max(pc))
                        .or_insert((pc, pc));
                }

                for d in inst.def_vars() {
                    intervals
                        .entry(d)
                        .and_modify(|(_, end)| *end = (*end).max(pc))
                        .or_insert((pc, pc));
                }

                pc += 1;
            }

            // Variables live out of the block extend to the end of the block
            if let Some(info) = self.blocks.get(&block.label) {
                for &v in &info.live_out {
                    intervals
                        .entry(v)
                        .and_modify(|(_, end)| *end = (*end).max(pc))
                        .or_insert((pc, pc));
                }
            }
        }

        intervals
            .into_iter()
            .map(|(var, (start, end))| (var, LiveInterval { var, start, end }))
            .collect()
    }
}
