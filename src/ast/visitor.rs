// AST Visitor pattern trait for AST traversal, linting, analysis and transformations.

#![allow(dead_code)]

use crate::ast::expr::Expr;
use crate::ast::stmt::Stmt;

pub trait AstVisitor {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        walk_expr(self, expr);
    }
}

pub fn walk_stmt<V: AstVisitor + ?Sized>(visitor: &mut V, stmt: &Stmt) {
    match stmt {
        Stmt::Local { initializer, .. } => {
            if let Some(init) = initializer {
                visitor.visit_expr(init);
            }
        }
        Stmt::LocalMany { initializers, .. } => {
            for init in initializers {
                visitor.visit_expr(init);
            }
        }
        Stmt::Assign { target, value, .. } => {
            visitor.visit_expr(target);
            visitor.visit_expr(value);
        }
        Stmt::AssignMany { targets, values } => {
            for t in targets {
                visitor.visit_expr(t);
            }
            for v in values {
                visitor.visit_expr(v);
            }
        }
        Stmt::Increment { target, .. } => {
            visitor.visit_expr(target);
        }
        Stmt::Function { body, .. } => {
            for s in body {
                visitor.visit_stmt(s);
            }
        }
        Stmt::If {
            condition,
            then_branch,
            else_if_branches,
            else_branch,
        } => {
            visitor.visit_expr(condition);
            for s in then_branch {
                visitor.visit_stmt(s);
            }
            for (c, b) in else_if_branches {
                visitor.visit_expr(c);
                for s in b {
                    visitor.visit_stmt(s);
                }
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    visitor.visit_stmt(s);
                }
            }
        }
        Stmt::For { source, body, .. } => {
            visitor.visit_expr(source);
            for s in body {
                visitor.visit_stmt(s);
            }
        }
        Stmt::NumericFor {
            start,
            end,
            step,
            body,
            ..
        } => {
            visitor.visit_expr(start);
            visitor.visit_expr(end);
            if let Some(st) = step {
                visitor.visit_expr(st);
            }
            for s in body {
                visitor.visit_stmt(s);
            }
        }
        Stmt::While { condition, body } => {
            visitor.visit_expr(condition);
            for s in body {
                visitor.visit_stmt(s);
            }
        }
        Stmt::Repeat { body, condition } => {
            for s in body {
                visitor.visit_stmt(s);
            }
            visitor.visit_expr(condition);
        }
        Stmt::Return(exprs) => {
            for e in exprs {
                visitor.visit_expr(e);
            }
        }
        Stmt::Expr(expr) => {
            visitor.visit_expr(expr);
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

pub fn walk_expr<V: AstVisitor + ?Sized>(visitor: &mut V, expr: &Expr) {
    match expr {
        Expr::Literal(_) | Expr::Str(_) | Expr::Variable(_) | Expr::Vararg => {}
        Expr::Interp(parts) => {
            for p in parts {
                if let crate::ast::expr::InterpPart::Expr(e) = p {
                    visitor.visit_expr(e);
                }
            }
        }
        Expr::Member { object, .. } => {
            visitor.visit_expr(object);
        }
        Expr::Index { object, index } => {
            visitor.visit_expr(object);
            visitor.visit_expr(index);
        }
        Expr::Call { callee, args } => {
            visitor.visit_expr(callee);
            for a in args {
                visitor.visit_expr(a);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            visitor.visit_expr(object);
            for a in args {
                visitor.visit_expr(a);
            }
        }
        Expr::Function { body, .. } => {
            for s in body {
                visitor.visit_stmt(s);
            }
        }
        Expr::Unary { expr, .. } => {
            visitor.visit_expr(expr);
        }
        Expr::Binary { left, right, .. } => {
            visitor.visit_expr(left);
            visitor.visit_expr(right);
        }
        Expr::Table(entries) => {
            for entry in entries {
                visitor.visit_expr(&entry.value);
            }
        }
    }
}
