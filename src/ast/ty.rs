// AST Type Annotation expressions for Neyuki.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeExpr {
    Named(String),
    Optional(Box<TypeExpr>),
    Array(Box<TypeExpr>),
    Table {
        key: Box<TypeExpr>,
        val: Box<TypeExpr>,
    },
    Function {
        params: Vec<TypeExpr>,
        ret: Box<TypeExpr>,
    },
    Union(Vec<TypeExpr>),
    Any,
}

impl TypeExpr {
    pub fn parse(s: &str) -> Self {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed == "any" {
            return Self::Any;
        }

        // Optional suffix `?`
        if let Some(stripped) = trimmed.strip_suffix('?') {
            return Self::Optional(Box::new(Self::parse(stripped)));
        }

        // Array `[T]` or `{T}`
        if (trimmed.starts_with('[') && trimmed.ends_with(']'))
            || (trimmed.starts_with('{') && trimmed.ends_with('}'))
        {
            let inner = &trimmed[1..trimmed.len() - 1].trim();
            if let Some((k, v)) = inner.split_once(':') {
                let key_str = k.trim().trim_start_matches('[').trim_end_matches(']');
                return Self::Table {
                    key: Box::new(Self::parse(key_str)),
                    val: Box::new(Self::parse(v)),
                };
            }
            return Self::Array(Box::new(Self::parse(inner)));
        }

        // Union `A | B`
        if trimmed.contains('|') {
            let parts: Vec<TypeExpr> = trimmed.split('|').map(Self::parse).collect();
            return Self::Union(parts);
        }

        Self::Named(trimmed.to_string())
    }

    pub fn is_optional(&self) -> bool {
        matches!(self, Self::Optional(_))
    }
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => write!(f, "{}", name),
            Self::Optional(inner) => write!(f, "{}?", inner),
            Self::Array(inner) => write!(f, "[{}]", inner),
            Self::Table { key, val } => write!(f, "{{[{}]: {}}}", key, val),
            Self::Function { params, ret } => {
                let ps = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "({}) -> {}", ps, ret)
            }
            Self::Union(types) => {
                let s = types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(" | ");
                write!(f, "{}", s)
            }
            Self::Any => write!(f, "any"),
        }
    }
}
