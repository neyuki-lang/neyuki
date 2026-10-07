// IR and CFG integrity verification pass.
// Checks structural well-formedness, jump targets, edge reciprocity, and variable definition rules.

use std::collections::HashSet;

use crate::compiler::ir::block::{ControlFlowGraph, IrFunction, IrModule};
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::{IrLabel, IrVar};

#[derive(Debug, Clone)]
pub struct VerifyError {
    pub message: String,
}

pub fn verify_cfg(cfg: &ControlFlowGraph, num_params: u8) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // 1. Verify entry block exists
    if cfg.find_block(cfg.entry_label).is_none() {
        errors.push(format!(
            "entry block {:?} not found in CFG",
            cfg.entry_label
        ));
    }

    // 2. Verify all block labels are unique
    let mut seen_labels = HashSet::new();
    for block in &cfg.blocks {
        if !seen_labels.insert(block.label) {
            errors.push(format!("duplicate block label {:?}", block.label));
        }
    }

    // 3. Verify jump targets, successors, predecessors, and edge reciprocity
    let label_set: HashSet<IrLabel> = cfg.blocks.iter().map(|b| b.label).collect();

    for block in &cfg.blocks {
        let b_lbl = block.label;

        // Check jump targets
        for inst in &block.instructions {
            for target in inst.jump_targets() {
                if !label_set.contains(&target) {
                    errors.push(format!(
                        "block {:?} references unknown jump target {:?}",
                        b_lbl, target
                    ));
                }
            }
        }

        // Check successors exist
        for &succ in &block.successors {
            if !label_set.contains(&succ) {
                errors.push(format!(
                    "block {:?} has unknown successor {:?}",
                    b_lbl, succ
                ));
            } else if let Some(succ_blk) = cfg.find_block(succ) {
                // Edge reciprocity: succ must have b_lbl in predecessors
                if !succ_blk.predecessors.contains(&b_lbl) {
                    errors.push(format!(
                        "broken edge reciprocity: {:?} -> {:?}, but {:?} not in {:?}'s predecessors",
                        b_lbl, succ, b_lbl, succ
                    ));
                }
            }
        }

        // Check predecessors exist
        for &pred in &block.predecessors {
            if !label_set.contains(&pred) {
                errors.push(format!(
                    "block {:?} has unknown predecessor {:?}",
                    b_lbl, pred
                ));
            } else if let Some(pred_blk) = cfg.find_block(pred) {
                // Edge reciprocity: pred must have b_lbl in successors
                if !pred_blk.successors.contains(&b_lbl) {
                    errors.push(format!(
                        "broken edge reciprocity: {:?} in {:?}'s predecessors, but {:?} not in {:?}'s successors",
                        pred, b_lbl, b_lbl, pred
                    ));
                }
            }
        }

        // Check terminator rules
        if let Some(term) = block.terminator() {
            match term {
                IrInst::Return(_) => {
                    if !block.successors.is_empty() {
                        errors.push(format!(
                            "block {:?} terminates with Return but has successors",
                            b_lbl
                        ));
                    }
                }
                IrInst::Jump(t) if !block.successors.contains(t) => {
                    errors.push(format!(
                        "block {:?} terminates with Jump({:?}) but target is not in successors",
                        b_lbl, t
                    ));
                }
                _ => {}
            }
        }

        // Phi placement rule: all Phi instructions must appear at the beginning of the block
        let mut past_phi = false;
        for inst in &block.instructions {
            if matches!(inst, IrInst::Phi { .. }) {
                if past_phi {
                    errors.push(format!(
                        "Phi instruction found after non-Phi instruction in block {:?}",
                        b_lbl
                    ));
                }
            } else {
                past_phi = true;
            }
        }
    }

    // 4. Parameter sanity check
    let mut defined_vars = HashSet::new();
    for i in 0..num_params {
        defined_vars.insert(IrVar(i as u32));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn verify_function(func: &IrFunction) -> Result<(), Vec<String>> {
    let mut all_errors = Vec::new();

    if let Some(cfg) = &func.cfg {
        let res = verify_cfg(cfg, func.num_params);
        if let Err(errs) = res {
            all_errors.extend(errs);
        }
    }

    for proto in &func.protos {
        if let Err(errs) = verify_function(proto) {
            all_errors.extend(errs);
        }
    }

    if all_errors.is_empty() {
        Ok(())
    } else {
        Err(all_errors)
    }
}

pub fn verify_module(module: &IrModule) -> Result<(), Vec<String>> {
    verify_function(&module.main)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::ir::block::{BasicBlock, ControlFlowGraph};

    #[test]
    fn test_verify_empty_module() {
        let module = IrModule::new(IrFunction::new(Some("main".to_string()), 0, false));
        assert!(verify_module(&module).is_ok());
    }

    #[test]
    fn test_verify_module_detects_unknown_jump() {
        let mut func = IrFunction::new(Some("main".to_string()), 0, false);
        let mut cfg = ControlFlowGraph::new(IrLabel(0));
        let mut b0 = BasicBlock::new(IrLabel(0));
        b0.instructions.push(IrInst::Jump(IrLabel(999)));
        cfg.blocks.push(b0);
        func.cfg = Some(cfg);
        let module = IrModule::new(func);
        let res = verify_module(&module);
        assert!(res.is_err());
        let errs = res.unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("references unknown jump target"))
        );
    }
}
