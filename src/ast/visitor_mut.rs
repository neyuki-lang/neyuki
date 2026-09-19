// In-place AST mutable visitor trait and walking functions for Neyuki.

use crate::ast::expr::{Expr, InterpPart, TableEntry};
use crate::ast::stmt::Stmt;

pub trait AstVisitorMut {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        walk_expr_mut(self, expr);
    }
}

pub fn walk_stmt_mut<V: AstVisitorMut + ?Sized>(visitor: &mut V, stmt: &mut Stmt) {
    match stmt {
        Stmt::Local { initializer, .. } => {
            if let Some(init) = initializer {
                visitor.visit_expr_mut(init);
            }
        }
        Stmt::LocalMany { initializers, .. } => {
            for init in initializers {
                visitor.visit_expr_mut(init);
            }
        }
        Stmt::Assign { target, value, .. } => {
            walk_assign_target_mut(visitor, target);
            visitor.visit_expr_mut(value);
        }
        Stmt::AssignMany {
            targets, values, ..
        } => {
            for t in targets {
                walk_assign_target_mut(visitor, t);
            }
            for v in values {
                visitor.visit_expr_mut(v);
            }
        }
        Stmt::Increment { target, .. } => {
            walk_assign_target_mut(visitor, target);
        }
        Stmt::Function { body, .. } => {
            for s in body {
                visitor.visit_stmt_mut(s);
            }
        }
        Stmt::If {
            condition,
            then_branch,
            else_if_branches,
            else_branch,
            ..
        } => {
            visitor.visit_expr_mut(condition);
            for s in then_branch {
                visitor.visit_stmt_mut(s);
            }
            for (cond, branch) in else_if_branches {
                visitor.visit_expr_mut(cond);
                for s in branch {
                    visitor.visit_stmt_mut(s);
                }
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    visitor.visit_stmt_mut(s);
                }
            }
        }
        Stmt::For { source, body, .. } => {
            visitor.visit_expr_mut(source);
            for s in body {
                visitor.visit_stmt_mut(s);
            }
        }
        Stmt::NumericFor {
            start,
            end,
            step,
            body,
            ..
        } => {
            visitor.visit_expr_mut(start);
            visitor.visit_expr_mut(end);
            if let Some(st) = step {
                visitor.visit_expr_mut(st);
            }
            for s in body {
                visitor.visit_stmt_mut(s);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            visitor.visit_expr_mut(condition);
            for s in body {
                visitor.visit_stmt_mut(s);
            }
        }
        Stmt::Repeat {
            body, condition, ..
        } => {
            for s in body {
                visitor.visit_stmt_mut(s);
            }
            visitor.visit_expr_mut(condition);
        }
        Stmt::Return { values, .. } => {
            for e in values {
                visitor.visit_expr_mut(e);
            }
        }
        Stmt::Expr { expr, .. } => {
            visitor.visit_expr_mut(expr);
        }
        Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Goto { .. } | Stmt::Label { .. } => {}
    }
}

pub fn walk_assign_target_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    target: &mut crate::ast::pattern::AssignTarget,
) {
    match target {
        crate::ast::pattern::AssignTarget::Variable(_) => {}
        crate::ast::pattern::AssignTarget::Member { object, .. } => {
            visitor.visit_expr_mut(object);
        }
        crate::ast::pattern::AssignTarget::Index { object, index } => {
            visitor.visit_expr_mut(object);
            visitor.visit_expr_mut(index);
        }
    }
}

pub fn walk_expr_mut<V: AstVisitorMut + ?Sized>(visitor: &mut V, expr: &mut Expr) {
    match expr {
        Expr::Interp { parts, .. } => {
            for part in parts {
                if let InterpPart::Expr(e) = part {
                    visitor.visit_expr_mut(e);
                }
            }
        }
        Expr::Member { object, .. } => {
            visitor.visit_expr_mut(object);
        }
        Expr::Index { object, index, .. } => {
            visitor.visit_expr_mut(object);
            visitor.visit_expr_mut(index);
        }
        Expr::Call { callee, args, .. } => {
            visitor.visit_expr_mut(callee);
            for arg in args {
                visitor.visit_expr_mut(arg);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            visitor.visit_expr_mut(object);
            for arg in args {
                visitor.visit_expr_mut(arg);
            }
        }
        Expr::Function { body, .. } => {
            for s in body {
                visitor.visit_stmt_mut(s);
            }
        }
        Expr::Unary { expr: inner, .. } => {
            visitor.visit_expr_mut(inner);
        }
        Expr::Binary { left, right, .. } => {
            visitor.visit_expr_mut(left);
            visitor.visit_expr_mut(right);
        }
        Expr::Table { entries, .. } => {
            for TableEntry { value, .. } in entries {
                visitor.visit_expr_mut(value);
            }
        }
        Expr::Literal { .. } | Expr::Variable { .. } | Expr::Vararg { .. } => {}
    }
}
