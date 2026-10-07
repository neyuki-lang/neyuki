// Semantic type representations and type compatibility checking for Neyuki.

use crate::ast::ty::TypeExpr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NeyukiType {
    Any,
    Never,
    Nil,
    Boolean,
    Int,
    Float,
    Number,
    String,
    Table,
    Array(Box<NeyukiType>),
    Record(Vec<(String, NeyukiType)>),
    TableMap {
        key: Box<NeyukiType>,
        val: Box<NeyukiType>,
    },
    Buffer,
    Thread,
    Function {
        params: Vec<NeyukiType>,
        return_type: Box<NeyukiType>,
        is_vararg: bool,
    },
    Union(Vec<NeyukiType>),
    Optional(Box<NeyukiType>),
    Custom(String),
}

impl NeyukiType {
    pub fn parse(s: &str) -> Self {
        let ast_ty = TypeExpr::parse(s);
        Self::from_ast_type(&ast_ty)
    }

    pub fn from_ast_type(ty: &TypeExpr) -> Self {
        match ty {
            TypeExpr::Any => NeyukiType::Any,
            TypeExpr::Nil => NeyukiType::Nil,
            TypeExpr::Never => NeyukiType::Never,
            TypeExpr::Named(name) => match name.to_lowercase().as_str() {
                "any" => NeyukiType::Any,
                "nil" | "void" => NeyukiType::Nil,
                "never" => NeyukiType::Never,
                "bool" | "boolean" => NeyukiType::Boolean,
                "int" | "integer" | "i32" | "i64" | "u32" | "u64" => NeyukiType::Int,
                "float" | "f32" | "f64" => NeyukiType::Float,
                "num" | "number" => NeyukiType::Number,
                "str" | "string" => NeyukiType::String,
                "table" | "dict" => NeyukiType::Table,
                "array" | "list" => NeyukiType::Array(Box::new(NeyukiType::Any)),
                "buf" | "buffer" => NeyukiType::Buffer,
                "thread" | "coroutine" => NeyukiType::Thread,
                other => NeyukiType::Custom(other.to_string()),
            },
            TypeExpr::Optional(inner) => NeyukiType::Optional(Box::new(Self::from_ast_type(inner))),
            TypeExpr::Array(inner) => NeyukiType::Array(Box::new(Self::from_ast_type(inner))),
            TypeExpr::Tuple(elems) => NeyukiType::Record(
                elems
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (format!("_{}", i), Self::from_ast_type(e)))
                    .collect(),
            ),
            TypeExpr::Record(fields) => NeyukiType::Record(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::from_ast_type(v)))
                    .collect(),
            ),
            TypeExpr::Table { key, val } => NeyukiType::TableMap {
                key: Box::new(Self::from_ast_type(key)),
                val: Box::new(Self::from_ast_type(val)),
            },
            TypeExpr::Function { params, ret } => NeyukiType::Function {
                params: params.iter().map(Self::from_ast_type).collect(),
                return_type: Box::new(Self::from_ast_type(ret)),
                is_vararg: false,
            },
            TypeExpr::Union(variants) => {
                let list: Vec<NeyukiType> = variants.iter().map(Self::from_ast_type).collect();
                NeyukiType::Union(list)
            }
        }
    }

    pub fn is_assignable_to(&self, target: &NeyukiType) -> bool {
        if *self == NeyukiType::Any || *target == NeyukiType::Any {
            return true;
        }
        if *self == NeyukiType::Never {
            return true;
        }
        if self == target {
            return true;
        }

        // Subtypes of Number
        if matches!(self, NeyukiType::Int | NeyukiType::Float) && *target == NeyukiType::Number {
            return true;
        }

        // Union target: A is assignable to A | B if A is assignable to any variant
        if let NeyukiType::Union(variants) = target {
            return variants.iter().any(|v| self.is_assignable_to(v));
        }

        // Union source: A | B is assignable to T if all variants are assignable to T
        if let NeyukiType::Union(variants) = self {
            return !variants.is_empty() && variants.iter().all(|v| v.is_assignable_to(target));
        }

        // Optional target: T? accepts T or Nil
        if let NeyukiType::Optional(target_inner) = target {
            if *self == NeyukiType::Nil {
                return true;
            }
            return self.is_assignable_to(target_inner);
        }

        // Optional source: T? is assignable to target if T and Nil both are
        if let NeyukiType::Optional(source_inner) = self {
            return source_inner.is_assignable_to(target)
                && NeyukiType::Nil.is_assignable_to(target);
        }

        match (self, target) {
            (NeyukiType::Nil, _) => false,
            (NeyukiType::Number, NeyukiType::Number) => true,
            (NeyukiType::Int, NeyukiType::Int) => true,
            (NeyukiType::Float, NeyukiType::Float) => true,
            (NeyukiType::String, NeyukiType::String) => true,
            (NeyukiType::Boolean, NeyukiType::Boolean) => true,
            (NeyukiType::Table, NeyukiType::Table) => true,
            (NeyukiType::Table, NeyukiType::Array(_)) => true,
            (NeyukiType::Table, NeyukiType::Record(_)) => true,
            (NeyukiType::Table, NeyukiType::TableMap { .. }) => true,
            (NeyukiType::Array(_), NeyukiType::Table) => true,
            (NeyukiType::Record(_), NeyukiType::Table) => true,
            (NeyukiType::TableMap { .. }, NeyukiType::Table) => true,
            (NeyukiType::Array(a), NeyukiType::Array(b)) => a.is_assignable_to(b),
            (NeyukiType::Record(src_fields), NeyukiType::Record(dst_fields)) => {
                for (dst_name, dst_ty) in dst_fields {
                    if let Some((_, src_ty)) = src_fields.iter().find(|(name, _)| name == dst_name)
                    {
                        if !src_ty.is_assignable_to(dst_ty) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                true
            }
            (
                NeyukiType::TableMap { key: k1, val: v1 },
                NeyukiType::TableMap { key: k2, val: v2 },
            ) => k1.is_assignable_to(k2) && v1.is_assignable_to(v2),
            (NeyukiType::Buffer, NeyukiType::Buffer) => true,
            (NeyukiType::Thread, NeyukiType::Thread) => true,
            (
                NeyukiType::Function {
                    params: p1,
                    return_type: r1,
                    is_vararg: v1,
                },
                NeyukiType::Function {
                    params: p2,
                    return_type: r2,
                    is_vararg: v2,
                },
            ) => {
                if v1 != v2 || p1.len() != p2.len() {
                    return false;
                }
                for (a, b) in p1.iter().zip(p2.iter()) {
                    if !b.is_assignable_to(a) {
                        return false;
                    }
                }
                r1.is_assignable_to(r2)
            }
            (NeyukiType::Custom(a), NeyukiType::Custom(b)) => a == b,
            _ => false,
        }
    }

    pub fn lub(&self, other: &NeyukiType) -> NeyukiType {
        if self == other {
            return self.clone();
        }
        if *self == NeyukiType::Any || *other == NeyukiType::Any {
            return NeyukiType::Any;
        }
        if *self == NeyukiType::Never {
            return other.clone();
        }
        if *other == NeyukiType::Never {
            return self.clone();
        }
        if (*self == NeyukiType::Int && *other == NeyukiType::Float)
            || (*self == NeyukiType::Float && *other == NeyukiType::Int)
            || (*self == NeyukiType::Number && matches!(other, NeyukiType::Int | NeyukiType::Float))
            || (*other == NeyukiType::Number && matches!(self, NeyukiType::Int | NeyukiType::Float))
        {
            return NeyukiType::Number;
        }
        if *self == NeyukiType::Nil {
            return NeyukiType::Optional(Box::new(other.clone()));
        }
        if *other == NeyukiType::Nil {
            return NeyukiType::Optional(Box::new(self.clone()));
        }
        NeyukiType::Union(vec![self.clone(), other.clone()])
    }

    pub fn narrow_truthy(&self) -> NeyukiType {
        match self {
            NeyukiType::Nil => NeyukiType::Never,
            NeyukiType::Optional(inner) => *inner.clone(),
            NeyukiType::Union(variants) => {
                let filtered: Vec<NeyukiType> = variants
                    .iter()
                    .filter(|v| **v != NeyukiType::Nil)
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    NeyukiType::Never
                } else if filtered.len() == 1 {
                    filtered.into_iter().next().unwrap()
                } else {
                    NeyukiType::Union(filtered)
                }
            }
            other => other.clone(),
        }
    }

    pub fn narrow_non_nil(&self) -> NeyukiType {
        match self {
            NeyukiType::Nil => NeyukiType::Never,
            NeyukiType::Optional(inner) => *inner.clone(),
            NeyukiType::Union(variants) => {
                let filtered: Vec<NeyukiType> = variants
                    .iter()
                    .filter(|v| **v != NeyukiType::Nil)
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    NeyukiType::Never
                } else if filtered.len() == 1 {
                    filtered.into_iter().next().unwrap()
                } else {
                    NeyukiType::Union(filtered)
                }
            }
            other => other.clone(),
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            NeyukiType::Any => "any".to_string(),
            NeyukiType::Never => "never".to_string(),
            NeyukiType::Nil => "nil".to_string(),
            NeyukiType::Boolean => "boolean".to_string(),
            NeyukiType::Int => "int".to_string(),
            NeyukiType::Float => "float".to_string(),
            NeyukiType::Number => "number".to_string(),
            NeyukiType::String => "string".to_string(),
            NeyukiType::Table => "table".to_string(),
            NeyukiType::Array(elem) => format!("[{}]", elem.display_name()),
            NeyukiType::Record(fields) => {
                let f = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.display_name()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{}}}", f)
            }
            NeyukiType::TableMap { key, val } => {
                format!("{{[{}]: {}}}", key.display_name(), val.display_name())
            }
            NeyukiType::Buffer => "buffer".to_string(),
            NeyukiType::Thread => "thread".to_string(),
            NeyukiType::Function {
                params,
                return_type,
                is_vararg,
            } => {
                let mut p = params
                    .iter()
                    .map(|t| t.display_name())
                    .collect::<Vec<_>>()
                    .join(", ");
                if *is_vararg {
                    if !p.is_empty() {
                        p.push_str(", ");
                    }
                    p.push_str("...");
                }
                format!("({}) -> {}", p, return_type.display_name())
            }
            NeyukiType::Union(variants) => {
                let s = variants
                    .iter()
                    .map(|v| v.display_name())
                    .collect::<Vec<_>>()
                    .join(" | ");
                format!("({})", s)
            }
            NeyukiType::Optional(inner) => format!("{}?", inner.display_name()),
            NeyukiType::Custom(name) => name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_types_parsing_and_subtyping() {
        let t_int = NeyukiType::parse("int");
        let t_num = NeyukiType::parse("number");
        assert_eq!(t_int, NeyukiType::Int);
        assert_eq!(t_num, NeyukiType::Number);
        assert!(t_int.is_assignable_to(&t_num));
        assert!(!t_num.is_assignable_to(&t_int));

        let t_opt = NeyukiType::parse("string?");
        assert!(matches!(t_opt, NeyukiType::Optional(_)));
        assert!(NeyukiType::String.is_assignable_to(&t_opt));
        assert!(NeyukiType::Nil.is_assignable_to(&t_opt));

        let t_union = NeyukiType::parse("int | string");
        assert!(matches!(t_union, NeyukiType::Union(_)));
        assert!(NeyukiType::Int.is_assignable_to(&t_union));
        assert!(NeyukiType::String.is_assignable_to(&t_union));
        assert!(!NeyukiType::Boolean.is_assignable_to(&t_union));
    }

    #[test]
    fn test_types_lub_and_narrowing() {
        let t_int = NeyukiType::Int;
        let t_float = NeyukiType::Float;
        assert_eq!(t_int.lub(&t_float), NeyukiType::Number);

        let t_opt_str = NeyukiType::Optional(Box::new(NeyukiType::String));
        assert_eq!(t_opt_str.narrow_truthy(), NeyukiType::String);
        assert_eq!(t_opt_str.narrow_non_nil(), NeyukiType::String);
    }
}
