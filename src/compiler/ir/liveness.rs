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
                if let Some(d) = inst.def_var() {
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
        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 1000;

        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            iterations += 1;

            for block in cfg.blocks.iter().rev() {
                let lbl = block.label;

                // Compute out[B]
                let mut new_out = HashSet::new();
                for succ in &block.successors {
                    if let Some(succ_info) = blocks.get(succ) {
                        for v in &succ_info.live_in {
                            new_out.insert(*v);
                        }
                    }
                }

                // Compute in[B] = use[B] Union (out[B] \ def[B])
                let (use_vars, def_vars) = {
                    let b = &blocks[&lbl];
                    (b.use_vars.clone(), b.def_vars.clone())
                };

                let mut new_in = use_vars;
                for v in &new_out {
                    if !def_vars.contains(v) {
                        new_in.insert(*v);
                    }
                }

                let cur = blocks.get_mut(&lbl).expect("block exists");
                if cur.live_out != new_out || cur.live_in != new_in {
                    cur.live_out = new_out;
                    cur.live_in = new_in;
                    changed = true;
                }
            }
        }

        Self { blocks }
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

                if let Some(d) = inst.def_var() {
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
