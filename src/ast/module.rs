// Module / Chunk root node representing a complete source file AST.

use crate::ast::node_id::NodeId;
use crate::ast::span::Span;
use crate::ast::stmt::Stmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub id: NodeId,
    pub path: Option<String>,
    pub statements: Vec<Stmt>,
    pub span: Span,
}

impl Module {
    pub fn new(path: Option<String>, statements: Vec<Stmt>) -> Self {
        Self {
            id: NodeId::next(),
            path,
            statements,
            span: Span::default(),
        }
    }
}
