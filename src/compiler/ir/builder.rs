// AST to IR translation builder with scope analysis and variable resolution.

use num_bigint::BigInt;
use std::collections::HashMap;
use std::str::FromStr;

use crate::ast::{Expr, InterpPart, Param, Stmt};
use crate::compiler::ir::block::{IrFunction, IrModule};
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::{IrBinaryOp, IrConstant, IrLabel, IrUnaryOp, IrVar};

struct IrLoopContext {
    break_label: IrLabel,
    continue_label: IrLabel,
}

pub struct IrBuilder {
    next_var: u32,
    next_label: usize,
    instructions: Vec<IrInst>,
    scopes: Vec<HashMap<String, IrVar>>,
    loops: Vec<IrLoopContext>,
    child_protos: Vec<IrFunction>,
}

impl IrBuilder {
    pub fn new() -> Self {
        Self {
            next_var: 0,
            next_label: 0,
            instructions: Vec::new(),
            scopes: vec![HashMap::new()],
            loops: Vec::new(),
            child_protos: Vec::new(),
        }
    }

    pub fn alloc_var(&mut self) -> IrVar {
        let v = IrVar(self.next_var);
        self.next_var += 1;
        v
    }

    pub fn alloc_label(&mut self) -> IrLabel {
        let l = IrLabel(self.next_label);
        self.next_label += 1;
        l
    }

