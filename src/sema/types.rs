// Semantic type representations and type compatibility checking for Neyuki.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NeyukiType {
    Any,
    Nil,
    Boolean,
    Number,
    String,
    Table,
    Buffer,
    Thread,
    Function {
        params: Vec<NeyukiType>,
        return_type: Box<NeyukiType>,
        is_vararg: bool,
    },
    Custom(String),
}

impl NeyukiType {
    pub fn parse(s: &str) -> Self {
        let trimmed = s.trim();
        match trimmed.to_lowercase().as_str() {
            "any" => NeyukiType::Any,
            "nil" | "void" => NeyukiType::Nil,
            "bool" | "boolean" => NeyukiType::Boolean,
            "num" | "number" | "int" | "float" | "i32" | "i64" | "u32" | "u64" | "f32" | "f64" => {
                NeyukiType::Number
            }
            "str" | "string" => NeyukiType::String,
            "table" | "dict" | "array" | "list" => NeyukiType::Table,
            "buf" | "buffer" => NeyukiType::Buffer,
            "thread" | "coroutine" => NeyukiType::Thread,
            other => NeyukiType::Custom(other.to_string()),
        }
    }

    pub fn is_assignable_to(&self, target: &NeyukiType) -> bool {
        if *self == NeyukiType::Any || *target == NeyukiType::Any {
            return true;
        }
        match (self, target) {
            (NeyukiType::Nil, _) => true, // nil is assignable to any optional / reference slot
            (NeyukiType::Number, NeyukiType::Number) => true,
            (NeyukiType::String, NeyukiType::String) => true,
            (NeyukiType::Boolean, NeyukiType::Boolean) => true,
            (NeyukiType::Table, NeyukiType::Table) => true,
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

    pub fn display_name(&self) -> String {
        match self {
            NeyukiType::Any => "any".to_string(),
            NeyukiType::Nil => "nil".to_string(),
            NeyukiType::Boolean => "boolean".to_string(),
            NeyukiType::Number => "number".to_string(),
            NeyukiType::String => "string".to_string(),
            NeyukiType::Table => "table".to_string(),
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
            NeyukiType::Custom(name) => name.clone(),
        }
    }
}
