// Typed AST Literal definitions for Neyuki.

use num_bigint::BigInt;
use std::fmt;

#[derive(Clone, Debug)]
pub enum Literal {
    Nil,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    String(String),
}

impl PartialEq for Literal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Nil, Self::Nil) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::String(a), Self::String(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Literal {}

impl Literal {
    pub fn parse_number(s: &str) -> Option<Self> {
        let normalized = s.replace('_', "");
        let integer = if let Some(hex) = normalized
            .strip_prefix("0x")
            .or_else(|| normalized.strip_prefix("0X"))
        {
            BigInt::parse_bytes(hex.as_bytes(), 16)
        } else if let Some(bin) = normalized
            .strip_prefix("0b")
            .or_else(|| normalized.strip_prefix("0B"))
        {
            BigInt::parse_bytes(bin.as_bytes(), 2)
        } else if let Some(oct) = normalized
            .strip_prefix("0o")
            .or_else(|| normalized.strip_prefix("0O"))
        {
            BigInt::parse_bytes(oct.as_bytes(), 8)
        } else if !normalized.contains('.')
            && !normalized.contains('e')
            && !normalized.contains('E')
        {
            BigInt::parse_bytes(normalized.as_bytes(), 10)
        } else {
            None
        };
        if let Some(bi) = integer {
            Some(Literal::Int(bi))
        } else if let Ok(f) = normalized.parse::<f64>() {
            Some(Literal::Float(f))
        } else {
            None
        }
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Literal::Nil)
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Literal::Nil => false,
            Literal::Bool(b) => *b,
            _ => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Literal::Nil => "nil",
            Literal::Bool(_) => "bool",
            Literal::Int(_) => "int",
            Literal::Float(_) => "float",
            Literal::String(_) => "string",
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Literal::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<&BigInt> {
        match self {
            Literal::Int(i) => Some(i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Literal::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Literal::String(s) => Some(s),
            _ => None,
        }
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Nil => write!(f, "nil"),
            Literal::Bool(b) => write!(f, "{}", b),
            Literal::Int(i) => write!(f, "{}", i),
            Literal::Float(fl) => write!(f, "{}", fl),
            Literal::String(s) => write!(f, "\"{}\"", s.escape_debug()),
        }
    }
}
