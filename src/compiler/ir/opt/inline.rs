// Function inlining pass for IR.
// Inlines small, straight-line leaf functions to eliminate call overhead.
//
// Soundness rules (all enforced below; anything else stays a call):
// - the body holds only frame-independent ops plus one trailing Return of
//   exactly one value: no calls, loops, branches, labels, varargs,
//   upvalues, spreads, or nested functions
// - the call site passes exactly `num_params` arguments and takes exactly
//   one value with a fixed `retc` (never MULTRET, which flows through the
//   stack top and cannot be spliced)
// - the callee resolves to a single Closure definition: directly, through
//   single-definition Move aliases, or through a single SetGlobal/GetGlobal
//   pair for one name

use std::collections::{HashMap, HashSet};

use crate::compiler::ir::block::{IrFunction, IrModule};
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::IrVar;

const MAX_INLINE_INSTRUCTIONS: usize = 16;

/// Frame-independent ops: relocating them into the caller preserves meaning.
/// Anything else (calls, control flow, labels, upvalues, varargs, spreads,
/// nested functions) rejects the candidate.
fn is_inlinable_body_inst(inst: &IrInst) -> bool {
    matches!(
        inst,
        IrInst::Move { .. }
            | IrInst::BinOp { .. }
            | IrInst::UnOp { .. }
            | IrInst::LoadConst { .. }
            | IrInst::LoadNil { .. }
            | IrInst::NewTable { .. }
            | IrInst::GetTable { .. }
            | IrInst::SetTable { .. }
            | IrInst::GetGlobal { .. }
            | IrInst::SetGlobal { .. }
            | IrInst::AppendArray { .. }
    )
}

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

    // Per-variable definition sites in main: needed to follow aliases and
    // to require single definitions before trusting them.
    let mut def_count: HashMap<IrVar, usize> = HashMap::new();
    let mut move_alias: HashMap<IrVar, IrVar> = HashMap::new();
    let mut closure_map: HashMap<IrVar, usize> = HashMap::new();
    let mut getglobal_name: HashMap<IrVar, String> = HashMap::new();
    let mut setglobal_srcs: HashMap<String, Vec<IrVar>> = HashMap::new();
    for inst in &module.main.instructions {
        for d in inst.def_vars() {
            *def_count.entry(d).or_insert(0) += 1;
        }
        match inst {
            IrInst::Closure { dst, proto_idx, .. } => {
                closure_map.insert(*dst, *proto_idx as usize);
            }
            IrInst::Move { dst, src } => {
                move_alias.insert(*dst, *src);
            }
            IrInst::GetGlobal { dst, name } => {
                getglobal_name.insert(*dst, name.clone());
            }
            IrInst::SetGlobal { name, src } => {
                setglobal_srcs.entry(name.clone()).or_default().push(*src);
            }
            _ => {}
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
        let inlined = match &inst {
            IrInst::Call {
                dsts,
                callee,
                args,
                retc,
            } if dsts.len() == 1 && *retc == 1 => resolve_callee(
                *callee,
                &closure_map,
                &move_alias,
                &getglobal_name,
                &setglobal_srcs,
                &def_count,
            )
            .and_then(|idx| candidates.get(&idx))
            .filter(|f| f.num_params as usize == args.len())
            .map(|target| (target, dsts[0].clone(), args.clone())),
            _ => None,
        };

        match inlined {
            Some((target_func, out_dst, call_args)) => {
                if splice_call(
                    &mut new_instructions,
                    target_func,
                    out_dst,
                    &call_args,
                    &mut max_var_id,
                ) {
                    changed = true;
                } else {
                    // Remap hit something unexpected: keep the call.
                    new_instructions.push(inst);
                }
            }
            None => {
                new_instructions.push(inst);
            }
        }
    }

    module.main.instructions = new_instructions;
    changed
}

/// Follows a variable to the Closure definition it must hold at runtime:
/// directly, through single-definition Move aliases, or through a
/// GetGlobal fed by a single SetGlobal of the same name. `visited` guards
/// against alias cycles. Returns the child proto index on success.
fn resolve_callee(
    mut var: IrVar,
    closure_map: &HashMap<IrVar, usize>,
    move_alias: &HashMap<IrVar, IrVar>,
    getglobal_name: &HashMap<IrVar, String>,
    setglobal_srcs: &HashMap<String, Vec<IrVar>>,
    def_count: &HashMap<IrVar, usize>,
) -> Option<usize> {
    let mut visited: HashSet<IrVar> = HashSet::new();
    loop {
        if let Some(&proto_idx) = closure_map.get(&var) {
            return Some(proto_idx);
        }
        if !visited.insert(var) {
            return None;
        }
        // Single-definition Move alias.
        if def_count.get(&var).copied().unwrap_or(0) == 1
            && let Some(&src) = move_alias.get(&var)
        {
            var = src;
            continue;
        }
        // GetGlobal fed by exactly one SetGlobal of the same name.
        if def_count.get(&var).copied().unwrap_or(0) == 1
            && let Some(name) = getglobal_name.get(&var)
            && let Some(srcs) = setglobal_srcs.get(name)
            && let [src] = srcs.as_slice()
        {
            var = *src;
            continue;
        }
        return None;
    }
}

