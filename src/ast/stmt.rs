// AST Statement node definitions for Neyuki.

use crate::ast::expr::{Expr, Param};
use crate::ast::node_id::NodeId;
use crate::ast::pattern::AssignTarget;

#[derive(Clone, Debug, Eq)]
pub enum Stmt {
    Local {
        name: String,
        is_const: bool,
        type_name: Option<String>,
        initializer: Option<Expr>,
        id: NodeId,
    },
    LocalMany {
        names: Vec<String>,
        is_const: bool,
        initializers: Vec<Expr>,
        id: NodeId,
    },
    Assign {
        target: AssignTarget,
        value: Expr,
        is_const: bool,
        id: NodeId,
    },
    AssignMany {
        targets: Vec<AssignTarget>,
        values: Vec<Expr>,
        id: NodeId,
    },
    Increment {
        target: AssignTarget,
        amount: i8,
        id: NodeId,
    },
    Function {
        name: Option<String>,
        is_const: bool,
        params: Vec<Param>,
        return_type: Option<String>,
        body: Vec<Stmt>,
        id: NodeId,
    },
    If {
        condition: Expr,
        then_branch: Vec<Stmt>,
        else_if_branches: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Option<Vec<Stmt>>,
        id: NodeId,
    },
    For {
        vars: Vec<String>,
        source: Expr,
        body: Vec<Stmt>,
        id: NodeId,
    },
    NumericFor {
        var: String,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
        id: NodeId,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
        id: NodeId,
    },
    Repeat {
        body: Vec<Stmt>,
        condition: Expr,
        id: NodeId,
    },
    Return {
        values: Vec<Expr>,
        id: NodeId,
    },
    Break {
        id: NodeId,
    },
    Continue {
        id: NodeId,
    },
    Goto {
        label: String,
        id: NodeId,
    },
    Label {
        name: String,
        id: NodeId,
    },
    Expr {
        expr: Expr,
        id: NodeId,
    },
}

impl PartialEq for Stmt {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Stmt::Local {
                    name: na,
                    is_const: ca,
                    type_name: ta,
                    initializer: ia,
                    ..
                },
                Stmt::Local {
                    name: nb,
                    is_const: cb,
                    type_name: tb,
                    initializer: ib,
                    ..
                },
            ) => na == nb && ca == cb && ta == tb && ia == ib,
            (
                Stmt::LocalMany {
                    names: na,
                    is_const: ca,
                    initializers: ia,
                    ..
                },
                Stmt::LocalMany {
                    names: nb,
                    is_const: cb,
                    initializers: ib,
                    ..
                },
            ) => na == nb && ca == cb && ia == ib,
            (
                Stmt::Assign {
                    target: ta,
                    value: va,
                    is_const: ca,
                    ..
                },
                Stmt::Assign {
                    target: tb,
                    value: vb,
                    is_const: cb,
                    ..
                },
            ) => ta == tb && va == vb && ca == cb,
            (
                Stmt::AssignMany {
                    targets: ta,
                    values: va,
                    ..
                },
                Stmt::AssignMany {
                    targets: tb,
                    values: vb,
                    ..
                },
            ) => ta == tb && va == vb,
            (
                Stmt::Increment {
                    target: ta,
                    amount: aa,
                    ..
                },
                Stmt::Increment {
                    target: tb,
                    amount: ab,
                    ..
                },
            ) => ta == tb && aa == ab,
            (
                Stmt::Function {
                    name: na,
                    is_const: ca,
                    params: pa,
                    return_type: ra,
                    body: ba,
                    ..
                },
                Stmt::Function {
                    name: nb,
                    is_const: cb,
                    params: pb,
                    return_type: rb,
                    body: bb,
                    ..
                },
            ) => na == nb && ca == cb && pa == pb && ra == rb && ba == bb,
            (
                Stmt::If {
                    condition: ca,
                    then_branch: ta,
                    else_if_branches: ea,
                    else_branch: ela,
                    ..
                },
                Stmt::If {
                    condition: cb,
                    then_branch: tb,
                    else_if_branches: eb,
                    else_branch: elb,
                    ..
                },
            ) => ca == cb && ta == tb && ea == eb && ela == elb,
            (
                Stmt::For {
                    vars: va,
                    source: sa,
                    body: ba,
                    ..
                },
                Stmt::For {
                    vars: vb,
                    source: sb,
                    body: bb,
                    ..
                },
            ) => va == vb && sa == sb && ba == bb,
            (
                Stmt::NumericFor {
                    var: va,
                    start: sa,
                    end: ea,
                    step: sta,
                    body: ba,
                    ..
                },
                Stmt::NumericFor {
                    var: vb,
                    start: sb,
                    end: eb,
                    step: stb,
                    body: bb,
                    ..
                },
            ) => va == vb && sa == sb && ea == eb && sta == stb && ba == bb,
            (
                Stmt::While {
                    condition: ca,
                    body: ba,
                    ..
                },
                Stmt::While {
                    condition: cb,
                    body: bb,
                    ..
                },
            ) => ca == cb && ba == bb,
            (
                Stmt::Repeat {
                    body: ba,
                    condition: ca,
                    ..
                },
                Stmt::Repeat {
                    body: bb,
                    condition: cb,
                    ..
                },
            ) => ba == bb && ca == cb,
            (Stmt::Return { values: va, .. }, Stmt::Return { values: vb, .. }) => va == vb,
            (Stmt::Break { .. }, Stmt::Break { .. }) => true,
            (Stmt::Continue { .. }, Stmt::Continue { .. }) => true,
            (Stmt::Goto { label: la, .. }, Stmt::Goto { label: lb, .. }) => la == lb,
            (Stmt::Label { name: na, .. }, Stmt::Label { name: nb, .. }) => na == nb,
            (Stmt::Expr { expr: ea, .. }, Stmt::Expr { expr: eb, .. }) => ea == eb,
            _ => false,
        }
    }
}

impl Stmt {
    pub fn node_id(&self) -> NodeId {
        match self {
            Stmt::Local { id, .. }
            | Stmt::LocalMany { id, .. }
            | Stmt::Assign { id, .. }
            | Stmt::AssignMany { id, .. }
            | Stmt::Increment { id, .. }
            | Stmt::Function { id, .. }
            | Stmt::If { id, .. }
            | Stmt::For { id, .. }
            | Stmt::NumericFor { id, .. }
            | Stmt::While { id, .. }
            | Stmt::Repeat { id, .. }
            | Stmt::Return { id, .. }
            | Stmt::Break { id }
            | Stmt::Continue { id }
            | Stmt::Goto { id, .. }
            | Stmt::Label { id, .. }
            | Stmt::Expr { id, .. } => *id,
        }
    }

    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            Self::Return { .. } | Self::Break { .. } | Self::Continue { .. } | Self::Goto { .. }
        )
    }
}
