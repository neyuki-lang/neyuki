#![allow(dead_code)]

use crate::parser::{Expr, TableEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableShapeKind {
    Empty,
    PureArray(usize),
    Record(Vec<String>),
    Mixed {
        array_len: usize,
        fields: Vec<String>,
    },
    Dynamic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableShape {
    pub kind: TableShapeKind,
    pub array_capacity: usize,
    pub hash_capacity: usize,
    pub has_duplicate_keys: bool,
}

impl TableShape {
    // Analyzes a list of table entries from AST table constructor: { k1 = v1, k2 = v2, ... }
    pub fn analyze(entries: &[TableEntry]) -> Self {
        if entries.is_empty() {
            return Self {
                kind: TableShapeKind::Empty,
                array_capacity: 0,
                hash_capacity: 0,
                has_duplicate_keys: false,
            };
        }

        let mut array_count = 0usize;
        let mut field_names = Vec::new();
        let mut seen_fields = std::collections::HashSet::new();
        let mut has_duplicate_keys = false;
        let mut is_pure_array = true;
        let mut is_pure_record = true;

        for entry in entries {
            match &entry.key {
                None => {
                    // Array element e.g. "foo" in {"foo", "bar"}
                    array_count += 1;
                    is_pure_record = false;
                }
                Some(name) => {
                    // Field property e.g. key = "val"
                    is_pure_array = false;
                    if !seen_fields.insert(name.clone()) {
                        has_duplicate_keys = true;
                    }
                    field_names.push(name.clone());
                }
            }
        }

        let kind = if is_pure_array {
            TableShapeKind::PureArray(array_count)
        } else if is_pure_record {
            TableShapeKind::Record(field_names)
        } else if array_count > 0 && !seen_fields.is_empty() {
            TableShapeKind::Mixed {
                array_len: array_count,
                fields: field_names,
            }
        } else {
            TableShapeKind::Dynamic
        };

        Self {
            kind,
            array_capacity: array_count,
            hash_capacity: seen_fields.len(),
            has_duplicate_keys,
        }
    }

    // Returns whether this table constructor qualifies for batch initialization via SetList
    pub fn qualifies_for_setlist(&self) -> bool {
        match self.kind {
            TableShapeKind::PureArray(len) => len > 0 && len <= 250,
            TableShapeKind::Mixed { array_len, .. } => array_len > 0 && array_len <= 250,
            _ => false,
        }
    }
}

// Walks an expression tree to extract all table constructors and analyze their shapes
pub fn analyze_expr_shapes(expr: &Expr, out: &mut Vec<TableShape>) {
    match expr {
        Expr::Table(entries) => {
            out.push(TableShape::analyze(entries));
            for entry in entries {
                analyze_expr_shapes(&entry.value, out);
            }
        }
        Expr::Call { callee, args } => {
            analyze_expr_shapes(callee, out);
            for arg in args {
                analyze_expr_shapes(arg, out);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            analyze_expr_shapes(object, out);
            for arg in args {
                analyze_expr_shapes(arg, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            analyze_expr_shapes(left, out);
            analyze_expr_shapes(right, out);
        }
        Expr::Unary { expr, .. } => {
            analyze_expr_shapes(expr, out);
        }
        Expr::Member { object, .. } => {
            analyze_expr_shapes(object, out);
        }
        Expr::Index { object, index } => {
            analyze_expr_shapes(object, out);
            analyze_expr_shapes(index, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_shape_pure_array() {
        let entries = vec![
            TableEntry { key: None, value: Expr::Literal("1".to_string()) },
            TableEntry { key: None, value: Expr::Literal("2".to_string()) },
            TableEntry { key: None, value: Expr::Literal("3".to_string()) },
        ];
        let shape = TableShape::analyze(&entries);
        assert_eq!(shape.kind, TableShapeKind::PureArray(3));
        assert_eq!(shape.array_capacity, 3);
        assert_eq!(shape.hash_capacity, 0);
        assert!(shape.qualifies_for_setlist());
    }

    #[test]
    fn test_table_shape_record() {
        let entries = vec![
            TableEntry { key: Some("name".to_string()), value: Expr::Str("Neyuki".to_string()) },
            TableEntry { key: Some("version".to_string()), value: Expr::Literal("1".to_string()) },
        ];
        let shape = TableShape::analyze(&entries);
        assert_eq!(
            shape.kind,
            TableShapeKind::Record(vec!["name".to_string(), "version".to_string()])
        );
        assert_eq!(shape.array_capacity, 0);
        assert_eq!(shape.hash_capacity, 2);
        assert!(!shape.qualifies_for_setlist());
    }

    #[test]
    fn test_table_shape_duplicate_keys() {
        let entries = vec![
            TableEntry { key: Some("a".to_string()), value: Expr::Literal("1".to_string()) },
            TableEntry { key: Some("a".to_string()), value: Expr::Literal("2".to_string()) },
        ];
        let shape = TableShape::analyze(&entries);
        assert!(shape.has_duplicate_keys);
        assert_eq!(shape.hash_capacity, 1);
    }
}
