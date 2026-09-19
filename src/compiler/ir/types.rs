// IR Primitive types, variables, constants, and operators for Neyuki.

use num_bigint::BigInt;

// Virtual variable ID in IR (unbounded, not limited to 255)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IrVar(pub u32);

// Label for control flow targets
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IrLabel(pub usize);

impl IrLabel {
    pub const fn new(id: usize) -> Self {
        Self(id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum IrConstant {
    Nil,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    String(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IrBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    LShl,
    LShr,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Coalesce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IrUnaryOp {
    Neg,
    Not,
    Len,
    BitNot,
}
