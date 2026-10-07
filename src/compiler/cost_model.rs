use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::Proto;
use crate::compiler::ir::block::IrFunction;
use crate::compiler::ir::inst::IrInst;
use crate::parser::{Expr, Stmt};

#[derive(Clone, Debug, PartialEq)]
pub struct CostModel {
    pub const_load_cost: u32,
    pub register_move_cost: u32,
    pub arithmetic_cost: u32,
    pub bitwise_cost: u32,
    pub table_access_cost: u32,
    pub branch_cost: u32,
    pub call_cost: u32,
    pub closure_alloc_cost: u32,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            const_load_cost: 1,
            register_move_cost: 1,
            arithmetic_cost: 2,
            bitwise_cost: 2,
            table_access_cost: 4,
            branch_cost: 3,
            call_cost: 8,
            closure_alloc_cost: 12,
        }
    }
}

impl CostModel {
    // Calculates estimated execution cost of an Instruction
    pub fn instruction_cost(&self, inst: &Instruction) -> u32 {
        match inst {
            Instruction::LoadNil { .. }
            | Instruction::LoadBool { .. }
            | Instruction::LoadInt { .. }
            | Instruction::LoadK { .. } => self.const_load_cost,

            Instruction::Move { .. } => self.register_move_cost,

            Instruction::Add { .. }
            | Instruction::Sub { .. }
            | Instruction::Mul { .. }
            | Instruction::Div { .. }
            | Instruction::IDiv { .. }
            | Instruction::Mod { .. }
            | Instruction::Pow { .. }
            | Instruction::Unm { .. } => self.arithmetic_cost,

            Instruction::BitAnd { .. }
            | Instruction::BitOr { .. }
            | Instruction::BitXor { .. }
            | Instruction::Shl { .. }
            | Instruction::Shr { .. }
            | Instruction::LShl { .. }
            | Instruction::LShr { .. }
            | Instruction::BitNot { .. } => self.bitwise_cost,

            Instruction::NewTable { .. } => self.closure_alloc_cost,
            Instruction::GetTable { .. }
            | Instruction::SetTable { .. }
            | Instruction::GetTableK { .. }
            | Instruction::SetTableK { .. }
            | Instruction::AppendArray { .. }
            | Instruction::SetList { .. } => self.table_access_cost,

            Instruction::Eq { .. }
            | Instruction::Ne { .. }
            | Instruction::Lt { .. }
            | Instruction::Le { .. }
            | Instruction::Gt { .. }
            | Instruction::Ge { .. }
            | Instruction::Test { .. }
            | Instruction::Jump { .. }
            | Instruction::ForPrep { .. }
            | Instruction::ForLoop { .. }
            | Instruction::TForCall { .. }
            | Instruction::TForLoop { .. } => self.branch_cost,

            Instruction::Call { .. } => self.call_cost,
            Instruction::Return { .. } => self.branch_cost,
            Instruction::Closure { .. } => self.closure_alloc_cost,
            Instruction::Vararg { .. } => self.register_move_cost,

            Instruction::GetGlobal { .. } | Instruction::SetGlobal { .. } => self.table_access_cost,
            Instruction::GetImport { .. } => self.table_access_cost,
            Instruction::GetUpval { .. } | Instruction::SetUpval { .. } => self.register_move_cost,
            Instruction::Concat { .. }
            | Instruction::Not { .. }
            | Instruction::Len { .. }
            | Instruction::Coalesce { .. } => self.arithmetic_cost,
        }
    }

    // Calculates total cost of a compiled bytecode Prototype
    pub fn proto_cost(&self, proto: &Proto) -> u32 {
        let mut total = 0u32;
        for inst in &proto.instructions {
            total = total.saturating_add(self.instruction_cost(inst));
        }
        for sub in &proto.protos {
            total = total.saturating_add(self.proto_cost(sub));
        }
        total
    }

