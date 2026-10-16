//! Linear scan register allocation from IR variables to the 256 VM
//! registers.

use std::collections::{BTreeSet, HashMap};

use crate::compiler::ir::block::ControlFlowGraph;
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::liveness::LivenessInfo;
use crate::compiler::ir::types::IrVar;

#[derive(Debug, Clone)]
pub struct RegisterAllocation {
    pub mapping: HashMap<IrVar, u8>,
    pub max_registers: u8,
}

impl RegisterAllocation {
    pub fn get(&self, var: IrVar) -> Option<u8> {
        self.mapping.get(&var).copied()
    }
}

pub fn allocate_registers(
    cfg: &ControlFlowGraph,
    num_params: u8,
) -> Result<RegisterAllocation, String> {
    let liveness = LivenessInfo::compute(cfg);
    let intervals_map = liveness.compute_live_intervals(cfg);

    let mut intervals: Vec<_> = intervals_map.into_values().collect();

    // A closure holds the register of each variable it captures, so that
    // register must not be handed to anything else once the variable's last
    // read goes by: the closure would see whatever overwrote it.
    let captured = crate::compiler::ir::opt::captured_vars(cfg);
    for interval in &mut intervals {
        if captured.contains(&interval.var) {
            interval.end = usize::MAX;
        }
    }

    // Collect copy hints for register coalescing
    let mut copy_hints: HashMap<IrVar, Vec<IrVar>> = HashMap::new();
    for block in &cfg.blocks {
        for inst in &block.instructions {
            match inst {
                IrInst::Move { dst, src } if dst != src => {
                    copy_hints.entry(*dst).or_default().push(*src);
                    copy_hints.entry(*src).or_default().push(*dst);
                }
                IrInst::Phi { dst, incoming } => {
                    for (_, src) in incoming {
                        if dst != src {
                            copy_hints.entry(*dst).or_default().push(*src);
                            copy_hints.entry(*src).or_default().push(*dst);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Sort by start position, with the variable id breaking ties so the
    // allocation does not depend on hash order.
    intervals.sort_by_key(|intv| (intv.start, intv.end, intv.var.0));

    let mut mapping: HashMap<IrVar, u8> = HashMap::new();

    // 1. Pre-allocate parameter registers: 0..num_params
    for i in 0..num_params {
        mapping.insert(IrVar(i as u32), i);
    }

    // 2. Linear scan register allocation with coalescing
    let mut free_registers: BTreeSet<u8> = (num_params..255).collect();
    // Active intervals: (end_point, register, IrVar)
    let mut active: Vec<(usize, u8, IrVar)> = Vec::new();
    let mut max_reg = num_params;

    for interval in intervals {
        // Skip already mapped parameters
        if mapping.contains_key(&interval.var) {
            continue;
        }

        // Expire old intervals
        active.retain(|&(end, reg, _var)| {
            if end < interval.start {
                free_registers.insert(reg);
                false
            } else {
                true
            }
        });

        // Coalescing: if any copy partner of interval.var is in active and ends at interval.start,
        // it dies right at this instruction (the copy). We can retire it early to reuse its register.
        if let Some(hints) = copy_hints.get(&interval.var) {
            for &partner in hints {
                if let Some(pos) = active
                    .iter()
                    .position(|&(end, _, v)| v == partner && end <= interval.start)
                {
                    let (_, reg, _) = active.remove(pos);
                    free_registers.insert(reg);
                }
            }
        }

        // Try allocating a preferred register from copy hints first
        let preferred_reg = copy_hints.get(&interval.var).and_then(|hints| {
            hints.iter().find_map(|partner| {
                mapping
                    .get(partner)
                    .copied()
                    .filter(|&r| free_registers.contains(&r))
            })
        });

        let chosen_reg = if let Some(pref) = preferred_reg {
            pref
        } else if let Some(&lowest) = free_registers.iter().next() {
            lowest
        } else {
            return Err(format!(
                "register allocation failure: register limit (255) exceeded for variable {:?}",
                interval.var
            ));
        };

        free_registers.remove(&chosen_reg);
        mapping.insert(interval.var, chosen_reg);
        active.push((interval.end, chosen_reg, interval.var));
        if chosen_reg.saturating_add(1) > max_reg {
            max_reg = chosen_reg.saturating_add(1);
        }
    }

    Ok(RegisterAllocation {
        mapping,
        max_registers: max_reg,
    })
}
