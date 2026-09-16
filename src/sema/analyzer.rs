// Semantic analyzer visiting AST nodes, building symbol tables, and generating diagnostics.

use std::collections::HashSet;

use crate::ast::expr::Expr;
use crate::ast::stmt::Stmt;
use crate::diagnostics::diagnostic::Diagnostic;
use crate::diagnostics::error_code::ErrorCode;
use crate::diagnostics::span::Span;
use crate::sema::scope::{ScopeKind, ScopeManager};
use crate::sema::symbol::{Symbol, SymbolKind};
use crate::sema::types::NeyukiType;

pub struct SemanticAnalyzer<'a> {
    pub source: &'a str,
    pub scope_mgr: ScopeManager,
    pub diagnostics: Vec<Diagnostic>,
    pub known_globals: HashSet<String>,
    current_return_type: Option<NeyukiType>,
    current_line: u32,
}

impl<'a> SemanticAnalyzer<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut known_globals = HashSet::new();
        let globals = [
            "print", "assert", "require", "type", "tostring", "tonumber", "pcall", "xpcall",
            "try", "error", "pairs", "ipairs", "next", "select", "rawget", "rawset",
            "setmetatable", "getmetatable", "collectgarbage", "math", "string", "table",
            "bit", "bit32", "buffer", "os", "coroutine", "utf8", "debug", "json",
            "true", "false", "nil", "_G", "_VERSION", "warn",
        ];
        for g in globals {
            known_globals.insert(g.to_string());
        }

        Self {
            source,
            scope_mgr: ScopeManager::new(),
            diagnostics: Vec::new(),
            known_globals,
            current_return_type: None,
            current_line: 1,
        }
    }

    pub fn analyze_program(&mut self, statements: &[Stmt]) {
        self.analyze_block(statements);
        // Flush all scope symbols
        let remaining = self.scope_mgr.flush_all();
        for sym in remaining {
            if !sym.used && !sym.name.starts_with('_') && sym.kind == SymbolKind::Variable {
                let diag = Diagnostic::warning(format!("unused variable '{}'", sym.name))
                    .with_code(ErrorCode::W0001)
                    .with_label(sym.span, "variable declared here but never used")
                    .with_help(format!("if this is intentional, prefix with an underscore: '_{}'", sym.name));
                self.diagnostics.push(diag);
            }
        }
    }

    fn analyze_block(&mut self, statements: &[Stmt]) {
        let mut unreachable = false;
        for stmt in statements {
            if unreachable {
                let span = self.find_stmt_span(stmt);
                let diag = Diagnostic::warning("unreachable code")
                    .with_code(ErrorCode::W0004)
                    .with_label(span, "unreachable statement follows terminal control flow");
                self.diagnostics.push(diag);
                continue;
            }

            self.analyze_stmt(stmt);

            if matches!(stmt, Stmt::Return(_) | Stmt::Break | Stmt::Continue) {
                unreachable = true;
            }
        }
    }

    fn analyze_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Local {
                name,
                is_const,
                type_name,
                initializer,
            } => {
                let span = self.find_ident_span(name);
                self.check_shadowing(name, span);

                let declared_type = type_name.as_deref().map(NeyukiType::parse);

                if let Some(init) = initializer {
                    self.analyze_expr(init);
                    let init_type = self.infer_expr_type(init);
                    if let Some(decl_type) = &declared_type
                        && init_type != NeyukiType::Any && !init_type.is_assignable_to(decl_type) {
                            let diag = Diagnostic::error(format!(
                                "type mismatch: variable '{}' declared as '{}' but initialized with '{}'",
                                name,
                                decl_type.display_name(),
                                init_type.display_name()
                            ))
                            .with_code(ErrorCode::E0003)
                            .with_label(span, format!("expected '{}', found '{}'", decl_type.display_name(), init_type.display_name()))
                            .with_help(format!("ensure the assigned value matches '{}'", decl_type.display_name()));
                            self.diagnostics.push(diag);
                    }
                }

                let mut sym = Symbol::new(
                    name.clone(),
                    SymbolKind::Variable,
                    *is_const,
                    declared_type,
                    span,
                );
                if let Some(init) = initializer {
                    sym.inferred_type = self.infer_expr_type(init);
                }
                if let Err(orig_span) = self.scope_mgr.define(sym) {
                    let diag = Diagnostic::error(format!("duplicate local variable '{}' in the same scope", name))
                        .with_code(ErrorCode::E0005)
                        .with_label(span, "redefined here")
                        .with_secondary_label(orig_span, "previous definition was here");
                    self.diagnostics.push(diag);
                }
            }
            Stmt::LocalMany {
                names,
                is_const,
                initializers,
            } => {
                for init in initializers {
                    self.analyze_expr(init);
                }
                for (i, name) in names.iter().enumerate() {
                    let span = self.find_ident_span(name);
                    self.check_shadowing(name, span);

                    let mut sym = Symbol::new(
                        name.clone(),
                        SymbolKind::Variable,
                        *is_const,
                        None,
                        span,
                    );
                    if let Some(init) = initializers.get(i) {
                        sym.inferred_type = self.infer_expr_type(init);
                    }
                    if let Err(orig_span) = self.scope_mgr.define(sym) {
                        let diag = Diagnostic::error(format!("duplicate local variable '{}' in the same scope", name))
                            .with_code(ErrorCode::E0005)
                            .with_label(span, "redefined here")
                            .with_secondary_label(orig_span, "previous definition was here");
                        self.diagnostics.push(diag);
                    }
                }
            }
            Stmt::Assign {
                target,
                value,
                is_const: _,
            } => {
                self.analyze_expr(value);
                self.check_assign_target(target, value);
            }
            Stmt::AssignMany { targets, values } => {
                for v in values {
                    self.analyze_expr(v);
                }
                for (i, t) in targets.iter().enumerate() {
                    if let Some(v) = values.get(i) {
                        self.check_assign_target(t, v);
                    } else {
                        self.check_assign_target(t, &Expr::Literal("nil".to_string()));
                    }
                }
            }
            Stmt::Increment { target, amount: _ } => {
                if let Expr::Variable(name) = target {
                    let span = self.find_ident_span(name);
                    if let Some(sym) = self.scope_mgr.lookup_mut(name) {
                        sym.used = true;
                        if sym.is_const {
                            let diag = Diagnostic::error(format!("cannot increment const variable '{}'", name))
                                .with_code(ErrorCode::E0002)
                                .with_label(span, "cannot mutate a const variable")
                                .with_secondary_label(sym.span, "defined as const here");
                            self.diagnostics.push(diag);
                        }
                    } else if !self.known_globals.contains(name) {
                        let diag = Diagnostic::error(format!("variable '{}' is not declared", name))
                            .with_code(ErrorCode::E0001)
                            .with_label(span, "cannot increment undeclared variable");
                        self.diagnostics.push(diag);
                    }
                }
            }
            Stmt::Function {
                name,
                is_const,
                params,
                return_type,
                body,
            } => {
                let ret_ty = return_type.as_deref().map(NeyukiType::parse);
                if let Some(fn_name) = name {
                    let span = self.find_ident_span(fn_name);
                    self.check_shadowing(fn_name, span);

                    let param_types: Vec<NeyukiType> = params
                        .iter()
                        .map(|p| {
                            p.type_name
                                .as_deref()
                                .map(NeyukiType::parse)
                                .unwrap_or(NeyukiType::Any)
                        })
                        .collect();

                    let is_vararg = params.iter().any(|p| p.variadic);
                    let fn_type = NeyukiType::Function {
                        params: param_types,
                        return_type: Box::new(ret_ty.clone().unwrap_or(NeyukiType::Any)),
                        is_vararg,
                    };

                    let mut sym = Symbol::new(
                        fn_name.clone(),
                        SymbolKind::Function,
                        *is_const,
                        Some(fn_type),
                        span,
                    );
                    sym.num_params = Some(params.iter().filter(|p| !p.variadic).count());
                    sym.is_vararg = is_vararg;
                    let _ = self.scope_mgr.define(sym);
                }

                // Analyze function body with its own scope
                self.scope_mgr.enter_scope(ScopeKind::Function);
                let prev_ret = self.current_return_type.clone();
                self.current_return_type = ret_ty;

                for param in params {
                    let span = self.find_ident_span(&param.name);
                    let decl_ty = param.type_name.as_deref().map(NeyukiType::parse);
                    let sym = Symbol::new(
                        param.name.clone(),
                        SymbolKind::Parameter,
                        false,
                        decl_ty,
                        span,
                    );
                    let _ = self.scope_mgr.define(sym);
                }

                self.analyze_block(body);

                self.check_unused_symbols();
                self.current_return_type = prev_ret;
            }
            Stmt::If {
                condition,
                then_branch,
                else_if_branches,
                else_branch,
            } => {
                self.analyze_expr(condition);
                self.scope_mgr.enter_scope(ScopeKind::Block);
                self.analyze_block(then_branch);
                self.check_unused_symbols();

                for (cond, branch) in else_if_branches {
                    self.analyze_expr(cond);
                    self.scope_mgr.enter_scope(ScopeKind::Block);
                    self.analyze_block(branch);
                    self.check_unused_symbols();
                }

                if let Some(branch) = else_branch {
                    self.scope_mgr.enter_scope(ScopeKind::Block);
                    self.analyze_block(branch);
                    self.check_unused_symbols();
                }
            }
            Stmt::While { condition, body } => {
                self.analyze_expr(condition);
                self.scope_mgr.enter_scope(ScopeKind::Loop);
                self.analyze_block(body);
                self.check_unused_symbols();
            }
            Stmt::Repeat { body, condition } => {
                self.scope_mgr.enter_scope(ScopeKind::Loop);
                self.analyze_block(body);
                self.analyze_expr(condition);
                self.check_unused_symbols();
            }
            Stmt::NumericFor {
                var,
                start,
                end,
                step,
                body,
            } => {
                self.analyze_expr(start);
                self.analyze_expr(end);
                if let Some(s) = step {
                    self.analyze_expr(s);
                }
                self.scope_mgr.enter_scope(ScopeKind::Loop);
                let span = self.find_ident_span(var);
                let mut sym = Symbol::new(
                    var.clone(),
                    SymbolKind::Variable,
                    false,
                    Some(NeyukiType::Number),
                    span,
                );
                sym.used = true; // loop variable is automatically considered used
                let _ = self.scope_mgr.define(sym);
                self.analyze_block(body);
                self.check_unused_symbols();
            }
            Stmt::For { vars, source, body } => {
                self.analyze_expr(source);
                self.scope_mgr.enter_scope(ScopeKind::Loop);
                for v in vars {
                    let span = self.find_ident_span(v);
                    let mut sym = Symbol::new(v.clone(), SymbolKind::Variable, false, None, span);
                    sym.used = true;
                    let _ = self.scope_mgr.define(sym);
                }
                self.analyze_block(body);
                self.check_unused_symbols();
            }
            Stmt::Return(exprs) => {
                for e in exprs {
                    self.analyze_expr(e);
                }
                if let Some(expected_ret) = &self.current_return_type {
                    if exprs.is_empty() {
                        if *expected_ret != NeyukiType::Nil && *expected_ret != NeyukiType::Any {
                            let span = Span::line(self.current_line);
                            let diag = Diagnostic::error(format!(
                                "empty return in function declared to return '{}'",
                                expected_ret.display_name()
                            ))
                            .with_code(ErrorCode::E0003)
                            .with_label(span, format!("expected return of type '{}'", expected_ret.display_name()));
                            self.diagnostics.push(diag);
                        }
                    } else {
                        let actual_ret = self.infer_expr_type(&exprs[0]);
                        if actual_ret != NeyukiType::Any && !actual_ret.is_assignable_to(expected_ret) {
                            let span = self.find_expr_span(&exprs[0]);
                            let diag = Diagnostic::error(format!(
                                "type mismatch: function declared with return type '{}' returns '{}'",
                                expected_ret.display_name(),
                                actual_ret.display_name()
                            ))
                            .with_code(ErrorCode::E0003)
                            .with_label(span, format!("expected '{}', found '{}'", expected_ret.display_name(), actual_ret.display_name()));
                            self.diagnostics.push(diag);
                        }
                    }
                }
            }
            Stmt::Break | Stmt::Continue => {}
            Stmt::Expr(e) => {
                self.analyze_expr(e);
            }
        }
    }

    fn check_assign_target(&mut self, target: &Expr, value: &Expr) {
        if let Expr::Variable(name) = target {
            let span = self.find_ident_span(name);
            let val_type = self.infer_expr_type(value);

            if let Some(sym) = self.scope_mgr.lookup_mut(name) {
                sym.assigned_count += 1;
                sym.used = true;
                if sym.is_const {
                    let diag = Diagnostic::error(format!("cannot reassign to const variable '{}'", name))
                        .with_code(ErrorCode::E0002)
                        .with_label(span, "cannot reassign to a const variable")
                        .with_secondary_label(sym.span, "defined as const here");
                    self.diagnostics.push(diag);
                } else if let Some(decl_type) = &sym.declared_type
                    && val_type != NeyukiType::Any && !val_type.is_assignable_to(decl_type) {
                        let diag = Diagnostic::error(format!(
                            "type mismatch in assignment to '{}': expected '{}', found '{}'",
                            name,
                            decl_type.display_name(),
                            val_type.display_name()
                        ))
                        .with_code(ErrorCode::E0003)
                        .with_label(span, format!("expected '{}', found '{}'", decl_type.display_name(), val_type.display_name()));
                        self.diagnostics.push(diag);
                }
            } else if !self.known_globals.contains(name) {
                let diag = Diagnostic::error(format!("variable '{}' is used before declaration", name))
                    .with_code(ErrorCode::E0001)
                    .with_label(span, "not found in this scope")
                    .with_help(format!("declare 'local {} = ...' before using it", name));
                self.diagnostics.push(diag);
            }
        } else {
            self.analyze_expr(target);
        }
    }

    fn analyze_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Variable(name) => {
                let span = self.find_ident_span(name);
                if !self.scope_mgr.mark_used(name) && !self.known_globals.contains(name) {
                    let diag = Diagnostic::error(format!("cannot find variable '{}' in this scope", name))
                        .with_code(ErrorCode::E0001)
                        .with_label(span, "not found in this scope")
                        .with_help(format!("declare 'local {} = ...' before accessing it", name));
                    self.diagnostics.push(diag);
                }
            }
            Expr::Call { callee, args } => {
                self.analyze_expr(callee);
                for a in args {
                    self.analyze_expr(a);
                }
                // Arity check if callee is a known local function
                if let Expr::Variable(fn_name) = callee.as_ref()
                    && let Some(sym) = self.scope_mgr.lookup(fn_name)
                    && let Some(expected) = sym.num_params
                    && !sym.is_vararg
                    && args.len() != expected {
                        let span = self.find_ident_span(fn_name);
                        let diag = Diagnostic::error(format!(
                            "function '{}' takes {} argument(s) but {} were supplied",
                            fn_name,
                            expected,
                            args.len()
                        ))
                        .with_code(ErrorCode::E0004)
                        .with_label(span, format!("takes {} arguments", expected))
                        .with_secondary_label(sym.span, "defined here");
                        self.diagnostics.push(diag);
                }
            }
            Expr::MethodCall { object, method: _, args } => {
                self.analyze_expr(object);
                for a in args {
                    self.analyze_expr(a);
                }
            }
            Expr::Member { object, field: _ } => {
                self.analyze_expr(object);
            }
            Expr::Index { object, index } => {
                self.analyze_expr(object);
                self.analyze_expr(index);
            }
            Expr::Unary { op: _, expr } => {
                self.analyze_expr(expr);
            }
            Expr::Binary { left, op: _, right } => {
                self.analyze_expr(left);
                self.analyze_expr(right);
            }
            Expr::Table(entries) => {
                for entry in entries {
                    self.analyze_expr(&entry.value);
                }
            }
            Expr::Function { params, body } => {
                self.scope_mgr.enter_scope(ScopeKind::Function);
                for param in params {
                    let span = self.find_ident_span(&param.name);
                    let decl_ty = param.type_name.as_deref().map(NeyukiType::parse);
                    let sym = Symbol::new(
                        param.name.clone(),
                        SymbolKind::Parameter,
                        false,
                        decl_ty,
                        span,
                    );
                    let _ = self.scope_mgr.define(sym);
                }
                self.analyze_block(body);
                self.check_unused_symbols();
            }
            Expr::Interp(parts) => {
                for part in parts {
                    if let crate::ast::expr::InterpPart::Expr(e) = part {
                        self.analyze_expr(e);
                    }
                }
            }
            Expr::Literal(_) | Expr::Str(_) | Expr::Vararg => {}
        }
    }

    fn infer_expr_type(&self, expr: &Expr) -> NeyukiType {
        match expr {
            Expr::Literal(val) => {
                if val == "nil" {
                    NeyukiType::Nil
                } else if val == "true" || val == "false" {
                    NeyukiType::Boolean
                } else {
                    NeyukiType::Number
                }
            }
            Expr::Str(_) => NeyukiType::String,
            Expr::Table(_) => NeyukiType::Table,
            Expr::Function { .. } => NeyukiType::Function {
                params: Vec::new(),
                return_type: Box::new(NeyukiType::Any),
                is_vararg: false,
            },
            Expr::Variable(name) => {
                if let Some(sym) = self.scope_mgr.lookup(name) {
                    sym.inferred_type.clone()
                } else {
                    NeyukiType::Any
                }
            }
            Expr::Unary { op, .. } => {
                if op == "not" {
                    NeyukiType::Boolean
                } else {
                    NeyukiType::Number
                }
            }
            Expr::Binary { left: _, op, right: _ } => match op.as_str() {
                "==" | "!=" | "<" | "<=" | ">" | ">=" => NeyukiType::Boolean,
                ".." => NeyukiType::String,
                _ => NeyukiType::Number,
            },
            _ => NeyukiType::Any,
        }
    }

    fn check_shadowing(&mut self, name: &str, span: Span) {
        if let Some(outer) = self.scope_mgr.find_outer(name) {
            let diag = Diagnostic::warning(format!("declaration of '{}' shadows a variable in an outer scope", name))
                .with_code(ErrorCode::W0003)
                .with_label(span, "shadows previous declaration")
                .with_secondary_label(outer.span, "outer variable was declared here");
            self.diagnostics.push(diag);
        }
    }

    fn check_unused_symbols(&mut self) {
        let symbols = self.scope_mgr.exit_scope();
        for sym in symbols {
            if !sym.used && !sym.name.starts_with('_') {
                let (code, msg, label_msg) = if sym.kind == SymbolKind::Parameter {
                    (ErrorCode::W0002, format!("unused parameter '{}'", sym.name), "parameter never used")
                } else {
                    (ErrorCode::W0001, format!("unused variable '{}'", sym.name), "variable never used")
                };
                let diag = Diagnostic::warning(msg)
                    .with_code(code)
                    .with_label(sym.span, label_msg)
                    .with_help(format!("if this is intentional, prefix with an underscore: '_{}'", sym.name));
                self.diagnostics.push(diag);
            }
        }
    }

    fn find_ident_span(&self, ident: &str) -> Span {
        for (line_idx, line) in self.source.lines().enumerate() {
            if let Some(col) = line.find(ident) {
                return Span::single_line(
                    (line_idx + 1) as u32,
                    (col + 1) as u32,
                    (col + 1 + ident.len()) as u32,
                );
            }
        }
        Span::line(1)
    }

    fn find_stmt_span(&self, _stmt: &Stmt) -> Span {
        Span::line(self.current_line)
    }

    fn find_expr_span(&self, _expr: &Expr) -> Span {
        Span::line(self.current_line)
    }
}
