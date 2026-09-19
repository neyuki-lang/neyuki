// Dominator Tree and Dominance Frontier analysis for IR CFG.

use std::collections::{HashMap, HashSet};

use crate::compiler::ir::block::ControlFlowGraph;
use crate::compiler::ir::types::IrLabel;

#[derive(Debug, Clone)]
pub struct DominatorTree {
    pub idom: HashMap<IrLabel, IrLabel>,
    pub children: HashMap<IrLabel, Vec<IrLabel>>,
    pub frontiers: HashMap<IrLabel, HashSet<IrLabel>>,
}

impl DominatorTree {
    pub fn build(cfg: &ControlFlowGraph) -> Self {
        let entry = cfg.entry_label;
        let mut idom: HashMap<IrLabel, IrLabel> = HashMap::new();
        let mut label_to_idx: HashMap<IrLabel, usize> = HashMap::new();

        for (idx, b) in cfg.blocks.iter().enumerate() {
            label_to_idx.insert(b.label, idx);
        }

        idom.insert(entry, entry);

        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 1000;

        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            iterations += 1;

            for block in &cfg.blocks {
                let b = block.label;
                if b == entry {
                    continue;
                }

                // Find first processed predecessor
                let mut processed_pred = None;
                for p in &block.predecessors {
                    if idom.contains_key(p) {
                        processed_pred = Some(*p);
                        break;
                    }
                }

                let Some(mut new_idom) = processed_pred else {
                    continue;
                };

                for p in &block.predecessors {
                    let pred = *p;
                    if pred != new_idom && idom.contains_key(&pred) {
                        new_idom = intersect(pred, new_idom, &idom, &label_to_idx);
                    }
                }

                if idom.get(&b) != Some(&new_idom) {
                    idom.insert(b, new_idom);
                    changed = true;
                }
            }
        }

        // Build tree children
        let mut children: HashMap<IrLabel, Vec<IrLabel>> = HashMap::new();
        for (&node, &parent) in &idom {
            if node != parent {
                children.entry(parent).or_default().push(node);
            }
        }

        // Build dominance frontiers
        let mut frontiers: HashMap<IrLabel, HashSet<IrLabel>> = HashMap::new();
        for b in &cfg.blocks {
            frontiers.entry(b.label).or_default();
        }

        for block in &cfg.blocks {
            let b = block.label;
            if block.predecessors.len() >= 2 {
                for p in &block.predecessors {
                    let mut runner = *p;
                    let target_idom = idom.get(&b).copied().unwrap_or(b);

                    let mut steps = 0;
                    while runner != target_idom && steps < 500 {
                        frontiers.entry(runner).or_default().insert(b);
                        if let Some(&next_runner) = idom.get(&runner) {
                            if next_runner == runner {
                                break;
                            }
                            runner = next_runner;
                        } else {
                            break;
                        }
                        steps += 1;
                    }
                }
            }
        }

        Self {
            idom,
            children,
            frontiers,
        }
    }

    pub fn dominates(&self, a: IrLabel, mut b: IrLabel) -> bool {
        if a == b {
            return true;
        }
        let mut steps = 0;
        while let Some(&parent) = self.idom.get(&b) {
            if parent == a {
                return true;
            }
            if parent == b {
                break;
            }
            b = parent;
            steps += 1;
            if steps > 1000 {
                break;
            }
        }
        false
    }
}

fn intersect(
    mut b1: IrLabel,
    mut b2: IrLabel,
    idom: &HashMap<IrLabel, IrLabel>,
    label_to_idx: &HashMap<IrLabel, usize>,
) -> IrLabel {
    let mut steps = 0;
    while b1 != b2 && steps < 1000 {
        steps += 1;
        let mut idx1 = label_to_idx.get(&b1).copied().unwrap_or(0);
        let mut idx2 = label_to_idx.get(&b2).copied().unwrap_or(0);

        while idx1 > idx2 {
            if let Some(&p) = idom.get(&b1) {
                if p == b1 {
                    break;
                }
                b1 = p;
                idx1 = label_to_idx.get(&b1).copied().unwrap_or(0);
            } else {
                break;
            }
        }
        while idx2 > idx1 {
            if let Some(&p) = idom.get(&b2) {
                if p == b2 {
                    break;
                }
                b2 = p;
                idx2 = label_to_idx.get(&b2).copied().unwrap_or(0);
            } else {
                break;
            }
        }
    }
    b1
}
