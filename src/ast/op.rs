// Type-safe operator definitions for AST expressions and statements.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    LShl,
    LShr,
    // String
    Concat,
    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Logical
    And,
    Or,
    // Nil-coalescing
    Coalesce,
}

impl BinOp {
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "+" => Some(Self::Add),
            "-" => Some(Self::Sub),
            "*" => Some(Self::Mul),
            "/" => Some(Self::Div),
            "//" => Some(Self::IDiv),
            "%" => Some(Self::Mod),
            "^" => Some(Self::Pow),
            "&" => Some(Self::BitAnd),
            "|" => Some(Self::BitOr),
            "~" => Some(Self::BitXor),
            "<<" => Some(Self::Shl),
            ">>" => Some(Self::Shr),
            "<<<" => Some(Self::LShl),
            ">>>" => Some(Self::LShr),
            ".." => Some(Self::Concat),
            "==" => Some(Self::Eq),
            "!=" | "~=" => Some(Self::Ne),
            "<" => Some(Self::Lt),
            "<=" => Some(Self::Le),
            ">" => Some(Self::Gt),
            ">=" => Some(Self::Ge),
            "and" | "&&" => Some(Self::And),
            "or" | "||" => Some(Self::Or),
            "??" => Some(Self::Coalesce),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::IDiv => "//",
            Self::Mod => "%",
            Self::Pow => "^",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "~",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::LShl => "<<<",
            Self::LShr => ">>>",
            Self::Concat => "..",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::And => "and",
            Self::Or => "or",
            Self::Coalesce => "??",
        }
    }

    pub fn is_comparison(&self) -> bool {
        matches!(
            self,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge
        )
    }

    pub fn is_arithmetic(&self) -> bool {
        matches!(
            self,
            Self::Add | Self::Sub | Self::Mul | Self::Div | Self::IDiv | Self::Mod | Self::Pow
        )
    }

    pub fn is_bitwise(&self) -> bool {
        matches!(
            self,
            Self::BitAnd
                | Self::BitOr
                | Self::BitXor
                | Self::Shl
                | Self::Shr
                | Self::LShl
                | Self::LShr
        )
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    Len,
}

impl UnOp {
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "-" => Some(Self::Neg),
            "not" | "!" => Some(Self::Not),
            "~" => Some(Self::BitNot),
            "#" => Some(Self::Len),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "not",
            Self::BitNot => "~",
            Self::Len => "#",
        }
    }
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompoundOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Concat,
}

impl CompoundOp {
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "+=" => Some(Self::Add),
            "-=" => Some(Self::Sub),
            "*=" => Some(Self::Mul),
            "/=" => Some(Self::Div),
            "%=" => Some(Self::Mod),
            "..=" => Some(Self::Concat),
            _ => None,
        }
    }

    pub fn to_binop(self) -> BinOp {
        match self {
            Self::Add => BinOp::Add,
            Self::Sub => BinOp::Sub,
            Self::Mul => BinOp::Mul,
            Self::Div => BinOp::Div,
            Self::Mod => BinOp::Mod,
            Self::Concat => BinOp::Concat,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssignOp {
    Assign,
    Compound(CompoundOp),
}
