// IR Instruction definitions for 3-Address Code.

use crate::compiler::ir::types::{IrBinaryOp, IrConstant, IrLabel, IrUnaryOp, IrVar};

#[derive(Clone, Debug)]
pub enum IrInst {
    LoadConst { dst: IrVar, val: IrConstant },
    LoadNil { dst: IrVar },
    Move { dst: IrVar, src: IrVar },
    BinOp { dst: IrVar, op: IrBinaryOp, lhs: IrVar, rhs: IrVar },
    UnOp { dst: IrVar, op: IrUnaryOp, src: IrVar },
    NewTable { dst: IrVar },
    GetTable { dst: IrVar, table: IrVar, key: IrVar },
    SetTable { table: IrVar, key: IrVar, val: IrVar },
    AppendArray { table: IrVar, src: IrVar },
    GetGlobal { dst: IrVar, name: String },
    SetGlobal { name: String, src: IrVar },
    GetUpval { dst: IrVar, index: u8 },
    SetUpval { index: u8, src: IrVar },
    Call { dst: Option<IrVar>, callee: IrVar, args: Vec<IrVar>, retc: u8 },
    Return(Vec<IrVar>),
    Closure { dst: IrVar, proto_idx: u16 },
    Vararg { dst: IrVar, count: u8 },
    ForPrep { base: IrVar, jump: IrLabel },
    ForLoop { base: IrVar, jump: IrLabel },
    TForCall { base: IrVar, retc: u8 },
    TForLoop { base: IrVar, jump: IrLabel },
    Jump(IrLabel),
    JumpIfFalse { cond: IrVar, target: IrLabel },
    Label(IrLabel),
}
