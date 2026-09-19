// IR Instruction definitions for 3-Address Code and SSA.

use crate::compiler::ir::types::{IrBinaryOp, IrConstant, IrLabel, IrUnaryOp, IrVar};

#[derive(Clone, Debug, PartialEq)]
pub enum IrInst {
    LoadConst {
        dst: IrVar,
        val: IrConstant,
    },
    LoadNil {
        dst: IrVar,
    },
    Move {
        dst: IrVar,
        src: IrVar,
    },
    BinOp {
        dst: IrVar,
        op: IrBinaryOp,
        lhs: IrVar,
        rhs: IrVar,
    },
    UnOp {
        dst: IrVar,
        op: IrUnaryOp,
        src: IrVar,
    },
    NewTable {
        dst: IrVar,
    },
    GetTable {
        dst: IrVar,
        table: IrVar,
        key: IrVar,
    },
    SetTable {
        table: IrVar,
        key: IrVar,
        val: IrVar,
    },
    AppendArray {
        table: IrVar,
        src: IrVar,
    },
    GetGlobal {
        dst: IrVar,
        name: String,
    },
    SetGlobal {
        name: String,
        src: IrVar,
    },
    GetUpval {
        dst: IrVar,
        index: u8,
    },
    SetUpval {
        index: u8,
        src: IrVar,
    },
    Call {
        dst: Option<IrVar>,
        callee: IrVar,
        args: Vec<IrVar>,
        retc: u8,
    },
    Return(Vec<IrVar>),
    Closure {
        dst: IrVar,
        proto_idx: u16,
    },
    Vararg {
        dst: IrVar,
        count: u8,
    },
    ForPrep {
        base: IrVar,
        limit: IrVar,
        step: IrVar,
        loop_var: IrVar,
        jump: IrLabel,
    },
    ForLoop {
        base: IrVar,
        jump: IrLabel,
    },
    TForCall {
        base: IrVar,
        state: IrVar,
        ctrl: IrVar,
        vars: Vec<IrVar>,
    },
    TForLoop {
        base: IrVar,
        jump: IrLabel,
    },
    Jump(IrLabel),
    JumpIfFalse {
        cond: IrVar,
        target: IrLabel,
    },
    Label(IrLabel),
    // SSA Phi node: chooses incoming variable based on predecessor block label
    Phi {
        dst: IrVar,
        incoming: Vec<(IrLabel, IrVar)>,
    },
}

impl IrInst {
    pub fn def_var(&self) -> Option<IrVar> {
        match self {
            Self::LoadConst { dst, .. }
            | Self::LoadNil { dst }
            | Self::Move { dst, .. }
            | Self::BinOp { dst, .. }
            | Self::UnOp { dst, .. }
            | Self::NewTable { dst }
            | Self::GetTable { dst, .. }
            | Self::GetGlobal { dst, .. }
            | Self::GetUpval { dst, .. }
            | Self::Closure { dst, .. }
            | Self::Vararg { dst, .. }
            | Self::Phi { dst, .. } => Some(*dst),
            Self::Call { dst, .. } => *dst,
            _ => None,
        }
    }

    pub fn use_vars(&self) -> Vec<IrVar> {
        match self {
            Self::Move { src, .. } | Self::UnOp { src, .. } | Self::AppendArray { src, .. } => {
                vec![*src]
            }
            Self::BinOp { lhs, rhs, .. } => vec![*lhs, *rhs],
            Self::GetTable { table, key, .. } => vec![*table, *key],
            Self::SetTable { table, key, val } => vec![*table, *key, *val],
            Self::SetGlobal { src, .. } | Self::SetUpval { src, .. } => vec![*src],
            Self::Call { callee, args, .. } => {
                let mut uses = vec![*callee];
                uses.extend(args.iter().copied());
                uses
            }
            Self::Return(vars) => vars.clone(),
            Self::JumpIfFalse { cond, .. } => vec![*cond],
            Self::ForPrep {
                base, limit, step, ..
            } => vec![*base, *limit, *step],
            Self::ForLoop { base, .. } | Self::TForLoop { base, .. } => vec![*base],
            Self::TForCall {
                base, state, ctrl, ..
            } => vec![*base, *state, *ctrl],
            Self::Phi { incoming, .. } => incoming.iter().map(|(_, v)| *v).collect(),
            _ => Vec::new(),
        }
    }

    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            Self::Return(_)
                | Self::Jump(_)
                | Self::JumpIfFalse { .. }
                | Self::ForPrep { .. }
                | Self::ForLoop { .. }
                | Self::TForLoop { .. }
        )
    }

    pub fn jump_targets(&self) -> Vec<IrLabel> {
        match self {
            Self::Jump(lbl) => vec![*lbl],
            Self::JumpIfFalse { target, .. } => vec![*target],
            Self::ForPrep { jump, .. }
            | Self::ForLoop { jump, .. }
            | Self::TForLoop { jump, .. } => {
                vec![*jump]
            }
            _ => Vec::new(),
        }
    }
}
