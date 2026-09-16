// Statement and assignment bytecode code generator.

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::Constant;
use crate::parser::{Expr, Stmt};
use super::Compiler;
use super::state::LoopContext;

impl Compiler {
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
                    let key_k = self.add_constant(Constant::String(method.clone()));
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
                            let name_k = self.add_constant(Constant::String(func_name.clone()));
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
                self.patch_jump(false_jump);

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
                    self.patch_jump(elif_false);
                }

                if let Some(else_stmts) = else_branch {
                    self.current_mut().enter_scope();
                    self.compile_program(else_stmts);
                    self.current_mut().exit_scope();
                }

                for j in end_jumps {
                    self.patch_jump(j);
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

                self.patch_jump(exit_jump);

                let loop_ctx = self.current_mut().loops.pop().unwrap();
                for b in loop_ctx.break_jumps {
                    self.patch_jump(b);
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

                self.patch_jump(exit_jump);

                let loop_ctx = self.current_mut().loops.pop().unwrap();
                for b in loop_ctx.break_jumps {
                    self.patch_jump(b);
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
                let base = self.current_mut().alloc_reg();
                self.compile_expr(start, Some(base));
                let limit_reg = self.current_mut().alloc_reg();
                self.compile_expr(end, Some(limit_reg));
                let step_reg = self.current_mut().alloc_reg();
                if let Some(step_expr) = step {
                    self.compile_expr(step_expr, Some(step_reg));
                } else {
                    self.current_mut().emit(Instruction::LoadInt { dst: step_reg, val: 1 });
                }
                let var_reg = self.current_mut().alloc_reg();
                self.current_mut().add_local(var.clone(), var_reg);

                let prep_ip = self.current_mut().emit(Instruction::ForPrep { base, jump: 0 });

                let body_start = self.current().proto.instructions.len();
                self.current_mut().loops.push(LoopContext {
                    _start_ip: body_start,
                    break_jumps: Vec::new(),
                    continue_ips: Vec::new(),
                });

                self.current_mut().enter_scope();
                self.compile_program(body);
                self.current_mut().exit_scope();

                let loop_ip = self.current().proto.instructions.len();
                let back_offset = (body_start as isize - (loop_ip as isize + 1)) as i16;
                self.current_mut().emit(Instruction::ForLoop { base, jump: back_offset });

                let prep_offset = (loop_ip as isize - (prep_ip as isize + 1)) as i16;
                if let Instruction::ForPrep { jump, .. } = &mut self.current_mut().proto.instructions[prep_ip] {
                    *jump = prep_offset;
                }

                let loop_ctx = self.current_mut().loops.pop().unwrap();
                for b in loop_ctx.break_jumps {
                    self.patch_jump(b);
                }
                for c_ip in loop_ctx.continue_ips {
                    let offset = (loop_ip as isize - (c_ip as isize + 1)) as i16;
                    if let Instruction::Jump { offset: o } = &mut self.current_mut().proto.instructions[c_ip] {
                        *o = offset;
                    }
                }

                self.current_mut().exit_scope();
            }
            Stmt::For { vars, source, body } => {
                self.current_mut().enter_scope();
                // Allocate 3 internal registers: iter_fn, state, ctrl
                // These must be consecutive starting at base
                let base = self.alloc_reg();
                let state_reg = self.alloc_reg();
                let ctrl_reg = self.alloc_reg();

                // Compile the source expression.
                // If it's a call (like pairs(t) or ipairs(t)), we need 3 return values.
                // We compile the call with retc=3 into base, state_reg, ctrl_reg.
                match source {
                    Expr::Call { callee, args } => {
                        let func_reg = base;
                        self.compile_expr(callee, Some(func_reg));
                        // Push extra regs for state+ctrl before call
                        let mut arg_regs = Vec::new();
                        for arg in args {
                            let r = self.alloc_reg();
                            self.compile_expr(arg, Some(r));
                            arg_regs.push(r);
                        }
                        self.current_mut().emit(Instruction::Call {
                            callee: func_reg,
                            argc: args.len() as u8,
                            retc: 3,
                        });
                        // After Call with retc=3: R(func_reg)=iter, R(func_reg+1)=state, R(func_reg+2)=ctrl
                        for r in arg_regs.into_iter().rev() {
                            self.current_mut().free_reg(r);
                        }
                    }
                    _ => {
                        // Single expr: iter=source, state=nil, ctrl=nil
                        self.compile_expr(source, Some(base));
                        self.current_mut().emit(Instruction::LoadNil { dst: state_reg });
                        self.current_mut().emit(Instruction::LoadNil { dst: ctrl_reg });
                    }
                }

                // Variable registers: R(base+3), R(base+3+1), ...
                let retc = vars.len() as u8;
                let var_base = base + 3;
                for _i in 0..(retc as usize) {
                    let vr = self.alloc_reg();
                    let _ = vr; // provisioned
                }

                let tfor_call_ip = self.current().proto.instructions.len();

                self.current_mut().loops.push(LoopContext {
                    _start_ip: tfor_call_ip,
                    break_jumps: Vec::new(),
                    continue_ips: Vec::new(),
                });

                // TForCall: call R(base)(R(base+1), R(base+2)) -> R(base+3)..
                self.current_mut().emit(Instruction::TForCall { base, retc });

                // TForLoop: if R(base+3) == nil, jump forward past body; else R(base+2) = R(base+3)
                let tfor_loop_ip = self.current_mut().emit(Instruction::TForLoop { base, jump: 0 });

                // Bind vars to the result registers
                self.current_mut().enter_scope();
                for (i, var_name) in vars.iter().enumerate() {
                    self.current_mut().add_local(var_name.clone(), var_base + i as u8);
                }

                self.compile_program(body);
                self.current_mut().exit_scope();

                // Back jump to TForCall
                let back_offset = (tfor_call_ip as isize - (self.current().proto.instructions.len() as isize + 1)) as i16;
                self.current_mut().emit(Instruction::Jump { offset: back_offset });

                // Patch TForLoop jump to point past the back-jump (i.e., current ip = loop exit)
                let exit_ip = self.current().proto.instructions.len();
                let tfor_loop_fwd = (exit_ip as isize - (tfor_loop_ip as isize + 1)) as i16;
                if let Instruction::TForLoop { jump, .. } = &mut self.current_mut().proto.instructions[tfor_loop_ip] {
                    *jump = tfor_loop_fwd;
                }

                let loop_ctx = self.current_mut().loops.pop().unwrap();
                for b in loop_ctx.break_jumps {
                    self.patch_jump(b);
                }
                for c_ip in loop_ctx.continue_ips {
                    let offset = (tfor_call_ip as isize - (c_ip as isize + 1)) as i16;
                    if let Instruction::Jump { offset: o } = &mut self.current_mut().proto.instructions[c_ip] {
                        *o = offset;
                    }
                }

                // Free var registers
                for _ in 0..(retc as usize) {
                    self.current_mut().reg_top = self.current().reg_top.saturating_sub(1);
                }
                self.current_mut().free_reg(ctrl_reg);
                self.current_mut().free_reg(state_reg);
                self.current_mut().free_reg(base);
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

    pub(crate) fn compile_assign(&mut self, target: &Expr, val_reg: u8) {
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
                    let name_k = self.add_constant(Constant::String(name.clone()));
                    self.current_mut().emit(Instruction::SetGlobal {
                        src: val_reg,
                        name_k,
                    });
                }
            }
            Expr::Member { object, field } => {
                let obj_reg = self.compile_expr(object, None);
                let key_k = self.add_constant(Constant::String(field.clone()));
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
            _ => {
                self.emit_error("invalid assignment target in bytecode compiler");
            }
        }
    }
}
