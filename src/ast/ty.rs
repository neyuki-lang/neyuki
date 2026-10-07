// AST Type Annotation expressions for Neyuki.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeExpr {
    Named(String),
    Nil,
    Never,
    Optional(Box<TypeExpr>),
    Array(Box<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Record(Vec<(String, TypeExpr)>),
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
        if trimmed == "nil" {
            return Self::Nil;
        }
        if trimmed == "never" || trimmed == "void" {
            return Self::Never;
        }

        // Arrow function syntax: (p1, p2) -> ret or p1 -> ret
        if let Some((lhs, rhs)) = trimmed.split_once("->") {
            let lhs = lhs.trim();
            let ret = Box::new(Self::parse(rhs));
            let params = if (lhs.starts_with('(') && lhs.ends_with(')'))
                || (lhs.starts_with('[') && lhs.ends_with(']'))
            {
                let inside = &lhs[1..lhs.len() - 1].trim();
                if inside.is_empty() {
                    Vec::new()
                } else {
                    inside.split(',').map(Self::parse).collect()
                }
            } else if lhs.is_empty() {
                Vec::new()
            } else {
                vec![Self::parse(lhs)]
            };
            return Self::Function { params, ret };
        }

        // Union `A | B`
        if trimmed.contains('|') {
            let parts: Vec<TypeExpr> = trimmed.split('|').map(Self::parse).collect();
            return Self::Union(parts);
        }

        // Optional suffix `?`
        if let Some(stripped) = trimmed.strip_suffix('?') {
            return Self::Optional(Box::new(Self::parse(stripped)));
        }

        // Tuple `(A, B, C)`
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            let inner = trimmed[1..trimmed.len() - 1].trim();
            if inner.contains(',') {
                let elems = inner.split(',').map(Self::parse).collect();
                return Self::Tuple(elems);
            }
            if !inner.is_empty() {
                return Self::parse(inner);
            }
        }

        // Array `[T]` or Record/Table `{ ... }`
        if (trimmed.starts_with('[') && trimmed.ends_with(']'))
            || (trimmed.starts_with('{') && trimmed.ends_with('}'))
        {
            let inner = trimmed[1..trimmed.len() - 1].trim();
            if inner.is_empty() {
                return Self::Table {
                    key: Box::new(Self::Any),
                    val: Box::new(Self::Any),
                };
            }

            // Key-value map syntax: `{[k]: v}` or `{ [k]: v }`
            if inner.starts_with('[')
                && let Some((k, v)) = inner.split_once("]:")
            {
                let key_str = k.trim().trim_start_matches('[');
                return Self::Table {
                    key: Box::new(Self::parse(key_str)),
                    val: Box::new(Self::parse(v)),
                };
            }

            // Record syntax `{ field1: type1, field2: type2 }`
            if inner.contains(':') && !inner.starts_with('[') {
                let mut fields = Vec::new();
                for part in inner.split(',') {
                    if let Some((fname, fty)) = part.split_once(':') {
                        fields.push((fname.trim().to_string(), Self::parse(fty)));
                    }
                }
                if !fields.is_empty() {
                    return Self::Record(fields);
                }
            }

            return Self::Array(Box::new(Self::parse(inner)));
        }

        Self::Named(trimmed.to_string())
    }

    pub fn is_optional(&self) -> bool {
        matches!(self, Self::Optional(_))
    }

    pub fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => write!(f, "{}", name),
            Self::Nil => write!(f, "nil"),
            Self::Never => write!(f, "never"),
            Self::Optional(inner) => write!(f, "{}?", inner),
            Self::Array(inner) => write!(f, "[{}]", inner),
            Self::Tuple(elems) => {
                let s = elems
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "({})", s)
            }
            Self::Record(fields) => {
                let s = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{{{}}}", s)
            }
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
