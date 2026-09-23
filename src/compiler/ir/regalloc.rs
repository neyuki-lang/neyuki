// Linear scan CFG-aware register allocator.
// Maps unbounded virtual IrVars to physical VM registers (0..255) using live intervals.

use std::collections::{BTreeSet, HashMap};

use crate::compiler::ir::block::ControlFlowGraph;
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

    // Sort by start position, with the variable id breaking ties so the
    // allocation does not depend on hash order.
    intervals.sort_by_key(|intv| (intv.start, intv.end, intv.var.0));

    let mut mapping: HashMap<IrVar, u8> = HashMap::new();

    // 1. Pre-allocate parameter registers: 0..num_params
    for i in 0..num_params {
        mapping.insert(IrVar(i as u32), i);
    }

    // 2. Linear scan register allocation
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

        // Allocate the lowest available register
        if let Some(&reg) = free_registers.iter().next() {
            free_registers.remove(&reg);
            mapping.insert(interval.var, reg);
            active.push((interval.end, reg, interval.var));
            if reg.saturating_add(1) > max_reg {
                max_reg = reg.saturating_add(1);
            }
        } else {
            return Err(format!(
                "register allocation failure: register limit (255) exceeded for variable {:?}",
                interval.var
            ));
        }
    }

    Ok(RegisterAllocation {
        mapping,
        max_registers: max_reg,
    })
}
