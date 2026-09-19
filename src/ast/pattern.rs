// Assignable l-value / pattern definitions for Neyuki.

use crate::ast::expr::Expr;
use crate::ast::node_id::NodeId;
use crate::ast::span::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssignTarget {
    Variable(String),
    Member { object: Box<Expr>, field: String },
    Index { object: Box<Expr>, index: Box<Expr> },
}

impl AssignTarget {
    pub fn from_expr(expr: Expr) -> Result<Self, String> {
        match expr {
            Expr::Variable { name, .. } => Ok(Self::Variable(name)),
            Expr::Member { object, field, .. } => Ok(Self::Member { object, field }),
            Expr::Index { object, index, .. } => Ok(Self::Index { object, index }),
            _ => Err("invalid assignment target".to_string()),
        }
    }

    pub fn from_expr_ref(expr: &Expr) -> Result<Self, String> {
        match expr {
            Expr::Variable { name, .. } => Ok(Self::Variable(name.clone())),
            Expr::Member { object, field, .. } => Ok(Self::Member {
                object: object.clone(),
                field: field.clone(),
            }),
            Expr::Index { object, index, .. } => Ok(Self::Index {
                object: object.clone(),
                index: index.clone(),
            }),
            _ => Err("invalid assignment target".to_string()),
        }
    }

    pub fn to_expr(&self) -> Expr {
        match self {
            Self::Variable(name) => Expr::Variable {
                name: name.clone(),
                id: NodeId::next(),
            },
            Self::Member { object, field } => Expr::Member {
                object: object.clone(),
                field: field.clone(),
                id: NodeId::next(),
            },
            Self::Index { object, index } => Expr::Index {
                object: object.clone(),
                index: index.clone(),
                id: NodeId::next(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpannedAssignTarget {
    pub target: AssignTarget,
    pub span: Span,
}
