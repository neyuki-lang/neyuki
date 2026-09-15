// Bytecode code generator compiling AST into register-based Prototypes.

use num_bigint::BigInt;
use std::str::FromStr;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::{Constant, Proto, UpvalueDesc};
use crate::parser::{Expr, InterpPart, Param, Stmt};

#[derive(Clone, Debug)]
struct LocalVar {
    name: String,
    reg: u8,
    depth: usize,
}

#[derive(Clone, Debug)]
struct LoopContext {
    _start_ip: usize,
    break_jumps: Vec<usize>,
    continue_ips: Vec<usize>,
}

struct FuncState {
    proto: Proto,
    locals: Vec<LocalVar>,
    scope_depth: usize,
    reg_top: u8,
    loops: Vec<LoopContext>,
    current_line: u32,
}

impl FuncState {
    fn new(name: Option<String>, num_params: u8, is_vararg: bool) -> Self {
        Self {
            proto: Proto::new(name, num_params, is_vararg),
            locals: Vec::new(),
            scope_depth: 0,
            reg_top: 0,
            loops: Vec::new(),
            current_line: 1,
        }
    }

    fn alloc_reg(&mut self) -> u8 {
        let r = self.reg_top;
        self.reg_top += 1;
        if self.reg_top > self.proto.max_registers {
            self.proto.max_registers = self.reg_top;
        }
        r
    }

    fn free_reg(&mut self, reg: u8) {
        if reg + 1 == self.reg_top {
            self.reg_top -= 1;
        }
    }

    fn emit(&mut self, inst: Instruction) -> usize {
        self.proto.emit(inst, self.current_line)
    }

    fn add_constant(&mut self, c: Constant) -> u16 {
        self.proto.add_constant(c)
    }

    fn enter_scope(&mut self) {
        self.scope_depth += 1;
    }

    fn exit_scope(&mut self) {
        self.scope_depth -= 1;
        while let Some(local) = self.locals.last() {
            if local.depth > self.scope_depth {
                let reg = local.reg;
                self.free_reg(reg);
                self.locals.pop();
            } else {
                break;
            }
        }
    }

    fn add_local(&mut self, name: String, reg: u8) {
        self.locals.push(LocalVar {
            name,
            reg,
            depth: self.scope_depth,
        });
    }

    fn resolve_local(&self, name: &str) -> Option<u8> {
        for local in self.locals.iter().rev() {
            if local.name == name {
                return Some(local.reg);
            }
        }
        None
    }

    fn add_upvalue(&mut self, in_stack: bool, index: u8) -> u8 {
        for (i, upval) in self.proto.upvalues.iter().enumerate() {
            if upval.in_stack == in_stack && upval.index == index {
                return i as u8;
            }
        }
        let idx = self.proto.upvalues.len() as u8;
        self.proto.upvalues.push(UpvalueDesc { in_stack, index });
        idx
    }

    fn patch_jump(&mut self, jump_ip: usize) {
        let target_ip = self.proto.instructions.len();
        let offset = (target_ip as isize - (jump_ip as isize + 1)) as i16;
        match &mut self.proto.instructions[jump_ip] {
            Instruction::Jump { offset: o } => *o = offset,
            Instruction::Test { jump_if_false: o, .. } => *o = offset,
            Instruction::Eq { jump_if_false: o, .. } => *o = offset,
            Instruction::Ne { jump_if_false: o, .. } => *o = offset,
            Instruction::Lt { jump_if_false: o, .. } => *o = offset,
            Instruction::Le { jump_if_false: o, .. } => *o = offset,
            Instruction::Gt { jump_if_false: o, .. } => *o = offset,
            Instruction::Ge { jump_if_false: o, .. } => *o = offset,
            _ => panic!("attempted to patch non-jump instruction"),
        }
    }

    fn finish(mut self) -> Proto {
        let needs_return = match self.proto.instructions.last() {
            Some(Instruction::Return { .. }) => false,
            _ => true,
        };
        if needs_return {
            let r = self.alloc_reg();
            self.emit(Instruction::LoadNil { dst: r });
            self.emit(Instruction::Return { base: r, count: 1 });
        }
        self.proto
    }
}

pub struct Compiler {
    funcs: Vec<FuncState>,
}

impl Compiler {
    pub fn new(name: Option<String>, num_params: u8, is_vararg: bool) -> Self {
        Self {
            funcs: vec![FuncState::new(name, num_params, is_vararg)],
        }
    }

    fn current(&self) -> &FuncState {
        self.funcs.last().unwrap()
    }

    fn current_mut(&mut self) -> &mut FuncState {
        self.funcs.last_mut().unwrap()
    }

    pub fn finish(mut self) -> Proto {
        self.funcs.pop().unwrap().finish()
    }

