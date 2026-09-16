// AST Statement node definitions for Neyuki.

use crate::ast::expr::{Expr, Param};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    Local {
        name: String,
        is_const: bool,
        type_name: Option<String>,
        initializer: Option<Expr>,
    },
    LocalMany {
        names: Vec<String>,
        is_const: bool,
        initializers: Vec<Expr>,
    },
    Assign {
        target: Expr,
        value: Expr,
        is_const: bool,
    },
    AssignMany {
        targets: Vec<Expr>,
        values: Vec<Expr>,
    },
    Increment {
        target: Expr,
        amount: i8,
    },
    Function {
        name: Option<String>,
        is_const: bool,
        params: Vec<Param>,
        return_type: Option<String>,
        body: Vec<Stmt>,
    },
    If {
        condition: Expr,
        then_branch: Vec<Stmt>,
        else_if_branches: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Option<Vec<Stmt>>,
    },
    For {
        vars: Vec<String>,
        source: Expr,
        body: Vec<Stmt>,
    },
    NumericFor {
        var: String,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    Repeat {
        body: Vec<Stmt>,
        condition: Expr,
    },
    Return(Vec<Expr>),
    Break,
    Continue,
    Expr(Expr),
}
