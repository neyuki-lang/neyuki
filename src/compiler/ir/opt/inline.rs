// Function inlining pass for IR.
// Inlines small, non-recursive leaf functions to eliminate call overhead.

use std::collections::HashMap;

use crate::compiler::ir::block::{IrFunction, IrModule};
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::IrVar;

const MAX_INLINE_INSTRUCTIONS: usize = 16;

pub fn inline_functions(module: &mut IrModule) -> bool {
    let mut changed = false;

    // Collect candidate leaf functions from module protos
    let candidates: HashMap<usize, IrFunction> = module
        .main
        .protos
        .iter()
        .enumerate()
        .filter_map(|(idx, f)| {
            if is_inline_candidate(f) {
                Some((idx, f.clone()))
            } else {
                None
            }
        })
        .collect();

    if candidates.is_empty() {
        return false;
    }

    // Find closure definitions mapping variable -> proto_idx
    let mut closure_map: HashMap<IrVar, usize> = HashMap::new();
    for inst in &module.main.instructions {
        if let IrInst::Closure { dst, proto_idx, .. } = inst {
            closure_map.insert(*dst, *proto_idx as usize);
        }
    }

    // Find max variable ID to avoid collisions during inlining
    let mut max_var_id = 0u32;
    for inst in &module.main.instructions {
        if let Some(d) = inst.def_var() {
            max_var_id = max_var_id.max(d.0);
        }
        for u in inst.use_vars() {
            max_var_id = max_var_id.max(u.0);
        }
    }

    let mut new_instructions = Vec::new();

    for inst in module.main.instructions.drain(..) {
        let candidate_target = match &inst {
            IrInst::Call { callee, args, .. } => closure_map.get(callee).and_then(|&idx| {
                candidates
                    .get(&idx)
                    .filter(|f| f.num_params as usize == args.len())
            }),
            _ => None,
        };

        match (candidate_target, inst) {
            (Some(target_func), IrInst::Call { dsts, args, .. }) => {
                let mut var_map: HashMap<IrVar, IrVar> = HashMap::new();
                for (i, &arg) in args.iter().enumerate() {
                    var_map.insert(IrVar(i as u32), arg);
                }
                for body_inst in &target_func.instructions {
                    if let IrInst::Return(ret_vars) = body_inst {
                        if let Some(&out_dst) = dsts.first() {
                            match ret_vars.first() {
                                Some(&ret_v) => {
                                    let resolved_ret =
                                        var_map.get(&ret_v).copied().unwrap_or(ret_v);
                                    new_instructions.push(IrInst::Move {
                                        dst: out_dst,
                                        src: resolved_ret,
                                    });
                                }
                                None => {
                                    new_instructions.push(IrInst::LoadNil { dst: out_dst });
                                }
                            }
                        }
                    } else {
                        let mut cloned = body_inst.clone();
                        remap_inline_vars(&mut cloned, &mut var_map, &mut max_var_id);
                        new_instructions.push(cloned);
                    }
                }
                changed = true;
            }
            (_, inst) => {
                new_instructions.push(inst);
            }
        }
    }

    module.main.instructions = new_instructions;
    changed
}

fn is_inline_candidate(func: &IrFunction) -> bool {
    if func.is_vararg || !func.protos.is_empty() {
        return false;
    }
    if func.instructions.len() > MAX_INLINE_INSTRUCTIONS {
        return false;
    }
    // Must not contain loops or calls
    !func.instructions.iter().any(|i| {
        matches!(
            i,
            IrInst::Call { .. }
                | IrInst::ForLoop { .. }
                | IrInst::TForLoop { .. }
                | IrInst::Closure { .. }
        )
    })
}

fn remap_inline_vars(inst: &mut IrInst, var_map: &mut HashMap<IrVar, IrVar>, max_id: &mut u32) {
    let mut get_or_create = |v: IrVar, is_def: bool| -> IrVar {
        if is_def {
            *max_id += 1;
            let fresh = IrVar(*max_id);
            var_map.insert(v, fresh);
            fresh
        } else {
            var_map.get(&v).copied().unwrap_or_else(|| {
                *max_id += 1;
                let fresh = IrVar(*max_id);
                var_map.insert(v, fresh);
                fresh
            })
        }
    };

    match inst {
        IrInst::Move { dst, src } => {
            *src = get_or_create(*src, false);
            *dst = get_or_create(*dst, true);
        }
        IrInst::BinOp { dst, lhs, rhs, .. } => {
            *lhs = get_or_create(*lhs, false);
            *rhs = get_or_create(*rhs, false);
            *dst = get_or_create(*dst, true);
        }
        IrInst::UnOp { dst, src, .. } => {
            *src = get_or_create(*src, false);
            *dst = get_or_create(*dst, true);
        }
        IrInst::LoadConst { dst, .. } | IrInst::LoadNil { dst } => {
            *dst = get_or_create(*dst, true);
        }
        IrInst::NewTable { dst } => {
            *dst = get_or_create(*dst, true);
        }
        _ => {}
    }
}
