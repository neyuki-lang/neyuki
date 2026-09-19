// IR Optimization module coordinating all mid-level passes.

pub mod const_fold;
pub mod copy_prop;
pub mod cse;
pub mod dce;
pub mod inline;
pub mod loop_invariant;
pub mod simplify_cfg;

use crate::compiler::ir::block::ControlFlowGraph;
use crate::compiler::ir::dom::DominatorTree;

pub use const_fold::constant_propagation;
pub use copy_prop::{copy_propagation, copy_propagation_cfg};
pub use cse::{common_subexpression_elimination, common_subexpression_elimination_cfg};
pub use dce::{dead_code_elimination, dead_code_elimination_cfg};
pub use inline::inline_functions;
pub use loop_invariant::loop_invariant_code_motion;
pub use simplify_cfg::simplify_cfg;

/// Fixpoint optimization driver for CFG.
/// Iteratively executes optimization passes in sequence until the CFG converges
/// to a fixed point (no further changes are made), bounded by MAX_FIXPOINT_ROUNDS.
pub fn optimize_cfg(cfg: &mut ControlFlowGraph) {
    const MAX_FIXPOINT_ROUNDS: usize = 16;

    for _ in 0..MAX_FIXPOINT_ROUNDS {
        let mut changed = false;

        // 1. Dominator-based Loop Invariant Code Motion
        let dom = DominatorTree::build(cfg);
        if loop_invariant_code_motion(cfg, &dom) {
            changed = true;
        }

        // 2. Common Subexpression Elimination
        if common_subexpression_elimination_cfg(cfg) {
            changed = true;
        }

        // 3. Copy Propagation
        if copy_propagation_cfg(cfg) {
            changed = true;
        }

        // 4. Dead Code Elimination
        if dead_code_elimination_cfg(cfg) {
            changed = true;
        }

        // 5. CFG Simplification (pruning unreachable, collapsing jumps, merging blocks)
        if simplify_cfg(cfg) {
            changed = true;
        }

        if !changed {
            break;
        }
    }
}
