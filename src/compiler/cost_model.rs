//! Rough instruction costs, used to decide which functions are worth inlining.

use crate::compiler::ir::block::IrFunction;
use crate::compiler::ir::inst::IrInst;

#[derive(Clone, Debug, PartialEq)]
pub struct CostModel {
    pub const_load_cost: u32,
    pub register_move_cost: u32,
    pub arithmetic_cost: u32,
    pub table_access_cost: u32,
    pub alloc_cost: u32,
    pub other_cost: u32,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            const_load_cost: 1,
            register_move_cost: 1,
            arithmetic_cost: 2,
            table_access_cost: 4,
            alloc_cost: 12,
            other_cost: 2,
        }
    }
}

impl CostModel {
    pub fn ir_inst_cost(&self, inst: &IrInst) -> u32 {
        match inst {
            IrInst::Move { .. } => self.register_move_cost,
            IrInst::BinOp { .. } | IrInst::UnOp { .. } => self.arithmetic_cost,
            IrInst::LoadConst { .. } | IrInst::LoadNil { .. } => self.const_load_cost,
            IrInst::NewTable { .. } => self.alloc_cost,
            IrInst::GetTable { .. }
            | IrInst::SetTable { .. }
            | IrInst::GetImport { .. }
            | IrInst::GetGlobal { .. }
            | IrInst::SetGlobal { .. } => self.table_access_cost,
            inst if inst.is_debug_marker() => 0,
            _ => self.other_cost,
        }
    }

    pub fn ir_function_cost(&self, func: &IrFunction) -> u32 {
        func.instructions.iter().fold(0u32, |total, inst| {
            total.saturating_add(self.ir_inst_cost(inst))
        })
    }

    /// Small leaf functions without varargs or nested functions.
    pub fn should_inline_ir(&self, func: &IrFunction) -> bool {
        const MAX_INLINE_COST: u32 = 45;
        const MAX_INLINE_INSTRUCTIONS: usize = 32;

        if func.is_vararg || !func.protos.is_empty() {
            return false;
        }
        let size = func
            .instructions
            .iter()
            .filter(|inst| !inst.is_debug_marker())
            .count();
        if size > MAX_INLINE_INSTRUCTIONS {
            return false;
        }
        self.ir_function_cost(func) <= MAX_INLINE_COST
    }
}
