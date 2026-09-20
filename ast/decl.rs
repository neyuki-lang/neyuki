// Declaration definitions for Neyuki AST.

use crate::ast::expr::{Expr, Param};
use crate::ast::span::Span;
use crate::ast::stmt::Stmt;
use crate::ast::ty::TypeExpr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decl {
    Local {
        name: String,
        is_const: bool,
        ty: Option<TypeExpr>,
        initializer: Option<Expr>,
        span: Span,
    },
    Function {
        name: Option<String>,
        is_const: bool,
        params: Vec<Param>,
        return_type: Option<TypeExpr>,
        body: Vec<Stmt>,
        span: Span,
    },
}

impl Decl {
    pub fn span(&self) -> Span {
        match self {
            Self::Local { span, .. } | Self::Function { span, .. } => *span,
        }
    }
}