    pub fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn add_local(&mut self, name: String, var: IrVar) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, var);
        }
    }

    pub fn resolve_local(&self, name: &str) -> Option<IrVar> {
        for scope in self.scopes.iter().rev() {
            if let Some(&var) = scope.get(name) {
                return Some(var);
            }
        }
        None
    }

    pub fn emit(&mut self, inst: IrInst) {
        self.instructions.push(inst);
    }

    pub fn compile_expr(&mut self, expr: &Expr, target: Option<IrVar>) -> IrVar {
        let dst = target.unwrap_or_else(|| self.alloc_var());

        match expr {
            Expr::Literal(s) => match s.as_str() {
                "nil" => {
                    self.emit(IrInst::LoadNil { dst });
                }
                "true" => {
                    self.emit(IrInst::LoadConst { dst, val: IrConstant::Bool(true) });
                }
                "false" => {
                    self.emit(IrInst::LoadConst { dst, val: IrConstant::Bool(false) });
                }
                lit => {
                    if let Ok(i) = lit.parse::<i64>() {
                        self.emit(IrInst::LoadConst { dst, val: IrConstant::Int(BigInt::from(i)) });
                    } else if let Ok(bi) = BigInt::from_str(lit) {
                        self.emit(IrInst::LoadConst { dst, val: IrConstant::Int(bi) });
                    } else if let Ok(f) = lit.parse::<f64>() {
                        self.emit(IrInst::LoadConst { dst, val: IrConstant::Float(f) });
                    } else {
                        self.emit(IrInst::LoadConst { dst, val: IrConstant::String(lit.to_string()) });
                    }
                }
            },
            Expr::Str(s) => {
                self.emit(IrInst::LoadConst { dst, val: IrConstant::String(s.clone()) });
            }
            Expr::Interp(parts) => {
                if parts.is_empty() {
                    self.emit(IrInst::LoadConst { dst, val: IrConstant::String(String::new()) });
                } else {
                    let mut prev_var: Option<IrVar> = None;
                    for part in parts {
                        let part_var = match part {
                            InterpPart::Literal(lit) => {
                                let v = self.alloc_var();
                                self.emit(IrInst::LoadConst { dst: v, val: IrConstant::String(lit.clone()) });
                                v
                            }
                            InterpPart::Expr(e) => self.compile_expr(e, None),
                        };
                        if let Some(prev) = prev_var {
                            let next = self.alloc_var();
                            self.emit(IrInst::BinOp { dst: next, op: IrBinaryOp::Concat, lhs: prev, rhs: part_var });
                            prev_var = Some(next);
                        } else {
                            prev_var = Some(part_var);
                        }
                    }
                    if let Some(res) = prev_var {
                        self.emit(IrInst::Move { dst, src: res });
                    }
                }
            }
            Expr::Vararg => {
                self.emit(IrInst::Vararg { dst, count: 1 });
            }
            Expr::Variable(name) => {
                if let Some(local_var) = self.resolve_local(name) {
                    self.emit(IrInst::Move { dst, src: local_var });
                } else {
                    self.emit(IrInst::GetGlobal { dst, name: name.clone() });
                }
            }
            Expr::Binary { left, op, right } => {
                let lhs = self.compile_expr(left, None);
                let rhs = self.compile_expr(right, None);
                let ir_op = match op.as_str() {
                    "+" => IrBinaryOp::Add,
                    "-" => IrBinaryOp::Sub,
                    "*" => IrBinaryOp::Mul,
                    "/" => IrBinaryOp::Div,
                    "//" => IrBinaryOp::IDiv,
                    "%" => IrBinaryOp::Mod,
                    "^" => IrBinaryOp::Pow,
                    "&" => IrBinaryOp::BitAnd,
                    "|" => IrBinaryOp::BitOr,
                    "~" => IrBinaryOp::BitXor,
                    "<<" => IrBinaryOp::Shl,
                    ">>" => IrBinaryOp::Shr,
                    "<<<" => IrBinaryOp::LShl,
                    ">>>" => IrBinaryOp::LShr,
                    ".." => IrBinaryOp::Concat,
                    "==" => IrBinaryOp::Eq,
                    "!=" => IrBinaryOp::Ne,
                    "<" => IrBinaryOp::Lt,
                    "<=" => IrBinaryOp::Le,
                    ">" => IrBinaryOp::Gt,
                    ">=" => IrBinaryOp::Ge,
                    "??" => IrBinaryOp::Coalesce,
                    _ => IrBinaryOp::Add,
                };
                self.emit(IrInst::BinOp { dst, op: ir_op, lhs, rhs });
            }
            Expr::Unary { op, expr } => {
                let src = self.compile_expr(expr, None);
                let ir_op = match op.as_str() {
                    "-" => IrUnaryOp::Neg,
                    "not" => IrUnaryOp::Not,
                    "#" => IrUnaryOp::Len,
                    "~" => IrUnaryOp::BitNot,
                    _ => IrUnaryOp::Neg,
                };
                self.emit(IrInst::UnOp { dst, op: ir_op, src });
            }
            Expr::Call { callee, args } => {
                let callee_var = self.compile_expr(callee, None);
                let mut arg_vars = Vec::new();
                for arg in args {
                    arg_vars.push(self.compile_expr(arg, None));
                }
                self.emit(IrInst::Call {
                    dst: Some(dst),
                    callee: callee_var,
                    args: arg_vars,
                    retc: 1,
                });
            }
            Expr::MethodCall { object, method, args } => {
                let tbl = self.compile_expr(object, None);
                let key = self.alloc_var();
                self.emit(IrInst::LoadConst { dst: key, val: IrConstant::String(method.clone()) });
                let callee = self.alloc_var();
                self.emit(IrInst::GetTable { dst: callee, table: tbl, key });

                let mut arg_vars = vec![tbl];
                for arg in args {
                    arg_vars.push(self.compile_expr(arg, None));
                }
                self.emit(IrInst::Call {
                    dst: Some(dst),
                    callee,
                    args: arg_vars,
                    retc: 1,
                });
            }
            Expr::Table(entries) => {
                self.emit(IrInst::NewTable { dst });
                for entry in entries {
                    if let Some(ref k) = entry.key {
                        let key_var = self.alloc_var();
                        self.emit(IrInst::LoadConst { dst: key_var, val: IrConstant::String(k.clone()) });
                        let val_var = self.compile_expr(&entry.value, None);
                        self.emit(IrInst::SetTable { table: dst, key: key_var, val: val_var });
                    } else {
                        let elem_var = self.compile_expr(&entry.value, None);
                        self.emit(IrInst::AppendArray { table: dst, src: elem_var });
                    }
                }
            }
            Expr::Index { object, index } => {
                let tbl = self.compile_expr(object, None);
                let key = self.compile_expr(index, None);
                self.emit(IrInst::GetTable { dst, table: tbl, key });
            }
            Expr::Member { object, field } => {
                let tbl = self.compile_expr(object, None);
                let key = self.alloc_var();
                self.emit(IrInst::LoadConst { dst: key, val: IrConstant::String(field.clone()) });
                self.emit(IrInst::GetTable { dst, table: tbl, key });
            }
            Expr::Function { params, body } => {
                let proto_idx = self.compile_sub_function(None, params, body);
                self.emit(IrInst::Closure { dst, proto_idx });
            }
        }
        dst
    }

    pub fn compile_sub_function(&mut self, name: Option<String>, params: &[Param], body: &[Stmt]) -> u16 {
        let is_vararg = params.iter().any(|p| p.variadic);
        let num_params = params.iter().filter(|p| !p.variadic).count() as u8;

        let mut sub_builder = IrBuilder::new();
        sub_builder.enter_scope();
        for param in params {
            if !param.variadic {
                let v = sub_builder.alloc_var();
                sub_builder.add_local(param.name.clone(), v);
            }
        }
        for stmt in body {
            sub_builder.compile_stmt(stmt);
        }
        sub_builder.exit_scope();

        let has_terminal = matches!(sub_builder.instructions.last(), Some(IrInst::Return(_)));
        if !has_terminal {
            let nil_var = sub_builder.alloc_var();
            sub_builder.emit(IrInst::LoadNil { dst: nil_var });
            sub_builder.emit(IrInst::Return(vec![nil_var]));
        }

        let proto_idx = self.child_protos.len() as u16;
        self.child_protos.push(IrFunction {
            name,
            params: params.to_vec(),
            num_params,
            is_vararg,
            instructions: sub_builder.instructions,
            protos: sub_builder.child_protos,
        });
        proto_idx
    }

    pub fn compile_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Local { name, initializer, .. } => {
                let var = self.alloc_var();
                if let Some(init) = initializer {
                    self.compile_expr(init, Some(var));
                } else {
                    self.emit(IrInst::LoadNil { dst: var });
                }
                self.add_local(name.clone(), var);
            }
            Stmt::LocalMany { names, initializers, .. } => {
                for (i, name) in names.iter().enumerate() {
                    let var = self.alloc_var();
                    if let Some(init) = initializers.get(i) {
                        self.compile_expr(init, Some(var));
                    } else {
                        self.emit(IrInst::LoadNil { dst: var });
                    }
                    self.add_local(name.clone(), var);
                }
            }
            Stmt::Assign { target, value, .. } => {
                let val_var = self.compile_expr(value, None);
                self.compile_assign(target, val_var);
            }
            Stmt::AssignMany { targets, values } => {
                let mut val_vars = Vec::new();
                for val in values {
                    val_vars.push(self.compile_expr(val, None));
                }
                while val_vars.len() < targets.len() {
                    let nil_var = self.alloc_var();
                    self.emit(IrInst::LoadNil { dst: nil_var });
                    val_vars.push(nil_var);
                }
                for (target, val_var) in targets.iter().zip(val_vars.iter()) {
                    self.compile_assign(target, *val_var);
                }
            }
            Stmt::Increment { target, amount } => {
                let current = self.compile_expr(target, None);
                let amount_var = self.alloc_var();
                self.emit(IrInst::LoadConst { dst: amount_var, val: IrConstant::Int(BigInt::from(*amount)) });
                let result = self.alloc_var();
                self.emit(IrInst::BinOp { dst: result, op: IrBinaryOp::Add, lhs: current, rhs: amount_var });
                self.compile_assign(target, result);
            }
            Stmt::Function { name, params, body, .. } => {
                let proto_idx = self.compile_sub_function(name.clone(), params, body);
                let closure_var = self.alloc_var();
                self.emit(IrInst::Closure { dst: closure_var, proto_idx });
                if let Some(func_name) = name {
                    if let Some(local_var) = self.resolve_local(func_name) {
                        self.emit(IrInst::Move { dst: local_var, src: closure_var });
                    } else {
                        self.emit(IrInst::SetGlobal { name: func_name.clone(), src: closure_var });
                    }
                }
            }
            Stmt::If { condition, then_branch, else_if_branches, else_branch } => {
                let cond_var = self.compile_expr(condition, None);
                let else_label = self.alloc_label();
                let end_label = self.alloc_label();

                self.emit(IrInst::JumpIfFalse { cond: cond_var, target: else_label });

                self.enter_scope();
                for s in then_branch {
                    self.compile_stmt(s);
                }
                self.exit_scope();
                self.emit(IrInst::Jump(end_label));

                self.emit(IrInst::Label(else_label));
                for (elif_cond, elif_body) in else_if_branches {
                    let next_elif_label = self.alloc_label();
                    let elif_var = self.compile_expr(elif_cond, None);
                    self.emit(IrInst::JumpIfFalse { cond: elif_var, target: next_elif_label });
                    self.enter_scope();
                    for s in elif_body {
                        self.compile_stmt(s);
                    }
                    self.exit_scope();
                    self.emit(IrInst::Jump(end_label));
                    self.emit(IrInst::Label(next_elif_label));
                }

                if let Some(else_stmts) = else_branch {
                    self.enter_scope();
                    for s in else_stmts {
                        self.compile_stmt(s);
                    }
                    self.exit_scope();
                }

                self.emit(IrInst::Label(end_label));
            }
            Stmt::While { condition, body } => {
                let start_label = self.alloc_label();
                let exit_label = self.alloc_label();

                self.loops.push(IrLoopContext {
                    break_label: exit_label,
                    continue_label: start_label,
                });

                self.emit(IrInst::Label(start_label));
                let cond_var = self.compile_expr(condition, None);
                self.emit(IrInst::JumpIfFalse { cond: cond_var, target: exit_label });

                self.enter_scope();
                for s in body {
                    self.compile_stmt(s);
                }
                self.exit_scope();

                self.emit(IrInst::Jump(start_label));
                self.emit(IrInst::Label(exit_label));

                self.loops.pop();
            }
            Stmt::Repeat { body, condition } => {
                let start_label = self.alloc_label();
                let exit_label = self.alloc_label();
                let cond_label = self.alloc_label();

                self.loops.push(IrLoopContext {
                    break_label: exit_label,
                    continue_label: cond_label,
                });

                self.emit(IrInst::Label(start_label));

                self.enter_scope();
                for s in body {
                    self.compile_stmt(s);
                }
                self.exit_scope();

                self.emit(IrInst::Label(cond_label));
                let cond_var = self.compile_expr(condition, None);
                self.emit(IrInst::JumpIfFalse { cond: cond_var, target: start_label });
                self.emit(IrInst::Label(exit_label));

                self.loops.pop();
            }
            Stmt::NumericFor { var, start, end, step, body } => {
                self.enter_scope();
                let base = self.alloc_var();
                self.compile_expr(start, Some(base));
                let _limit = self.compile_expr(end, None);
                let _step_var = if let Some(s) = step {
                    self.compile_expr(s, None)
                } else {
                    let sv = self.alloc_var();
                    self.emit(IrInst::LoadConst { dst: sv, val: IrConstant::Int(BigInt::from(1)) });
                    sv
                };

                let loop_var = self.alloc_var();
                self.add_local(var.clone(), loop_var);

                let body_label = self.alloc_label();
                let exit_label = self.alloc_label();

                self.loops.push(IrLoopContext {
                    break_label: exit_label,
                    continue_label: body_label,
                });

                self.emit(IrInst::ForPrep { base, jump: exit_label });

                self.emit(IrInst::Label(body_label));
                self.enter_scope();
                for s in body {
                    self.compile_stmt(s);
                }
                self.exit_scope();

                self.emit(IrInst::ForLoop { base, jump: body_label });
                self.emit(IrInst::Label(exit_label));

                self.loops.pop();
                self.exit_scope();
            }
            Stmt::For { vars, source, body } => {
                self.enter_scope();
                let base = self.alloc_var();
                let state_var = self.alloc_var();
                let ctrl_var = self.alloc_var();

                match source {
                    Expr::Call { callee, args } => {
                        let callee_var = self.compile_expr(callee, Some(base));
                        let mut arg_vars = Vec::new();
                        for arg in args {
                            arg_vars.push(self.compile_expr(arg, None));
                        }
                        self.emit(IrInst::Call {
                            dst: Some(callee_var),
                            callee: callee_var,
                            args: arg_vars,
                            retc: 3,
                        });
                    }
                    _ => {
                        self.compile_expr(source, Some(base));
                        self.emit(IrInst::LoadNil { dst: state_var });
                        self.emit(IrInst::LoadNil { dst: ctrl_var });
                    }
                }

                let mut iter_vars = Vec::new();
                for vname in vars {
                    let iv = self.alloc_var();
                    self.add_local(vname.clone(), iv);
                    iter_vars.push(iv);
                }

                let call_label = self.alloc_label();
                let exit_label = self.alloc_label();

                self.loops.push(IrLoopContext {
                    break_label: exit_label,
                    continue_label: call_label,
                });

                self.emit(IrInst::Label(call_label));
                self.emit(IrInst::TForCall { base, retc: vars.len() as u8 });
                self.emit(IrInst::TForLoop { base, jump: exit_label });

                self.enter_scope();
                for s in body {
                    self.compile_stmt(s);
                }
                self.exit_scope();

                self.emit(IrInst::Jump(call_label));
                self.emit(IrInst::Label(exit_label));

                self.loops.pop();
                self.exit_scope();
            }
            Stmt::Break => {
                if let Some(lp) = self.loops.last() {
                    self.emit(IrInst::Jump(lp.break_label));
                }
            }
            Stmt::Continue => {
                if let Some(lp) = self.loops.last() {
                    self.emit(IrInst::Jump(lp.continue_label));
                }
            }
            Stmt::Return(exprs) => {
                let mut ret_vars = Vec::new();
                for e in exprs {
                    ret_vars.push(self.compile_expr(e, None));
                }
                self.emit(IrInst::Return(ret_vars));
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr, None);
            }
        }
    }

    fn compile_assign(&mut self, target: &Expr, val_var: IrVar) {
        match target {
            Expr::Variable(name) => {
                if let Some(local_var) = self.resolve_local(name) {
                    self.emit(IrInst::Move { dst: local_var, src: val_var });
                } else {
                    self.emit(IrInst::SetGlobal { name: name.clone(), src: val_var });
                }
            }
            Expr::Member { object, field } => {
                let tbl = self.compile_expr(object, None);
                let key = self.alloc_var();
                self.emit(IrInst::LoadConst { dst: key, val: IrConstant::String(field.clone()) });
                self.emit(IrInst::SetTable { table: tbl, key, val: val_var });
            }
            Expr::Index { object, index } => {
                let tbl = self.compile_expr(object, None);
                let key = self.compile_expr(index, None);
                self.emit(IrInst::SetTable { table: tbl, key, val: val_var });
            }
            _ => {}
        }
    }
}

pub fn ast_to_ir(stmts: &[Stmt]) -> IrModule {
    let mut builder = IrBuilder::new();
    for stmt in stmts {
        builder.compile_stmt(stmt);
    }
    let has_terminal = matches!(builder.instructions.last(), Some(IrInst::Return(_)));
    if !has_terminal {
        let nil_var = builder.alloc_var();
        builder.emit(IrInst::LoadNil { dst: nil_var });
        builder.emit(IrInst::Return(vec![nil_var]));
    }

    IrModule {
        main: IrFunction {
            name: Some("main".to_string()),
            params: Vec::new(),
            num_params: 0,
            is_vararg: false,
            instructions: builder.instructions,
            protos: builder.child_protos,
        },
    }
}