    fn resolve_upval_rec(funcs: &mut [FuncState], func_idx: usize, name: &str) -> Option<u8> {
        if func_idx == 0 {
            return None;
        }
        let parent_idx = func_idx - 1;
        if let Some(local_reg) = funcs[parent_idx].resolve_local(name) {
            return Some(funcs[func_idx].add_upvalue(true, local_reg));
        }
        if let Some(parent_upval) = Self::resolve_upval_rec(funcs, parent_idx, name) {
            return Some(funcs[func_idx].add_upvalue(false, parent_upval));
        }
        None
    }

    fn resolve_variable(&mut self, name: &str) -> (bool, Option<u8>) {
        if let Some(local_reg) = self.current().resolve_local(name) {
            return (true, Some(local_reg));
        }
        let func_idx = self.funcs.len() - 1;
        if let Some(upval_idx) = Self::resolve_upval_rec(&mut self.funcs, func_idx, name) {
            return (false, Some(upval_idx));
        }
        (false, None)
    }

    pub fn compile_program(&mut self, statements: &[Stmt]) {
        for stmt in statements {
            self.compile_stmt(stmt);
        }
    }

    pub fn compile_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Local {
                name,
                initializer,
                ..
            } => {
                let reg = self.current_mut().alloc_reg();
                if let Some(init) = initializer {
                    self.compile_expr(init, Some(reg));
                } else {
                    self.current_mut().emit(Instruction::LoadNil { dst: reg });
                }
                self.current_mut().add_local(name.clone(), reg);
            }
            Stmt::LocalMany {
                names,
                initializers,
                ..
            } => {
                if initializers.len() == 1 && matches!(initializers[0], Expr::Call { .. }) {
                    let Expr::Call { callee, args } = &initializers[0] else { unreachable!() };
                    let func_reg = self.current_mut().alloc_reg();
                    self.compile_expr(callee, Some(func_reg));
                    let mut arg_regs = Vec::new();
                    for arg in args {
                        let r = self.current_mut().alloc_reg();
                        self.compile_expr(arg, Some(r));
                        arg_regs.push(r);
                    }
                    let retc = names.len() as u8;
                    for _ in 1..names.len() {
                        self.current_mut().alloc_reg();
                    }
                    self.current_mut().emit(Instruction::Call {
                        callee: func_reg,
                        argc: args.len() as u8,
                        retc,
                    });
                    for r in arg_regs.into_iter().rev() {
                        self.current_mut().free_reg(r);
                    }
                    for (i, name) in names.iter().enumerate() {
                        self.current_mut().add_local(name.clone(), func_reg + i as u8);
                    }
                } else {
                    for (i, name) in names.iter().enumerate() {
                        let reg = self.current_mut().alloc_reg();
                        if let Some(init) = initializers.get(i) {
                            self.compile_expr(init, Some(reg));
                        } else {
                            self.current_mut().emit(Instruction::LoadNil { dst: reg });
                        }
                        self.current_mut().add_local(name.clone(), reg);
                    }
                }
            }
            Stmt::Assign { target, value, .. } => {
                let val_reg = self.compile_expr(value, None);
                self.compile_assign(target, val_reg);
                self.current_mut().free_reg(val_reg);
            }
            Stmt::AssignMany { targets, values } => {
                if values.len() == 1 && matches!(values[0], Expr::Call { .. }) {
                    let Expr::Call { callee, args } = &values[0] else { unreachable!() };
                    let func_reg = self.current_mut().alloc_reg();
                    self.compile_expr(callee, Some(func_reg));
                    let mut arg_regs = Vec::new();
                    for arg in args {
                        let r = self.current_mut().alloc_reg();
                        self.compile_expr(arg, Some(r));
                        arg_regs.push(r);
                    }
                    let retc = targets.len() as u8;
                    while self.current_mut().reg_top < func_reg + retc {
                        self.current_mut().alloc_reg();
                    }
                    self.current_mut().emit(Instruction::Call {
                        callee: func_reg,
                        argc: args.len() as u8,
                        retc,
                    });
                    for r in arg_regs.into_iter().rev() {
                        self.current_mut().free_reg(r);
                    }
                    for (i, target) in targets.iter().enumerate() {
                        self.compile_assign(target, func_reg + i as u8);
                    }
                    self.current_mut().reg_top = func_reg;
                } else if values.len() == 1 && matches!(values[0], Expr::MethodCall { .. }) {
                    let Expr::MethodCall { object, method, args } = &values[0] else { unreachable!() };
                    let func_reg = self.current_mut().alloc_reg();
                    let arg0 = self.current_mut().alloc_reg();
                    self.compile_expr(object, Some(arg0));
                    let key_k = self.current_mut().add_constant(Constant::String(method.clone()));
                    self.current_mut().emit(Instruction::GetTableK {
                        dst: func_reg,
                        table: arg0,
                        key_k,
                    });
                    let mut arg_regs = Vec::new();
                    for arg in args {
                        let r = self.current_mut().alloc_reg();
                        self.compile_expr(arg, Some(r));
                        arg_regs.push(r);
                    }
                    let retc = targets.len() as u8;
                    while self.current_mut().reg_top < func_reg + retc {
                        self.current_mut().alloc_reg();
                    }
                    self.current_mut().emit(Instruction::Call {
                        callee: func_reg,
                        argc: (args.len() + 1) as u8,
                        retc,
                    });
                    for r in arg_regs.into_iter().rev() {
                        self.current_mut().free_reg(r);
                    }
                    self.current_mut().free_reg(arg0);
                    for (i, target) in targets.iter().enumerate() {
                        self.compile_assign(target, func_reg + i as u8);
                    }
                    self.current_mut().reg_top = func_reg;
                } else {
                    let mut val_regs = Vec::new();
                    for val in values {
                        let r = self.compile_expr(val, None);
                        val_regs.push(r);
                    }
                    while val_regs.len() < targets.len() {
                        let r = self.current_mut().alloc_reg();
                        self.current_mut().emit(Instruction::LoadNil { dst: r });
                        val_regs.push(r);
                    }
                    for (target, r) in targets.iter().zip(val_regs.iter()) {
                        self.compile_assign(target, *r);
                    }
                    for r in val_regs.into_iter().rev() {
                        self.current_mut().free_reg(r);
                    }
                }
            }
            Stmt::Increment { target, amount } => {
                let current = self.compile_expr(target, None);
                let amount_reg = self.current_mut().alloc_reg();
                self.current_mut().emit(Instruction::LoadInt {
                    dst: amount_reg,
                    val: *amount as i32,
                });
                let result = self.current_mut().alloc_reg();
                self.current_mut().emit(Instruction::Add {
                    dst: result,
                    a: current,
                    b: amount_reg,
                });
                self.compile_assign(target, result);
                self.current_mut().free_reg(result);
                self.current_mut().free_reg(amount_reg);
                self.current_mut().free_reg(current);
            }
            Stmt::Function {
                name,
                params,
                body,
                ..
            } => {
                let closure_reg = self.current_mut().alloc_reg();
                let proto_idx = self.compile_function(name.clone(), params, body);
                self.current_mut().emit(Instruction::Closure {
                    dst: closure_reg,
                    proto_idx,
                });
                if let Some(func_name) = name {
                    if func_name.contains('.') {
                        let parts: Vec<&str> = func_name.split('.').collect();
                        let mut target = Expr::Variable(parts[0].to_string());
                        for field in &parts[1..parts.len() - 1] {
                            target = Expr::Member {
                                object: Box::new(target),
                                field: field.to_string(),
                            };
                        }
                        let final_target = Expr::Member {
                            object: Box::new(target),
                            field: parts.last().unwrap().to_string(),
                        };
                        self.compile_assign(&final_target, closure_reg);
                    } else {
                        let (is_local, reg_or_upval) = self.resolve_variable(func_name);
                        if is_local {
                            let local_reg = reg_or_upval.unwrap();
                            self.current_mut().emit(Instruction::Move {
                                dst: local_reg,
                                src: closure_reg,
                            });
                        } else if let Some(upval_idx) = reg_or_upval {
                            self.current_mut().emit(Instruction::SetUpval {
                                src: closure_reg,
                                upval_idx,
                            });
                        } else {
                            let name_k = self.current_mut().add_constant(Constant::String(func_name.clone()));
                            self.current_mut().emit(Instruction::SetGlobal {
                                src: closure_reg,
                                name_k,
                            });
                        }
                    }
                }
                self.current_mut().free_reg(closure_reg);
            }
            Stmt::If {
                condition,
                then_branch,
                else_if_branches,
                else_branch,
            } => {
                let mut end_jumps = Vec::new();

                let cond_reg = self.compile_expr(condition, None);
                let false_jump = self.current_mut().emit(Instruction::Test {
                    reg: cond_reg,
                    jump_if_false: 0,
                });
                self.current_mut().free_reg(cond_reg);

                self.current_mut().enter_scope();
                self.compile_program(then_branch);
                self.current_mut().exit_scope();

                end_jumps.push(self.current_mut().emit(Instruction::Jump { offset: 0 }));
                self.current_mut().patch_jump(false_jump);

                for (elif_cond, elif_body) in else_if_branches {
                    let elif_reg = self.compile_expr(elif_cond, None);
                    let elif_false = self.current_mut().emit(Instruction::Test {
                        reg: elif_reg,
                        jump_if_false: 0,
                    });
                    self.current_mut().free_reg(elif_reg);

                    self.current_mut().enter_scope();
                    self.compile_program(elif_body);
                    self.current_mut().exit_scope();

                    end_jumps.push(self.current_mut().emit(Instruction::Jump { offset: 0 }));
                    self.current_mut().patch_jump(elif_false);
                }

                if let Some(else_stmts) = else_branch {
                    self.current_mut().enter_scope();
                    self.compile_program(else_stmts);
                    self.current_mut().exit_scope();
                }

                for j in end_jumps {
                    self.current_mut().patch_jump(j);
                }
            }
            Stmt::While { condition, body } => {
                let loop_start = self.current().proto.instructions.len();
                self.current_mut().loops.push(LoopContext {
                    _start_ip: loop_start,
                    break_jumps: Vec::new(),
                    continue_ips: Vec::new(),
                });

                let cond_reg = self.compile_expr(condition, None);
                let exit_jump = self.current_mut().emit(Instruction::Test {
                    reg: cond_reg,
                    jump_if_false: 0,
                });
                self.current_mut().free_reg(cond_reg);

                self.current_mut().enter_scope();
                self.compile_program(body);
                self.current_mut().exit_scope();

                let back_offset = (loop_start as isize - (self.current().proto.instructions.len() as isize + 1)) as i16;
                self.current_mut().emit(Instruction::Jump { offset: back_offset });

                self.current_mut().patch_jump(exit_jump);

                let loop_ctx = self.current_mut().loops.pop().unwrap();
                for b in loop_ctx.break_jumps {
                    self.current_mut().patch_jump(b);
                }
                for c in loop_ctx.continue_ips {
                    let off = (loop_start as isize - (c as isize + 1)) as i16;
                    if let Instruction::Jump { offset } = &mut self.current_mut().proto.instructions[c] {
                        *offset = off;
                    }
                }
            }
            Stmt::Repeat { body, condition } => {
                let loop_start = self.current().proto.instructions.len();
                self.current_mut().loops.push(LoopContext {
                    _start_ip: loop_start,
                    break_jumps: Vec::new(),
                    continue_ips: Vec::new(),
                });

                self.current_mut().enter_scope();
                self.compile_program(body);
                self.current_mut().exit_scope();

                let cond_reg = self.compile_expr(condition, None);
                let exit_jump = self.current_mut().emit(Instruction::Test {
                    reg: cond_reg,
                    jump_if_false: 0,
                });
                self.current_mut().free_reg(cond_reg);

                let back_offset = (loop_start as isize - (self.current().proto.instructions.len() as isize + 1)) as i16;
                let repeat_jump = self.current_mut().emit(Instruction::Jump { offset: back_offset });
                let _ = repeat_jump;

                self.current_mut().patch_jump(exit_jump);

                let loop_ctx = self.current_mut().loops.pop().unwrap();
                for b in loop_ctx.break_jumps {
                    self.current_mut().patch_jump(b);
                }
            }
            Stmt::NumericFor {
                var,
                start,
                end,
                step,
                body,
            } => {
                self.current_mut().enter_scope();
                let var_reg = self.current_mut().alloc_reg();
                self.compile_expr(start, Some(var_reg));
                self.current_mut().add_local(var.clone(), var_reg);

                let end_reg = self.compile_expr(end, None);
                let step_reg = if let Some(step_expr) = step {
                    self.compile_expr(step_expr, None)
                } else {
                    let r = self.current_mut().alloc_reg();
                    self.current_mut().emit(Instruction::LoadInt { dst: r, val: 1 });
                    r
                };

                let loop_start = self.current().proto.instructions.len();
                self.current_mut().loops.push(LoopContext {
                    _start_ip: loop_start,
                    break_jumps: Vec::new(),
                    continue_ips: Vec::new(),
                });

                let check_jump = self.current_mut().emit(Instruction::Le {
                    a: var_reg,
                    b: end_reg,
                    jump_if_false: 0,
                });

                self.current_mut().enter_scope();
                self.compile_program(body);
                self.current_mut().exit_scope();

                self.current_mut().emit(Instruction::Add {
                    dst: var_reg,
                    a: var_reg,
                    b: step_reg,
                });

                let back_offset = (loop_start as isize - (self.current().proto.instructions.len() as isize + 1)) as i16;
                self.current_mut().emit(Instruction::Jump { offset: back_offset });

                self.current_mut().patch_jump(check_jump);

                let loop_ctx = self.current_mut().loops.pop().unwrap();
                for b in loop_ctx.break_jumps {
                    self.current_mut().patch_jump(b);
                }
                self.current_mut().free_reg(step_reg);
                self.current_mut().free_reg(end_reg);
                self.current_mut().exit_scope();
            }
            Stmt::For { vars, source, body } => {
                self.current_mut().enter_scope();
                let source_reg = self.compile_expr(source, None);
                let idx_reg = self.current_mut().alloc_reg();
                self.current_mut().emit(Instruction::LoadInt { dst: idx_reg, val: 1 });

                let loop_start = self.current().proto.instructions.len();
                self.current_mut().loops.push(LoopContext {
                    _start_ip: loop_start,
                    break_jumps: Vec::new(),
                    continue_ips: Vec::new(),
                });

                let val_reg = self.current_mut().alloc_reg();
                self.current_mut().emit(Instruction::GetTable {
                    dst: val_reg,
                    table: source_reg,
                    key: idx_reg,
                });

                let nil_reg = self.current_mut().alloc_reg();
                self.current_mut().emit(Instruction::LoadNil { dst: nil_reg });
                let exit_jump = self.current_mut().emit(Instruction::Ne {
                    a: val_reg,
                    b: nil_reg,
                    jump_if_false: 0,
                });
                self.current_mut().free_reg(nil_reg);

                self.current_mut().enter_scope();
                if let Some(first_var) = vars.first() {
                    if vars.len() == 1 {
                        self.current_mut().add_local(first_var.clone(), val_reg);
                    } else {
                        self.current_mut().add_local(first_var.clone(), idx_reg);
                        if let Some(second_var) = vars.get(1) {
                            self.current_mut().add_local(second_var.clone(), val_reg);
                        }
                    }
                }
                self.compile_program(body);
                self.current_mut().exit_scope();

                let one = self.current_mut().alloc_reg();
                self.current_mut().emit(Instruction::LoadInt { dst: one, val: 1 });
                self.current_mut().emit(Instruction::Add {
                    dst: idx_reg,
                    a: idx_reg,
                    b: one,
                });
                self.current_mut().free_reg(one);

                let back_offset = (loop_start as isize - (self.current().proto.instructions.len() as isize + 1)) as i16;
                self.current_mut().emit(Instruction::Jump { offset: back_offset });

                self.current_mut().patch_jump(exit_jump);

                let loop_ctx = self.current_mut().loops.pop().unwrap();
                for b in loop_ctx.break_jumps {
                    self.current_mut().patch_jump(b);
                }
                self.current_mut().free_reg(val_reg);
                self.current_mut().free_reg(idx_reg);
                self.current_mut().free_reg(source_reg);
                self.current_mut().exit_scope();
            }
            Stmt::Return(exprs) => {
                if exprs.is_empty() {
                    let r = self.current_mut().alloc_reg();
                    self.current_mut().emit(Instruction::LoadNil { dst: r });
                    self.current_mut().emit(Instruction::Return { base: r, count: 1 });
                    self.current_mut().free_reg(r);
                } else if exprs.len() == 1 {
                    let r = self.compile_expr(&exprs[0], None);
                    self.current_mut().emit(Instruction::Return {
                        base: r,
                        count: 1,
                    });
                    self.current_mut().free_reg(r);
                } else {
                    let base = self.current_mut().alloc_reg();
                    self.compile_expr(&exprs[0], Some(base));
                    for expr in &exprs[1..] {
                        let next_r = self.current_mut().alloc_reg();
                        self.compile_expr(expr, Some(next_r));
                    }
                    self.current_mut().emit(Instruction::Return {
                        base,
                        count: exprs.len() as u8,
                    });
                }
            }
            Stmt::Break => {
                let jump_ip = self.current_mut().emit(Instruction::Jump { offset: 0 });
                if let Some(lp) = self.current_mut().loops.last_mut() {
                    lp.break_jumps.push(jump_ip);
                }
            }
            Stmt::Continue => {
                let jump_ip = self.current_mut().emit(Instruction::Jump { offset: 0 });
                if let Some(lp) = self.current_mut().loops.last_mut() {
                    lp.continue_ips.push(jump_ip);
                }
            }
            Stmt::Expr(expr) => {
                let r = self.compile_expr(expr, None);
                self.current_mut().free_reg(r);
            }
        }
    }

    fn compile_assign(&mut self, target: &Expr, val_reg: u8) {
        match target {
            Expr::Variable(name) => {
                let (is_local, reg_or_upval) = self.resolve_variable(name);
                if is_local {
                    let local_reg = reg_or_upval.unwrap();
                    self.current_mut().emit(Instruction::Move {
                        dst: local_reg,
                        src: val_reg,
                    });
                } else if let Some(upval_idx) = reg_or_upval {
                    self.current_mut().emit(Instruction::SetUpval {
                        src: val_reg,
                        upval_idx,
                    });
                } else {
                    let name_k = self.current_mut().add_constant(Constant::String(name.clone()));
                    self.current_mut().emit(Instruction::SetGlobal {
                        src: val_reg,
                        name_k,
                    });
                }
            }
            Expr::Member { object, field } => {
                let obj_reg = self.compile_expr(object, None);
                let key_k = self.current_mut().add_constant(Constant::String(field.clone()));
                self.current_mut().emit(Instruction::SetTableK {
                    table: obj_reg,
                    key_k,
                    val: val_reg,
                });
                self.current_mut().free_reg(obj_reg);
            }
            Expr::Index { object, index } => {
                let obj_reg = self.compile_expr(object, None);
                let idx_reg = self.compile_expr(index, None);
                self.current_mut().emit(Instruction::SetTable {
                    table: obj_reg,
                    key: idx_reg,
                    val: val_reg,
                });
                self.current_mut().free_reg(idx_reg);
                self.current_mut().free_reg(obj_reg);
            }
            _ => panic!("invalid assignment target in bytecode compiler"),
        }
    }

    pub fn compile_expr(&mut self, expr: &Expr, target: Option<u8>) -> u8 {
        let dst = target.unwrap_or_else(|| self.current_mut().alloc_reg());

        match expr {
            Expr::Literal(val) => {
                match val.as_str() {
                    "nil" => {
                        self.current_mut().emit(Instruction::LoadNil { dst });
                    }
                    "true" => {
                        self.current_mut().emit(Instruction::LoadBool { dst, val: true });
                    }
                    "false" => {
                        self.current_mut().emit(Instruction::LoadBool { dst, val: false });
                    }
                    s => {
                        if let Ok(i) = s.parse::<i32>() {
                            self.current_mut().emit(Instruction::LoadInt { dst, val: i });
                        } else if let Ok(bi) = BigInt::from_str(s) {
                            let k = self.current_mut().add_constant(Constant::Int(bi));
                            self.current_mut().emit(Instruction::LoadK { dst, k });
                        } else if let Ok(f) = s.parse::<f64>() {
                            let k = self.current_mut().add_constant(Constant::Float(f));
                            self.current_mut().emit(Instruction::LoadK { dst, k });
                        } else {
                            let k = self.current_mut().add_constant(Constant::String(s.to_string()));
                            self.current_mut().emit(Instruction::LoadK { dst, k });
                        }
                    }
                }
            }
            Expr::Str(s) => {
                let k = self.current_mut().add_constant(Constant::String(s.clone()));
                self.current_mut().emit(Instruction::LoadK { dst, k });
            }
            Expr::Interp(parts) => {
                if parts.is_empty() {
                    let k = self.current_mut().add_constant(Constant::String(String::new()));
                    self.current_mut().emit(Instruction::LoadK { dst, k });
                } else if parts.iter().all(|p| matches!(p, InterpPart::Literal(_))) {
                    let mut s = String::new();
                    for p in parts {
                        if let InterpPart::Literal(lit) = p {
                            s.push_str(lit);
                        }
                    }
                    let k = self.current_mut().add_constant(Constant::String(s));
                    self.current_mut().emit(Instruction::LoadK { dst, k });
                } else {
                    let mut cur_reg = None;
                    for part in parts {
                        let part_reg = match part {
                            InterpPart::Literal(lit) => {
                                let r = self.current_mut().alloc_reg();
                                let k = self.current_mut().add_constant(Constant::String(lit.clone()));
                                self.current_mut().emit(Instruction::LoadK { dst: r, k });
                                r
                            }
                            InterpPart::Expr(expr) => {
                                self.compile_expr(expr, None)
                            }
                        };
                        if let Some(prev) = cur_reg {
                            let next = self.current_mut().alloc_reg();
                            self.current_mut().emit(Instruction::Concat { dst: next, a: prev, b: part_reg });
                            self.current_mut().free_reg(part_reg);
                            self.current_mut().free_reg(prev);
                            cur_reg = Some(next);
                        } else if parts.len() == 1 {
                            let empty_reg = self.current_mut().alloc_reg();
                            let k = self.current_mut().add_constant(Constant::String(String::new()));
                            self.current_mut().emit(Instruction::LoadK { dst: empty_reg, k });
                            let next = self.current_mut().alloc_reg();
                            self.current_mut().emit(Instruction::Concat { dst: next, a: empty_reg, b: part_reg });
                            self.current_mut().free_reg(empty_reg);
                            self.current_mut().free_reg(part_reg);
                            cur_reg = Some(next);
                        } else {
                            cur_reg = Some(part_reg);
                        }
                    }
                    if let Some(res) = cur_reg {
                        if res != dst {
                            self.current_mut().emit(Instruction::Move { dst, src: res });
                        }
                        self.current_mut().free_reg(res);
                    }
                }
            }
            Expr::Variable(name) => {
                let (is_local, reg_or_upval) = self.resolve_variable(name);
                if is_local {
                    let local_reg = reg_or_upval.unwrap();
                    if local_reg != dst {
                        self.current_mut().emit(Instruction::Move {
                            dst,
                            src: local_reg,
                        });
                    }
                } else if let Some(upval_idx) = reg_or_upval {
                    self.current_mut().emit(Instruction::GetUpval { dst, upval_idx });
                } else {
                    let name_k = self.current_mut().add_constant(Constant::String(name.clone()));
                    self.current_mut().emit(Instruction::GetGlobal { dst, name_k });
                }
            }
            Expr::Vararg => {
                self.current_mut().emit(Instruction::Vararg { dst, count: 1 });
            }
            Expr::Member { object, field } => {
                let obj_reg = self.compile_expr(object, None);
                let key_k = self.current_mut().add_constant(Constant::String(field.clone()));
                self.current_mut().emit(Instruction::GetTableK {
                    dst,
                    table: obj_reg,
                    key_k,
                });
                self.current_mut().free_reg(obj_reg);
            }
            Expr::Index { object, index } => {
                let obj_reg = self.compile_expr(object, None);
                let idx_reg = self.compile_expr(index, None);
                self.current_mut().emit(Instruction::GetTable {
                    dst,
                    table: obj_reg,
                    key: idx_reg,
                });
                self.current_mut().free_reg(idx_reg);
                self.current_mut().free_reg(obj_reg);
            }
            Expr::Table(entries) => {
                self.current_mut().emit(Instruction::NewTable { dst });
                for entry in entries {
                    let val_reg = self.compile_expr(&entry.value, None);
                    if let Some(key) = &entry.key {
                        let key_k = self.current_mut().add_constant(Constant::String(key.clone()));
                        self.current_mut().emit(Instruction::SetTableK {
                            table: dst,
                            key_k,
                            val: val_reg,
                        });
                    } else {
                        self.current_mut().emit(Instruction::AppendArray {
                            table: dst,
                            src: val_reg,
                        });
                    }
                    self.current_mut().free_reg(val_reg);
                }
            }
            Expr::Unary { op, expr } => {
                let src = self.compile_expr(expr, None);
                match op.as_str() {
                    "-" => self.current_mut().emit(Instruction::Unm { dst, src }),
                    "not" => self.current_mut().emit(Instruction::Not { dst, src }),
                    "#" => self.current_mut().emit(Instruction::Len { dst, src }),
                    "~" => self.current_mut().emit(Instruction::BitNot { dst, src }),
                    _ => panic!("unsupported unary operator {}", op),
                };
                self.current_mut().free_reg(src);
            }
            Expr::Binary { left, op, right } => {
                match op.as_str() {
                    "and" => {
                        self.compile_expr(left, Some(dst));
                        let false_jump = self.current_mut().emit(Instruction::Test {
                            reg: dst,
                            jump_if_false: 0,
                        });
                        self.compile_expr(right, Some(dst));
                        self.current_mut().patch_jump(false_jump);
                    }
                    "or" => {
                        self.compile_expr(left, Some(dst));
                        let false_jump = self.current_mut().emit(Instruction::Test {
                            reg: dst,
                            jump_if_false: 0,
                        });
                        let end_jump = self.current_mut().emit(Instruction::Jump { offset: 0 });
                        self.current_mut().patch_jump(false_jump);
                        self.compile_expr(right, Some(dst));
                        self.current_mut().patch_jump(end_jump);
                    }
                    "??" => {
                        let a = self.compile_expr(left, None);
                        let b = self.compile_expr(right, None);
                        self.current_mut().emit(Instruction::Coalesce { dst, a, b });
                        self.current_mut().free_reg(b);
                        self.current_mut().free_reg(a);
                    }
                    "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                        let a = self.compile_expr(left, None);
                        let b = self.compile_expr(right, None);
                        let false_jump = match op.as_str() {
                            "==" => self.current_mut().emit(Instruction::Eq { a, b, jump_if_false: 0 }),
                            "!=" => self.current_mut().emit(Instruction::Ne { a, b, jump_if_false: 0 }),
                            "<" => self.current_mut().emit(Instruction::Lt { a, b, jump_if_false: 0 }),
                            "<=" => self.current_mut().emit(Instruction::Le { a, b, jump_if_false: 0 }),
                            ">" => self.current_mut().emit(Instruction::Gt { a, b, jump_if_false: 0 }),
                            ">=" => self.current_mut().emit(Instruction::Ge { a, b, jump_if_false: 0 }),
                            _ => unreachable!(),
                        };
                        self.current_mut().free_reg(b);
                        self.current_mut().free_reg(a);
                        self.current_mut().emit(Instruction::LoadBool { dst, val: true });
                        let skip = self.current_mut().emit(Instruction::Jump { offset: 1 });
                        self.current_mut().patch_jump(false_jump);
                        self.current_mut().emit(Instruction::LoadBool { dst, val: false });
                        self.current_mut().patch_jump(skip);
                    }
                    _ => {
                        let a = self.compile_expr(left, None);
                        let b = self.compile_expr(right, None);
                        match op.as_str() {
                            "+" => self.current_mut().emit(Instruction::Add { dst, a, b }),
                            "-" => self.current_mut().emit(Instruction::Sub { dst, a, b }),
                            "*" => self.current_mut().emit(Instruction::Mul { dst, a, b }),
                            "/" => self.current_mut().emit(Instruction::Div { dst, a, b }),
                            "//" => self.current_mut().emit(Instruction::IDiv { dst, a, b }),
                            "%" => self.current_mut().emit(Instruction::Mod { dst, a, b }),
                            "^" => self.current_mut().emit(Instruction::Pow { dst, a, b }),
                            "&" => self.current_mut().emit(Instruction::BitAnd { dst, a, b }),
                            "|" => self.current_mut().emit(Instruction::BitOr { dst, a, b }),
                            "~" => self.current_mut().emit(Instruction::BitXor { dst, a, b }),
                            "<<" => self.current_mut().emit(Instruction::Shl { dst, a, b }),
                            ">>" => self.current_mut().emit(Instruction::Shr { dst, a, b }),
                            ".." => self.current_mut().emit(Instruction::Concat { dst, a, b }),
                            _ => panic!("unsupported binary operator {}", op),
                        };
                        self.current_mut().free_reg(b);
                        self.current_mut().free_reg(a);
                    }
                }
            }
            Expr::Call { callee, args } => {
                let func_reg = self.current_mut().alloc_reg();
                self.compile_expr(callee, Some(func_reg));
                let mut arg_regs = Vec::new();
                for arg in args {
                    let r = self.current_mut().alloc_reg();
                    self.compile_expr(arg, Some(r));
                    arg_regs.push(r);
                }
                self.current_mut().emit(Instruction::Call {
                    callee: func_reg,
                    argc: args.len() as u8,
                    retc: 1,
                });
                if func_reg != dst {
                    self.current_mut().emit(Instruction::Move {
                        dst,
                        src: func_reg,
                    });
                }
                for r in arg_regs.into_iter().rev() {
                    self.current_mut().free_reg(r);
                }
                self.current_mut().free_reg(func_reg);
            }
            Expr::MethodCall { object, method, args } => {
                let func_reg = self.current_mut().alloc_reg();
                let arg0 = self.current_mut().alloc_reg();
                self.compile_expr(object, Some(arg0));
                let key_k = self.current_mut().add_constant(Constant::String(method.clone()));
                self.current_mut().emit(Instruction::GetTableK {
                    dst: func_reg,
                    table: arg0,
                    key_k,
                });
                let mut arg_regs = Vec::new();
                for arg in args {
                    let r = self.current_mut().alloc_reg();
                    self.compile_expr(arg, Some(r));
                    arg_regs.push(r);
                }
                self.current_mut().emit(Instruction::Call {
                    callee: func_reg,
                    argc: (args.len() + 1) as u8,
                    retc: 1,
                });
                if func_reg != dst {
                    self.current_mut().emit(Instruction::Move {
                        dst,
                        src: func_reg,
                    });
                }
                for r in arg_regs.into_iter().rev() {
                    self.current_mut().free_reg(r);
                }
                self.current_mut().free_reg(arg0);
                self.current_mut().free_reg(func_reg);
            }
            Expr::Function { params, body } => {
                let proto_idx = self.compile_function(None, params, body);
                self.current_mut().emit(Instruction::Closure {
                    dst,
                    proto_idx,
                });
            }
        }

        dst
    }

    fn compile_function(
        &mut self,
        name: Option<String>,
        params: &[Param],
        body: &[Stmt],
    ) -> u16 {
        let is_vararg = params.iter().any(|p| p.variadic);
        let num_params = params.iter().filter(|p| !p.variadic).count() as u8;

        self.funcs.push(FuncState::new(name, num_params, is_vararg));

        for param in params {
            if !param.variadic {
                let reg = self.current_mut().alloc_reg();
                self.current_mut().add_local(param.name.clone(), reg);
            }
        }

        self.compile_program(body);
        let child_proto = self.funcs.pop().unwrap().finish();

        let parent = self.current_mut();
        let idx = parent.proto.protos.len() as u16;
        parent.proto.protos.push(child_proto);
        idx
    }
}