    // Evaluates whether a function proto is small enough to be profitable for inlining
    pub fn should_inline(&self, proto: &Proto) -> bool {
        // Small leaf functions with few instructions and no heavy allocations
        const INLINE_COST_THRESHOLD: u32 = 40;
        const MAX_INLINE_REGISTERS: u8 = 8;

        if !proto.protos.is_empty() || proto.is_vararg {
            return false;
        }

        if proto.max_registers > MAX_INLINE_REGISTERS {
            return false;
        }

        self.proto_cost(proto) <= INLINE_COST_THRESHOLD
    }

    // Calculates execution cost of an IR instruction
    pub fn ir_inst_cost(&self, inst: &IrInst) -> u32 {
        match inst {
            IrInst::Move { .. } => self.register_move_cost,
            IrInst::BinOp { .. } | IrInst::UnOp { .. } => self.arithmetic_cost,
            IrInst::LoadConst { .. } | IrInst::LoadNil { .. } => self.const_load_cost,
            IrInst::NewTable { .. } => self.closure_alloc_cost,
            IrInst::GetTable { .. } | IrInst::SetTable { .. } | IrInst::GetImport { .. } => {
                self.table_access_cost
            }
            IrInst::GetGlobal { .. } | IrInst::SetGlobal { .. } => self.table_access_cost,
            _ => 2,
        }
    }

    // Calculates total estimated cost of an IR function
    pub fn ir_function_cost(&self, func: &IrFunction) -> u32 {
        let mut total = 0u32;
        for inst in &func.instructions {
            total = total.saturating_add(self.ir_inst_cost(inst));
        }
        total
    }

    // Evaluates whether an IR function is small and profitable to inline
    pub fn should_inline_ir(&self, func: &IrFunction) -> bool {
        const MAX_INLINE_COST: u32 = 40;
        const MAX_INLINE_INSTRUCTIONS: usize = 30;

        if func.is_vararg || !func.protos.is_empty() {
            return false;
        }
        if func.instructions.len() > MAX_INLINE_INSTRUCTIONS {
            return false;
        }
        self.ir_function_cost(func) <= MAX_INLINE_COST
    }

    // Calculates total cost of an entire AST statement list
    pub fn program_cost(&self, stmts: &[Stmt]) -> u32 {
        let mut total = 0u32;
        for s in stmts {
            total = total.saturating_add(self.stmt_cost(s));
        }
        total
    }

    // Calculates rough complexity score for an AST statement tree
    pub fn stmt_cost(&self, stmt: &Stmt) -> u32 {
        match stmt {
            Stmt::Local { initializer, .. } => {
                initializer.as_ref().map_or(1, |e| self.expr_cost(e) + 1)
            }
            Stmt::Assign { value, .. } => self.expr_cost(value) + 1,
            Stmt::AssignMany {
                targets, values, ..
            } => (targets.len() + values.len()) as u32 * 2,
            Stmt::Increment { .. } => 2,
            Stmt::Expr { expr: e, .. } => self.expr_cost(e),
            Stmt::If {
                condition,
                then_branch,
                else_if_branches,
                else_branch,
                ..
            } => {
                let mut cost = self.expr_cost(condition) + self.branch_cost;
                for s in then_branch {
                    cost = cost.saturating_add(self.stmt_cost(s));
                }
                for (cond, branch) in else_if_branches {
                    cost = cost.saturating_add(self.expr_cost(cond) + self.branch_cost);
                    for s in branch {
                        cost = cost.saturating_add(self.stmt_cost(s));
                    }
                }
                if let Some(eb) = else_branch {
                    for s in eb {
                        cost = cost.saturating_add(self.stmt_cost(s));
                    }
                }
                cost
            }
            Stmt::While {
                condition, body, ..
            } => {
                let mut cost = self.expr_cost(condition) + self.branch_cost * 2;
                for s in body {
                    cost = cost.saturating_add(self.stmt_cost(s));
                }
                cost * 3 // Weighted higher for loop bodies
            }
            Stmt::Repeat {
                body, condition, ..
            } => {
                let mut cost = self.expr_cost(condition) + self.branch_cost;
                for s in body {
                    cost = cost.saturating_add(self.stmt_cost(s));
                }
                cost * 3
            }
            Stmt::NumericFor { body, .. } | Stmt::For { body, .. } => {
                let mut cost = self.branch_cost * 2;
                for s in body {
                    cost = cost.saturating_add(self.stmt_cost(s));
                }
                cost * 4
            }
            Stmt::Function { body, .. } => {
                let mut cost = self.closure_alloc_cost;
                for s in body {
                    cost = cost.saturating_add(self.stmt_cost(s));
                }
                cost
            }
            Stmt::Return { values: exprs, .. } => {
                let mut cost = self.branch_cost;
                for e in exprs {
                    cost = cost.saturating_add(self.expr_cost(e));
                }
                cost
            }
            Stmt::Break { .. } | Stmt::Continue { .. } => self.branch_cost,
            Stmt::Goto { .. } => self.branch_cost,
            Stmt::Label { .. } => 0,
            Stmt::LocalMany { initializers, .. } => {
                let mut cost = 2u32;
                for e in initializers {
                    cost = cost.saturating_add(self.expr_cost(e));
                }
                cost
            }
        }
    }

