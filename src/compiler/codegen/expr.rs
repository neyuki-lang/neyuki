// Expression and function prototype bytecode code generator.

use num_traits::ToPrimitive;

use super::Compiler;
use super::state::FuncState;
use crate::ast::op::{BinOp, UnOp};
use crate::bytecode::instruction::{Instruction, MULTRET};
use crate::bytecode::proto::Constant;
use crate::parser::{Expr, InterpPart, Param, Stmt};

impl Compiler {
    pub fn compile_expr(&mut self, expr: &Expr, target: Option<u8>) -> u8 {
        let dst = target.unwrap_or_else(|| self.current_mut().alloc_reg());

        match expr {
            Expr::Literal { value: lit, .. } => match lit {
                crate::ast::Literal::Nil => {
                    self.current_mut().emit(Instruction::LoadNil { dst });
                }
                crate::ast::Literal::Bool(val) => {
                    self.current_mut()
                        .emit(Instruction::LoadBool { dst, val: *val });
                }
                crate::ast::Literal::Int(bi) => {
                    if let Some(i) = bi.to_i32() {
                        self.current_mut()
                            .emit(Instruction::LoadInt { dst, val: i });
                    } else {
                        let k = self.add_constant(Constant::Int(bi.clone()));
                        self.current_mut().emit(Instruction::LoadK { dst, k });
                    }
                }
                crate::ast::Literal::Float(f) => {
                    let k = self.add_constant(Constant::Float(*f));
                    self.current_mut().emit(Instruction::LoadK { dst, k });
                }
                crate::ast::Literal::String(s) => {
                    let k = self.add_constant(Constant::String(s.clone()));
                    self.current_mut().emit(Instruction::LoadK { dst, k });
                }
            },
            Expr::Interp { parts, .. } => {
                if parts.is_empty() {
                    let k = self.add_constant(Constant::String(String::new()));
                    self.current_mut().emit(Instruction::LoadK { dst, k });
                } else if parts.iter().all(|p| matches!(p, InterpPart::Literal(_))) {
                    let mut s = String::new();
                    for p in parts {
                        if let InterpPart::Literal(lit) = p {
                            s.push_str(lit);
                        }
                    }
                    let k = self.add_constant(Constant::String(s));
                    self.current_mut().emit(Instruction::LoadK { dst, k });
                } else {
                    let mut cur_reg = None;
                    for part in parts {
                        let part_reg = match part {
                            InterpPart::Literal(lit) => {
                                let r = self.current_mut().alloc_reg();
                                let k = self.add_constant(Constant::String(lit.clone()));
                                self.current_mut().emit(Instruction::LoadK { dst: r, k });
                                r
                            }
                            InterpPart::Expr(expr) => self.compile_expr(expr, None),
                        };
                        if let Some(prev) = cur_reg {
                            let next = self.current_mut().alloc_reg();
                            self.current_mut().emit(Instruction::Concat {
                                dst: next,
                                a: prev,
                                b: part_reg,
                            });
                            self.current_mut().free_reg(part_reg);
                            self.current_mut().free_reg(prev);
                            cur_reg = Some(next);
                        } else if parts.len() == 1 {
                            let empty_reg = self.current_mut().alloc_reg();
                            let k = self.add_constant(Constant::String(String::new()));
                            self.current_mut()
                                .emit(Instruction::LoadK { dst: empty_reg, k });
                            let next = self.current_mut().alloc_reg();
                            self.current_mut().emit(Instruction::Concat {
                                dst: next,
                                a: empty_reg,
                                b: part_reg,
                            });
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
            Expr::Variable { name, .. } => {
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
                    self.current_mut()
                        .emit(Instruction::GetUpval { dst, upval_idx });
                } else {
                    let name_k = self.add_constant(Constant::String(name.clone()));
                    self.current_mut()
                        .emit(Instruction::GetGlobal { dst, name_k });
                }
            }
            Expr::Vararg { .. } => {
                self.current_mut()
                    .emit(Instruction::Vararg { dst, count: 1 });
            }
            Expr::Member { object, field, .. } => {
                let obj_reg = self.compile_expr(object, None);
                let key_k = self.add_constant(Constant::String(field.clone()));
                self.current_mut().emit(Instruction::GetTableK {
                    dst,
                    table: obj_reg,
                    key_k,
                });
                self.current_mut().free_reg(obj_reg);
            }
            Expr::Index { object, index, .. } => {
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
            Expr::Table { entries, .. } => {
                self.current_mut().emit(Instruction::NewTable { dst });
                let mut array_regs = Vec::new();
                for (index, entry) in entries.iter().enumerate() {
                    let is_last = index + 1 == entries.len();
                    if let Some(key) = &entry.key {
                        if !array_regs.is_empty() {
                            let base = array_regs[0];
                            let count = array_regs.len() as u8;
                            self.current_mut().emit(Instruction::SetList {
                                table: dst,
                                base,
                                count,
                            });
                            for r in array_regs.drain(..).rev() {
                                self.current_mut().free_reg(r);
                            }
                        }
                        let val_reg = self.compile_expr(&entry.value, None);
                        let key_k = self.add_constant(Constant::String(key.clone()));
                        self.current_mut().emit(Instruction::SetTableK {
                            table: dst,
                            key_k,
                            val: val_reg,
                        });
                        self.current_mut().free_reg(val_reg);
                    } else if is_last && Compiler::is_multi_value(&entry.value) {
                        if !array_regs.is_empty() {
                            let base = array_regs[0];
                            let count = array_regs.len() as u8;
                            self.current_mut().emit(Instruction::SetList {
                                table: dst,
                                base,
                                count,
                            });
                            for r in array_regs.drain(..).rev() {
                                self.current_mut().free_reg(r);
                            }
                        }
                        let val_reg = self.current_mut().alloc_reg();
                        self.compile_expr_multi(&entry.value, val_reg);
                        self.current_mut().emit(Instruction::SetList {
                            table: dst,
                            base: val_reg,
                            count: MULTRET,
                        });
                        self.current_mut().free_reg(val_reg);
                    } else {
                        let val_reg = self.compile_expr(&entry.value, None);
                        array_regs.push(val_reg);
                        if array_regs.len() >= 32 {
                            let base = array_regs[0];
                            let count = array_regs.len() as u8;
                            self.current_mut().emit(Instruction::SetList {
                                table: dst,
                                base,
                                count,
                            });
                            for r in array_regs.drain(..).rev() {
                                self.current_mut().free_reg(r);
                            }
                        }
                    }
                }
                if !array_regs.is_empty() {
                    let base = array_regs[0];
                    let count = array_regs.len() as u8;
                    self.current_mut().emit(Instruction::SetList {
                        table: dst,
                        base,
                        count,
                    });
                    for r in array_regs.drain(..).rev() {
                        self.current_mut().free_reg(r);
                    }
                }
            }
            Expr::Unary { op, expr, .. } => {
                let src = self.compile_expr(expr, None);
                match op {
                    UnOp::Neg => self.current_mut().emit(Instruction::Unm { dst, src }),
                    UnOp::Not => self.current_mut().emit(Instruction::Not { dst, src }),
                    UnOp::Len => self.current_mut().emit(Instruction::Len { dst, src }),
                    UnOp::BitNot => self.current_mut().emit(Instruction::BitNot { dst, src }),
                };
                self.current_mut().free_reg(src);
            }
            Expr::Binary {
                left, op, right, ..
            } => match op {
                BinOp::And => {
                    self.compile_expr(left, Some(dst));
                    let false_jump = self.current_mut().emit(Instruction::Test {
                        reg: dst,
                        jump_if_false: 0,
                    });
                    self.compile_expr(right, Some(dst));
                    self.patch_jump(false_jump);
                }
                BinOp::Or => {
                    self.compile_expr(left, Some(dst));
                    let false_jump = self.current_mut().emit(Instruction::Test {
                        reg: dst,
                        jump_if_false: 0,
                    });
                    let end_jump = self.current_mut().emit(Instruction::Jump { offset: 0 });
                    self.patch_jump(false_jump);
                    self.compile_expr(right, Some(dst));
                    self.patch_jump(end_jump);
                }
                BinOp::Coalesce => {
                    let a = self.compile_expr(left, None);
                    let b = self.compile_expr(right, None);
                    self.current_mut().emit(Instruction::Coalesce { dst, a, b });
                    self.current_mut().free_reg(b);
                    self.current_mut().free_reg(a);
                }
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let a = self.compile_expr(left, None);
                    let b = self.compile_expr(right, None);
                    let false_jump = match op {
                        BinOp::Eq => self.current_mut().emit(Instruction::Eq {
                            a,
                            b,
                            jump_if_false: 0,
                        }),
                        BinOp::Ne => self.current_mut().emit(Instruction::Ne {
                            a,
                            b,
                            jump_if_false: 0,
                        }),
                        BinOp::Lt => self.current_mut().emit(Instruction::Lt {
                            a,
                            b,
                            jump_if_false: 0,
                        }),
                        BinOp::Le => self.current_mut().emit(Instruction::Le {
                            a,
                            b,
                            jump_if_false: 0,
                        }),
                        BinOp::Gt => self.current_mut().emit(Instruction::Gt {
                            a,
                            b,
                            jump_if_false: 0,
                        }),
                        BinOp::Ge => self.current_mut().emit(Instruction::Ge {
                            a,
                            b,
                            jump_if_false: 0,
                        }),
                        _ => unreachable!(),
                    };
                    self.current_mut().free_reg(b);
                    self.current_mut().free_reg(a);
                    self.current_mut()
                        .emit(Instruction::LoadBool { dst, val: true });
                    let skip = self.current_mut().emit(Instruction::Jump { offset: 1 });
                    self.patch_jump(false_jump);
                    self.current_mut()
                        .emit(Instruction::LoadBool { dst, val: false });
                    self.patch_jump(skip);
                }
                _ => {
                    let a = self.compile_expr(left, None);
                    let b = self.compile_expr(right, None);
                    match op {
                        BinOp::Add => self.current_mut().emit(Instruction::Add { dst, a, b }),
                        BinOp::Sub => self.current_mut().emit(Instruction::Sub { dst, a, b }),
                        BinOp::Mul => self.current_mut().emit(Instruction::Mul { dst, a, b }),
                        BinOp::Div => self.current_mut().emit(Instruction::Div { dst, a, b }),
                        BinOp::IDiv => self.current_mut().emit(Instruction::IDiv { dst, a, b }),
                        BinOp::Mod => self.current_mut().emit(Instruction::Mod { dst, a, b }),
                        BinOp::Pow => self.current_mut().emit(Instruction::Pow { dst, a, b }),
                        BinOp::BitAnd => self.current_mut().emit(Instruction::BitAnd { dst, a, b }),
                        BinOp::BitOr => self.current_mut().emit(Instruction::BitOr { dst, a, b }),
                        BinOp::BitXor => self.current_mut().emit(Instruction::BitXor { dst, a, b }),
                        BinOp::Shl => self.current_mut().emit(Instruction::Shl { dst, a, b }),
                        BinOp::Shr => self.current_mut().emit(Instruction::Shr { dst, a, b }),
                        BinOp::LShl => self.current_mut().emit(Instruction::LShl { dst, a, b }),
                        BinOp::LShr => self.current_mut().emit(Instruction::LShr { dst, a, b }),
                        BinOp::Concat => self.current_mut().emit(Instruction::Concat { dst, a, b }),
                        _ => unreachable!(),
                    };
                    self.current_mut().free_reg(b);
                    self.current_mut().free_reg(a);
                }
            },
            Expr::Call { callee, args, .. } => {
                let func_reg = self.current_mut().alloc_reg();
                self.compile_expr(callee, Some(func_reg));
                let (argc, arg_regs) = self.compile_args(args, 0);
                self.current_mut().emit(Instruction::Call {
                    callee: func_reg,
                    argc,
                    retc: 1,
                });
                if func_reg != dst {
                    self.current_mut()
                        .emit(Instruction::Move { dst, src: func_reg });
                }
                for r in arg_regs.into_iter().rev() {
                    self.current_mut().free_reg(r);
                }
                self.current_mut().free_reg(func_reg);
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                let func_reg = self.current_mut().alloc_reg();
                let arg0 = self.current_mut().alloc_reg();
                self.compile_expr(object, Some(arg0));
                let key_k = self.add_constant(Constant::String(method.clone()));
                self.current_mut().emit(Instruction::GetTableK {
                    dst: func_reg,
                    table: arg0,
                    key_k,
                });
                let (argc, arg_regs) = self.compile_args(args, 1);
                self.current_mut().emit(Instruction::Call {
                    callee: func_reg,
                    argc,
                    retc: 1,
                });
                if func_reg != dst {
                    self.current_mut()
                        .emit(Instruction::Move { dst, src: func_reg });
                }
                for r in arg_regs.into_iter().rev() {
                    self.current_mut().free_reg(r);
                }
                self.current_mut().free_reg(arg0);
                self.current_mut().free_reg(func_reg);
            }
            Expr::Function { params, body, .. } => {
                let proto_idx = self.compile_function(None, params, body);
                self.current_mut()
                    .emit(Instruction::Closure { dst, proto_idx });
            }
        }

        dst
    }

    /// True when `expr` can produce more than one value at runtime, so that a
    /// call, a `return` or a table constructor should keep them all instead of
    /// truncating to the first.
    pub(crate) fn is_multi_value(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Call { .. } | Expr::MethodCall { .. } | Expr::Vararg { .. }
        )
    }

    /// Compiles `expr` at `dst` keeping every value it yields; the VM's stack
    /// top is left just past the last one. `dst` must be the highest register
    /// allocated so far, because a call lays its arguments out above it.
    pub(crate) fn compile_expr_multi(&mut self, expr: &Expr, dst: u8) {
        match expr {
            Expr::Call { callee, args, .. } => {
                self.compile_expr(callee, Some(dst));
                let (argc, arg_regs) = self.compile_args(args, 0);
                self.current_mut().emit(Instruction::Call {
                    callee: dst,
                    argc,
                    retc: MULTRET,
                });
                for r in arg_regs.into_iter().rev() {
                    self.current_mut().free_reg(r);
                }
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                let arg0 = self.current_mut().alloc_reg();
                self.compile_expr(object, Some(arg0));
                let key_k = self.add_constant(Constant::String(method.clone()));
                self.current_mut().emit(Instruction::GetTableK {
                    dst,
                    table: arg0,
                    key_k,
                });
                let (argc, arg_regs) = self.compile_args(args, 1);
                self.current_mut().emit(Instruction::Call {
                    callee: dst,
                    argc,
                    retc: MULTRET,
                });
                for r in arg_regs.into_iter().rev() {
                    self.current_mut().free_reg(r);
                }
                self.current_mut().free_reg(arg0);
            }
            Expr::Vararg { .. } => {
                self.current_mut()
                    .emit(Instruction::Vararg { dst, count: 0 });
            }
            other => {
                self.compile_expr(other, Some(dst));
            }
        }
    }

    /// Compiles call arguments into the registers above the callee. `fixed`
    /// counts arguments already placed there (the receiver of a method call).
    /// The returned `argc` is MULTRET when the last argument spreads.
    pub(crate) fn compile_args(&mut self, args: &[Expr], fixed: u8) -> (u8, Vec<u8>) {
        let mut arg_regs = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let r = self.current_mut().alloc_reg();
            arg_regs.push(r);
            if index + 1 == args.len() && Compiler::is_multi_value(arg) {
                self.compile_expr_multi(arg, r);
                return (MULTRET, arg_regs);
            }
            self.compile_expr(arg, Some(r));
        }
        (fixed + args.len() as u8, arg_regs)
    }

    pub(crate) fn compile_function(
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
