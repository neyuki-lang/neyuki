// AST Expression node definitions for Neyuki.

use crate::ast::stmt::Stmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Literal(String),
    Str(String),
    Interp(Vec<InterpPart>),
    Variable(String),
    Vararg,
    Member {
        object: Box<Expr>,
        field: String,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    Function {
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
    Unary {
        op: String,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: String,
        right: Box<Expr>,
    },
    Table(Vec<TableEntry>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterpPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableEntry {
    pub key: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    pub variadic: bool,
}