    pub fn expr_cost(&self, expr: &Expr) -> u32 {
        match expr {
            Expr::Literal { .. } | Expr::Variable { .. } | Expr::Vararg { .. } => {
                self.const_load_cost
            }
            Expr::Binary { left, right, .. } => {
                self.expr_cost(left) + self.expr_cost(right) + self.arithmetic_cost
            }
            Expr::Unary { expr, .. } => self.expr_cost(expr) + self.arithmetic_cost,
            Expr::Call { callee, args, .. } => {
                let mut cost = self.expr_cost(callee) + self.call_cost;
                for arg in args {
                    cost = cost.saturating_add(self.expr_cost(arg));
                }
                cost
            }
            Expr::MethodCall { object, args, .. } => {
                let mut cost = self.expr_cost(object) + self.call_cost + self.table_access_cost;
                for arg in args {
                    cost = cost.saturating_add(self.expr_cost(arg));
                }
                cost
            }
            Expr::Member { object, .. } => self.expr_cost(object) + self.table_access_cost,
            Expr::Index { object, index, .. } => {
                self.expr_cost(object) + self.expr_cost(index) + self.table_access_cost
            }
            Expr::Table { entries, .. } => {
                let mut cost = self.closure_alloc_cost;
                for entry in entries {
                    cost = cost.saturating_add(self.expr_cost(&entry.value));
                }
                cost
            }
            Expr::Function { body, .. } => {
                let mut cost = self.closure_alloc_cost;
                for s in body {
                    cost = cost.saturating_add(self.stmt_cost(s));
                }
                cost
            }
            Expr::Interp { parts, .. } => parts.len() as u32 * 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instruction_cost_model() {
        let model = CostModel::default();
        let load_cost = model.instruction_cost(&Instruction::LoadInt { dst: 0, val: 10 });
        let add_cost = model.instruction_cost(&Instruction::Add { dst: 0, a: 1, b: 2 });
        let call_cost = model.instruction_cost(&Instruction::Call {
            callee: 0,
            argc: 1,
            retc: 1,
        });

        assert_eq!(load_cost, 1);
        assert_eq!(add_cost, 2);
        assert_eq!(call_cost, 8);
    }

    #[test]
    fn test_inline_heuristics() {
        let model = CostModel::default();
        let mut small_proto = Proto::new(Some("add_one".to_string()), 1, false);
        small_proto.max_registers = 2;
        small_proto
            .instructions
            .push(Instruction::LoadInt { dst: 1, val: 1 });
        small_proto
            .instructions
            .push(Instruction::Add { dst: 0, a: 0, b: 1 });
        small_proto
            .instructions
            .push(Instruction::Return { base: 0, count: 1 });

        assert!(model.should_inline(&small_proto));

        // Heavy proto with closure and table creation should not inline
        let mut heavy_proto = Proto::new(Some("heavy".to_string()), 0, false);
        heavy_proto.max_registers = 16;
        for _ in 0..30 {
            heavy_proto
                .instructions
                .push(Instruction::NewTable { dst: 0 });
        }
        assert!(!model.should_inline(&heavy_proto));
    }
}