/// Splices one inlined call into `out`: parameters map to the call's
/// arguments, the single returned value moves to `out_dst`. Returns false
/// if any body instruction cannot be remapped, in which case the caller
/// keeps the original call and `out` is left with a partial splice that
/// must be discarded.
fn splice_call(
    out: &mut Vec<IrInst>,
    target_func: &IrFunction,
    out_dst: IrVar,
    call_args: &[IrVar],
    max_var_id: &mut u32,
) -> bool {
    let checkpoint = out.len();
    let mut var_map: HashMap<IrVar, IrVar> = HashMap::new();
    for (i, &arg) in call_args.iter().enumerate() {
        var_map.insert(IrVar(i as u32), arg);
    }
    for body_inst in &target_func.instructions {
        if let IrInst::Return(ret_vars) = body_inst {
            match ret_vars.first() {
                Some(&ret_v) => {
                    let resolved_ret = var_map.get(&ret_v).copied().unwrap_or(ret_v);
                    out.push(IrInst::Move {
                        dst: out_dst,
                        src: resolved_ret,
                    });
                }
                None => {
                    out.push(IrInst::LoadNil { dst: out_dst });
                }
            }
        } else {
            let mut cloned = body_inst.clone();
            if !remap_inline_vars(&mut cloned, &mut var_map, max_var_id) {
                out.truncate(checkpoint);
                return false;
            }
            out.push(cloned);
        }
    }
    true
}

fn is_inline_candidate(func: &IrFunction) -> bool {
    if func.is_vararg || !func.protos.is_empty() {
        return false;
    }
    if func.instructions.len() > MAX_INLINE_INSTRUCTIONS {
        return false;
    }
    // Straight-line frame-independent body plus one trailing Return of
    // exactly one value. Anything else is rejected.
    let mut returns = 0;
    for (i, inst) in func.instructions.iter().enumerate() {
        match inst {
            IrInst::Return(vars) => {
                if vars.len() != 1 || i + 1 != func.instructions.len() {
                    return false;
                }
                returns += 1;
            }
            _ if is_inlinable_body_inst(inst) => {}
            _ => return false,
        }
    }
    returns == 1
}

/// Maps a use to its replacement; bails on uses that are neither parameters
/// nor remapped definitions (the body would not be self-contained).
fn use_mapped(v: &mut IrVar, var_map: &HashMap<IrVar, IrVar>) -> bool {
    match var_map.get(v).copied() {
        Some(mapped) => {
            *v = mapped;
            true
        }
        None => false,
    }
}

/// Mints a fresh variable for a definition so spliced code cannot collide
/// with caller variables.
fn fresh_def(v: IrVar, var_map: &mut HashMap<IrVar, IrVar>, max_id: &mut u32) -> IrVar {
    *max_id += 1;
    let fresh = IrVar(*max_id);
    var_map.insert(v, fresh);
    fresh
}

fn remap_inline_vars(
    inst: &mut IrInst,
    var_map: &mut HashMap<IrVar, IrVar>,
    max_id: &mut u32,
) -> bool {
    match inst {
        IrInst::Move { dst, src } => {
            if !use_mapped(src, var_map) {
                return false;
            }
            *dst = fresh_def(*dst, var_map, max_id);
        }
        IrInst::BinOp { dst, lhs, rhs, .. } => {
            if !use_mapped(lhs, var_map) || !use_mapped(rhs, var_map) {
                return false;
            }
            *dst = fresh_def(*dst, var_map, max_id);
        }
        IrInst::UnOp { dst, src, .. } => {
            if !use_mapped(src, var_map) {
                return false;
            }
            *dst = fresh_def(*dst, var_map, max_id);
        }
        IrInst::LoadConst { dst, .. } | IrInst::LoadNil { dst } => {
            *dst = fresh_def(*dst, var_map, max_id);
        }
        IrInst::NewTable { dst } => {
            *dst = fresh_def(*dst, var_map, max_id);
        }
        IrInst::GetTable { dst, table, key } => {
            if !use_mapped(table, var_map) || !use_mapped(key, var_map) {
                return false;
            }
            *dst = fresh_def(*dst, var_map, max_id);
        }
        IrInst::SetTable { table, key, val } => {
            if !use_mapped(table, var_map) || !use_mapped(key, var_map) || !use_mapped(val, var_map)
            {
                return false;
            }
        }
        IrInst::GetGlobal { dst, .. } => {
            *dst = fresh_def(*dst, var_map, max_id);
        }
        IrInst::SetGlobal { src, .. } => {
            if !use_mapped(src, var_map) {
                return false;
            }
        }
        IrInst::AppendArray { table, src } => {
            if !use_mapped(table, var_map) || !use_mapped(src, var_map) {
                return false;
            }
        }
        _ => return false,
    }
    true
}
