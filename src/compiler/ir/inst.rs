// IR Instruction definitions for 3-Address Code and SSA.

use crate::compiler::ir::types::{IrBinaryOp, IrConstant, IrLabel, IrUnaryOp, IrVar};

/// Where a run of values goes once it has been produced. The register VM keeps
/// such a run on the stack rather than in named registers, so the producer and
/// the consumer are lowered together as one instruction.
#[derive(Clone, Debug, PartialEq)]
pub enum SpreadSink {
    /// Passed as the trailing arguments of another call.
    Call {
        dsts: Vec<IrVar>,
        callee: IrVar,
        fixed_args: Vec<IrVar>,
        retc: u8,
    },
    /// Appended to a table constructor's array part.
    List { table: IrVar },
    /// Returned from the enclosing function.
    Return,
}

/// What produces a run of values: a call keeping all its results, or `...`.
#[derive(Clone, Debug, PartialEq)]
pub enum SpreadSource {
    Call {
        callee: IrVar,
        args: Vec<IrVar>,
        /// A producer for this call's own trailing arguments, as in
        /// `f(g(h()))`, where each call passes on everything the next yields.
        trailing: Option<Box<SpreadSource>>,
    },
    Vararg,
}

impl SpreadSource {
    /// The variables read while producing the run.
    pub fn use_vars(&self) -> Vec<IrVar> {
        match self {
            Self::Call {
                callee,
                args,
                trailing,
            } => {
                let mut uses = vec![*callee];
                uses.extend(args.iter().copied());
                if let Some(inner) = trailing {
                    uses.extend(inner.use_vars());
                }
                uses
            }
            Self::Vararg => Vec::new(),
        }
    }

    /// How many consecutive registers laying this producer out needs.
    pub fn slots(&self) -> u8 {
        match self {
            Self::Call { args, trailing, .. } => {
                let nested = trailing.as_ref().map(|t| t.slots()).unwrap_or(0);
                1u8.saturating_add(args.len() as u8).saturating_add(nested)
            }
            Self::Vararg => 1,
        }
    }
}

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
    GetImport {
        dst: IrVar,
        module: String,
        field: String,
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
        /// Where the call's results go, one variable per result. A call used
        /// as a statement has none; `local a, b = f()` has two.
        dsts: Vec<IrVar>,
        callee: IrVar,
        args: Vec<IrVar>,
        retc: u8,
    },
    Return(Vec<IrVar>),
    Closure {
        dst: IrVar,
        proto_idx: u16,
        /// Parallel to the nested prototype's upvalues: the parent variable
        /// each one captures, or `None` when it comes from the parent's own
        /// upvalues. Listing them here keeps the optimizer from dropping or
        /// renaming a variable that only a nested function reads.
        captures: Vec<Option<IrVar>>,
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
    /// Every value from `source` handed straight to `sink`, e.g. `f(g())`,
    /// `{...}` or `return f()`.
    Spread {
        source: SpreadSource,
        sink: SpreadSink,
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
            | Self::GetImport { dst, .. }
            | Self::GetGlobal { dst, .. }
            | Self::GetUpval { dst, .. }
            | Self::Closure { dst, .. }
            | Self::Vararg { dst, .. }
            | Self::Phi { dst, .. } => Some(*dst),
            Self::Call { dsts, .. } => dsts.first().copied(),
            _ => None,
        }
    }

    /// Every variable this instruction writes. `def_var` names only the single
    /// value an expression produces; a `for` loop also writes its control and
    /// loop variables on each iteration, and an analysis that cannot see those
    /// will treat a use of the loop variable as loop-invariant.
    pub fn def_vars(&self) -> Vec<IrVar> {
        match self {
            Self::ForPrep {
                base,
                limit,
                step,
                loop_var,
                ..
            } => vec![*base, *limit, *step, *loop_var],
            Self::ForLoop { base, .. } | Self::TForLoop { base, .. } => vec![*base],
            Self::TForCall {
                base,
                state,
                ctrl,
                vars,
            } => {
                let mut defs = vec![*base, *state, *ctrl];
                defs.extend(vars.iter().copied());
                defs
            }
            Self::Call { dsts, .. } => dsts.clone(),
            Self::Spread {
                sink: SpreadSink::Call { dsts, .. },
                ..
            } => dsts.clone(),
            _ => self.def_var().into_iter().collect(),
        }
    }

    pub fn use_vars(&self) -> Vec<IrVar> {
        match self {
            Self::Move { src, .. } | Self::UnOp { src, .. } => vec![*src],
            Self::AppendArray { table, src } => vec![*table, *src],
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
            Self::Closure { captures, .. } => captures.iter().flatten().copied().collect(),
            Self::Spread { source, sink } => {
                let mut uses = source.use_vars();
                match sink {
                    SpreadSink::Call {
                        callee, fixed_args, ..
                    } => {
                        uses.push(*callee);
                        uses.extend(fixed_args.iter().copied());
                    }
                    SpreadSink::List { table } => uses.push(*table),
                    SpreadSink::Return => {}
                }
                uses
            }
            Self::Phi { incoming, .. } => incoming.iter().map(|(_, v)| *v).collect(),
            _ => Vec::new(),
        }
    }

    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            Self::Return(_)
                | Self::Spread {
                    sink: SpreadSink::Return,
                    ..
                }
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
