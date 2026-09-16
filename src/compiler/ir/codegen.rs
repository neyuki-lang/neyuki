// Lowering from IR to Neyuki Bytecode Proto with register allocation and jump patching.

use std::collections::HashMap;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::{Constant, Proto};
use crate::compiler::ir::block::{IrFunction, IrModule};
use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::{IrBinaryOp, IrConstant, IrLabel, IrUnaryOp, IrVar};

pub fn ir_to_bytecode(module: &IrModule) -> Proto {
    ir_function_to_proto(&module.main)
}

fn ir_function_to_proto(func: &IrFunction) -> Proto {
    let mut proto = Proto::new(func.name.clone(), func.num_params, func.is_vararg);

    let mut var_to_reg: HashMap<IrVar, u8> = HashMap::new();
    // Parameters are assigned to registers 0 .. num_params upfront
    for i in 0..func.num_params {
        var_to_reg.insert(IrVar(i as u32), i);
    }
    let mut next_reg: u8 = func.num_params;
    if next_reg > proto.max_registers {
        proto.max_registers = next_reg;
    }

    let mut get_reg = |var: IrVar, proto: &mut Proto| -> u8 {
        if let Some(&r) = var_to_reg.get(&var) {
            r
        } else {
            let r = next_reg;
            next_reg = next_reg.saturating_add(1);
            if next_reg > proto.max_registers {
                proto.max_registers = next_reg;
            }
            var_to_reg.insert(var, r);
            r
        }
    };

    let mut label_positions: HashMap<IrLabel, usize> = HashMap::new();
    let mut jump_patches: Vec<(usize, IrLabel)> = Vec::new();

    for inst in &func.instructions {
        match inst {
            IrInst::Label(l) => {
                label_positions.insert(*l, proto.instructions.len());
            }
            IrInst::LoadConst { dst, val } => {
                let r = get_reg(*dst, &mut proto);
                match val {
                    IrConstant::Nil => { proto.emit(Instruction::LoadNil { dst: r }, 1); }
                    IrConstant::Bool(b) => { proto.emit(Instruction::LoadBool { dst: r, val: *b }, 1); }
                    IrConstant::Int(i) => {
                        let k = proto.add_constant(Constant::Int(i.clone()));
                        proto.emit(Instruction::LoadK { dst: r, k }, 1);
                    }
                    IrConstant::Float(f) => {
                        let k = proto.add_constant(Constant::Float(*f));
                        proto.emit(Instruction::LoadK { dst: r, k }, 1);
                    }
                    IrConstant::String(s) => {
                        let k = proto.add_constant(Constant::String(s.clone()));
                        proto.emit(Instruction::LoadK { dst: r, k }, 1);
                    }
                }
            }
            IrInst::LoadNil { dst } => {
                let r = get_reg(*dst, &mut proto);
                proto.emit(Instruction::LoadNil { dst: r }, 1);
            }
            IrInst::Move { dst, src } => {
                let rd = get_reg(*dst, &mut proto);
                let rs = get_reg(*src, &mut proto);
                proto.emit(Instruction::Move { dst: rd, src: rs }, 1);
            }
            IrInst::BinOp { dst, op, lhs, rhs } => {
                let rd = get_reg(*dst, &mut proto);
                let ra = get_reg(*lhs, &mut proto);
                let rb = get_reg(*rhs, &mut proto);
                match op {
                    IrBinaryOp::Eq | IrBinaryOp::Ne | IrBinaryOp::Lt | IrBinaryOp::Le | IrBinaryOp::Gt | IrBinaryOp::Ge => {
                        let false_jump = match op {
                            IrBinaryOp::Eq => proto.emit(Instruction::Eq { a: ra, b: rb, jump_if_false: 0 }, 1),
                            IrBinaryOp::Ne => proto.emit(Instruction::Ne { a: ra, b: rb, jump_if_false: 0 }, 1),
                            IrBinaryOp::Lt => proto.emit(Instruction::Lt { a: ra, b: rb, jump_if_false: 0 }, 1),
                            IrBinaryOp::Le => proto.emit(Instruction::Le { a: ra, b: rb, jump_if_false: 0 }, 1),
                            IrBinaryOp::Gt => proto.emit(Instruction::Gt { a: ra, b: rb, jump_if_false: 0 }, 1),
                            IrBinaryOp::Ge => proto.emit(Instruction::Ge { a: ra, b: rb, jump_if_false: 0 }, 1),
                            _ => unreachable!(),
                        };
                        proto.emit(Instruction::LoadBool { dst: rd, val: true }, 1);
                        let skip = proto.emit(Instruction::Jump { offset: 1 }, 1);
                        let false_target = proto.instructions.len();
                        let false_offset = (false_target as isize - (false_jump as isize + 1)) as i16;
                        match &mut proto.instructions[false_jump] {
                            Instruction::Eq { jump_if_false: o, .. }
                            | Instruction::Ne { jump_if_false: o, .. }
                            | Instruction::Lt { jump_if_false: o, .. }
                            | Instruction::Le { jump_if_false: o, .. }
                            | Instruction::Gt { jump_if_false: o, .. }
                            | Instruction::Ge { jump_if_false: o, .. } => *o = false_offset,
                            _ => {}
                        }
                        proto.emit(Instruction::LoadBool { dst: rd, val: false }, 1);
                        let skip_target = proto.instructions.len();
                        let skip_offset = (skip_target as isize - (skip as isize + 1)) as i16;
                        if let Instruction::Jump { offset: o } = &mut proto.instructions[skip] {
                            *o = skip_offset;
                        }
                    }
                    _ => {
                        let inst = match op {
                            IrBinaryOp::Add => Instruction::Add { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Sub => Instruction::Sub { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Mul => Instruction::Mul { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Div => Instruction::Div { dst: rd, a: ra, b: rb },
                            IrBinaryOp::IDiv => Instruction::IDiv { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Mod => Instruction::Mod { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Pow => Instruction::Pow { dst: rd, a: ra, b: rb },
                            IrBinaryOp::BitAnd => Instruction::BitAnd { dst: rd, a: ra, b: rb },
                            IrBinaryOp::BitOr => Instruction::BitOr { dst: rd, a: ra, b: rb },
                            IrBinaryOp::BitXor => Instruction::BitXor { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Shl => Instruction::Shl { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Shr => Instruction::Shr { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Concat => Instruction::Concat { dst: rd, a: ra, b: rb },
                            IrBinaryOp::Coalesce => Instruction::Coalesce { dst: rd, a: ra, b: rb },
                            _ => unreachable!(),
                        };
                        proto.emit(inst, 1);
                    }
                }
            }
            IrInst::UnOp { dst, op, src } => {
                let rd = get_reg(*dst, &mut proto);
                let rs = get_reg(*src, &mut proto);
                let inst = match op {
                    IrUnaryOp::Neg => Instruction::Unm { dst: rd, src: rs },
                    IrUnaryOp::Not => Instruction::Not { dst: rd, src: rs },
                    IrUnaryOp::Len => Instruction::Len { dst: rd, src: rs },
                    IrUnaryOp::BitNot => Instruction::BitNot { dst: rd, src: rs },
                };
                proto.emit(inst, 1);
            }
            IrInst::NewTable { dst } => {
                let rd = get_reg(*dst, &mut proto);
                proto.emit(Instruction::NewTable { dst: rd }, 1);
            }
            IrInst::GetTable { dst, table, key } => {
                let rd = get_reg(*dst, &mut proto);
                let rt = get_reg(*table, &mut proto);
                let rk = get_reg(*key, &mut proto);
                proto.emit(Instruction::GetTable { dst: rd, table: rt, key: rk }, 1);
            }
            IrInst::SetTable { table, key, val } => {
                let rt = get_reg(*table, &mut proto);
                let rk = get_reg(*key, &mut proto);
                let rv = get_reg(*val, &mut proto);
                proto.emit(Instruction::SetTable { table: rt, key: rk, val: rv }, 1);
            }
            IrInst::AppendArray { table, src } => {
                let rt = get_reg(*table, &mut proto);
                let rs = get_reg(*src, &mut proto);
                proto.emit(Instruction::AppendArray { table: rt, src: rs }, 1);
            }
            IrInst::GetGlobal { dst, name } => {
                let rd = get_reg(*dst, &mut proto);
                let k = proto.add_constant(Constant::String(name.clone()));
                proto.emit(Instruction::GetGlobal { dst: rd, name_k: k }, 1);
            }
            IrInst::SetGlobal { name, src } => {
                let rs = get_reg(*src, &mut proto);
                let k = proto.add_constant(Constant::String(name.clone()));
                proto.emit(Instruction::SetGlobal { src: rs, name_k: k }, 1);
            }
            IrInst::GetUpval { dst, index } => {
                let rd = get_reg(*dst, &mut proto);
                proto.emit(Instruction::GetUpval { dst: rd, upval_idx: *index }, 1);
            }
            IrInst::SetUpval { index, src } => {
                let rs = get_reg(*src, &mut proto);
                proto.emit(Instruction::SetUpval { src: rs, upval_idx: *index }, 1);
            }
            IrInst::Closure { dst, proto_idx } => {
                let rd = get_reg(*dst, &mut proto);
                proto.emit(Instruction::Closure { dst: rd, proto_idx: *proto_idx }, 1);
            }
            IrInst::Vararg { dst, count } => {
                let rd = get_reg(*dst, &mut proto);
                proto.emit(Instruction::Vararg { dst: rd, count: *count }, 1);
            }
            IrInst::ForPrep { base, jump } => {
                let rb = get_reg(*base, &mut proto);
                let ip = proto.emit(Instruction::ForPrep { base: rb, jump: 0 }, 1);
                jump_patches.push((ip, *jump));
            }
            IrInst::ForLoop { base, jump } => {
                let rb = get_reg(*base, &mut proto);
                let ip = proto.emit(Instruction::ForLoop { base: rb, jump: 0 }, 1);
                jump_patches.push((ip, *jump));
            }
            IrInst::TForCall { base, retc } => {
                let rb = get_reg(*base, &mut proto);
                proto.emit(Instruction::TForCall { base: rb, retc: *retc }, 1);
            }
            IrInst::TForLoop { base, jump } => {
                let rb = get_reg(*base, &mut proto);
                let ip = proto.emit(Instruction::TForLoop { base: rb, jump: 0 }, 1);
                jump_patches.push((ip, *jump));
            }
            IrInst::Call { dst, callee, args, retc } => {
                let rc = get_reg(*callee, &mut proto);
                for (i, arg) in args.iter().enumerate() {
                    let ra = get_reg(*arg, &mut proto);
                    let target_reg = rc + 1 + i as u8;
                    if ra != target_reg {
                        proto.emit(Instruction::Move { dst: target_reg, src: ra }, 1);
                    }
                }
                let needed_regs = (rc as usize) + 1 + (args.len()).max(*retc as usize);
                if needed_regs > proto.max_registers as usize {
                    proto.max_registers = needed_regs as u8;
                }
                proto.emit(Instruction::Call { callee: rc, argc: args.len() as u8, retc: *retc }, 1);
                if let Some(d) = dst {
                    let rd = get_reg(*d, &mut proto);
                    if rd != rc {
                        proto.emit(Instruction::Move { dst: rd, src: rc }, 1);
                    }
                }
            }
            IrInst::Return(vars) => {
                if vars.is_empty() {
                    let r = if proto.max_registers == 0 { proto.max_registers = 1; 0 } else { 0 };
                    proto.emit(Instruction::LoadNil { dst: r }, 1);
                    proto.emit(Instruction::Return { base: r, count: 1 }, 1);
                } else if vars.len() == 1 {
                    let r = get_reg(vars[0], &mut proto);
                    proto.emit(Instruction::Return { base: r, count: 1 }, 1);
                } else {
                    let base = get_reg(vars[0], &mut proto);
                    for (i, v) in vars[1..].iter().enumerate() {
                        let vr = get_reg(*v, &mut proto);
                        let target_r = base + 1 + i as u8;
                        if vr != target_r {
                            proto.emit(Instruction::Move { dst: target_r, src: vr }, 1);
                        }
                    }
                    let needed = base as usize + vars.len();
                    if needed > proto.max_registers as usize {
                        proto.max_registers = needed as u8;
                    }
                    proto.emit(Instruction::Return { base, count: vars.len() as u8 }, 1);
                }
            }
            IrInst::Jump(label) => {
                let ip = proto.emit(Instruction::Jump { offset: 0 }, 1);
                jump_patches.push((ip, *label));
            }
            IrInst::JumpIfFalse { cond, target } => {
                let r = get_reg(*cond, &mut proto);
                let ip = proto.emit(Instruction::Test { reg: r, jump_if_false: 0 }, 1);
                jump_patches.push((ip, *target));
            }
        }
    }

    for (jump_ip, label) in jump_patches {
        if let Some(&target_ip) = label_positions.get(&label) {
            let offset = (target_ip as isize - (jump_ip as isize + 1)) as i16;
            match &mut proto.instructions[jump_ip] {
                Instruction::Jump { offset: o } => *o = offset,
                Instruction::Test { jump_if_false: o, .. } => *o = offset,
                Instruction::ForPrep { jump: o, .. } => *o = offset,
                Instruction::ForLoop { jump: o, .. } => *o = offset,
                Instruction::TForLoop { jump: o, .. } => *o = offset,
                _ => {}
            }
        }
    }

    // Lower nested function prototypes
    for child in &func.protos {
        let child_proto = ir_function_to_proto(child);
        proto.protos.push(child_proto);
    }

    proto
}
