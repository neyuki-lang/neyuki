// Lowering from IR to Neyuki Bytecode Proto with register allocation and jump patching.

use std::collections::HashMap;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::MULTRET;
use crate::bytecode::proto::{Constant, Proto};
use crate::compiler::ir::block::{IrFunction, IrModule};
use crate::compiler::ir::cfg_builder::build_cfg;
use crate::compiler::ir::inst::{IrInst, SpreadSink, SpreadSource};
use crate::compiler::ir::regalloc::allocate_registers;
use crate::compiler::ir::types::{IrBinaryOp, IrConstant, IrLabel, IrUnaryOp, IrVar};

struct RegAlloc {
    var_to_reg: HashMap<IrVar, u8>,
    next_reg: u8,
    /// Set once the 255-register limit is hit, so lowering reports it instead
    /// of handing out a register that is already in use.
    exhausted: bool,
}

impl RegAlloc {
    fn new(num_params: u8) -> Self {
        let mut var_to_reg = HashMap::new();
        for i in 0..num_params {
            var_to_reg.insert(IrVar(i as u32), i);
        }
        Self {
            var_to_reg,
            next_reg: num_params,
            exhausted: false,
        }
    }

    /// Starts from a linear-scan allocation, which reuses a register once the
    /// variable holding it is dead. Giving every variable its own register
    /// instead runs a function of any size past the 255-register limit.
    fn from_allocation(alloc: &crate::compiler::ir::regalloc::RegisterAllocation) -> Self {
        Self {
            var_to_reg: alloc.mapping.clone(),
            next_reg: alloc.max_registers,
            exhausted: false,
        }
    }

    fn get(&mut self, var: IrVar, proto: &mut Proto) -> u8 {
        if let Some(&r) = self.var_to_reg.get(&var) {
            r
        } else {
            let r = self.next_reg;
            if self.next_reg == u8::MAX {
                self.exhausted = true;
            }
            self.next_reg = self.next_reg.saturating_add(1);
            if self.next_reg > proto.max_registers {
                proto.max_registers = self.next_reg;
            }
            self.var_to_reg.insert(var, r);
            r
        }
    }

    /// Claims `size` consecutive registers that no variable will ever be given.
    /// Calls and `for` loops need their operands laid out side by side, and the
    /// registers next to an arbitrary variable's are not free to overwrite.
    fn reserve(&mut self, size: u8, proto: &mut Proto) -> u8 {
        let base = self.next_reg;
        if self.next_reg.checked_add(size).is_none() {
            self.exhausted = true;
        }
        self.next_reg = self.next_reg.saturating_add(size);
        if self.next_reg > proto.max_registers {
            proto.max_registers = self.next_reg;
        }
        base
    }

    fn bind(&mut self, var: IrVar, reg: u8, proto: &mut Proto) {
        self.var_to_reg.insert(var, reg);
        let needed = reg.saturating_add(1);
        if needed > self.next_reg {
            self.next_reg = needed;
        }
        if self.next_reg > proto.max_registers {
            proto.max_registers = self.next_reg;
        }
    }
}

/// Lays a producer out starting at register `at` and leaves every value it
/// yields on the stack from there. A trailing producer is emitted just above
/// this call's own arguments, so the two runs are contiguous.
fn emit_spread_source(source: &SpreadSource, at: u8, regs: &mut RegAlloc, proto: &mut Proto) {
    match source {
        SpreadSource::Call {
            callee,
            args,
            trailing,
        } => {
            let rc = regs.get(*callee, proto);
            if rc != at {
                proto.emit(Instruction::Move { dst: at, src: rc }, 1);
            }
            for (i, arg) in args.iter().enumerate() {
                let ra = regs.get(*arg, proto);
                let target = at + 1 + i as u8;
                if ra != target {
                    proto.emit(
                        Instruction::Move {
                            dst: target,
                            src: ra,
                        },
                        1,
                    );
                }
            }
            let argc = match trailing {
                Some(inner) => {
                    emit_spread_source(inner, at + 1 + args.len() as u8, regs, proto);
                    MULTRET
                }
                None => args.len() as u8,
            };
            proto.emit(
                Instruction::Call {
                    callee: at,
                    argc,
                    retc: MULTRET,
                },
                1,
            );
        }
        SpreadSource::Vararg => {
            proto.emit(Instruction::Vararg { dst: at, count: 0 }, 1);
        }
    }
}

/// The conditional-jump instruction for a comparison, with its offset unset.
fn compare_instruction(op: IrBinaryOp, a: u8, b: u8) -> Instruction {
    let jump_if_false = 0;
    match op {
        IrBinaryOp::Eq => Instruction::Eq {
            a,
            b,
            jump_if_false,
        },
        IrBinaryOp::Ne => Instruction::Ne {
            a,
            b,
            jump_if_false,
        },
        IrBinaryOp::Lt => Instruction::Lt {
            a,
            b,
            jump_if_false,
        },
        IrBinaryOp::Le => Instruction::Le {
            a,
            b,
            jump_if_false,
        },
        IrBinaryOp::Gt => Instruction::Gt {
            a,
            b,
            jump_if_false,
        },
        IrBinaryOp::Ge => Instruction::Ge {
            a,
            b,
            jump_if_false,
        },
        _ => unreachable!("not a comparison"),
    }
}

pub fn ir_to_bytecode(module: &IrModule) -> Result<Proto, String> {
    ir_function_to_proto(&module.main, 0)
}

fn ir_function_to_proto(func: &IrFunction, depth: usize) -> Result<Proto, String> {
    const MAX_IR_DEPTH: usize = 64;
    if depth >= MAX_IR_DEPTH {
        return Err(format!(
            "IR lowering depth limit ({}) exceeded: function nesting too deep",
            MAX_IR_DEPTH
        ));
    }
    let mut proto = Proto::new(func.name.clone(), func.num_params, func.is_vararg);
    proto.upvalues = func.upvalues.clone();

    let allocation = allocate_registers(&build_cfg(&func.instructions), func.num_params)?;
    let mut regs = RegAlloc::from_allocation(&allocation);
    if regs.next_reg > proto.max_registers {
        proto.max_registers = regs.next_reg;
    }

    // Reserve the register runs that calls and `for` loops need before any
    // variable is given a register. A call needs `[callee, args…]` side by
    // side and a `for` needs `[index, limit, step, vars…]`; taking the slots
    // next to an arbitrary variable would overwrite whatever lives there.
    // Binding the loop's variables into its window up front also means their
    // values are written straight into place, instead of being copied there
    // by moves that would re-run on every iteration.
    // Any variable the allocation missed still has to land below the windows
    // reserved next, because a call places the callee's frame directly above
    // the callee's register and would overwrite anything higher up.
    for inst in &func.instructions {
        for var in inst.use_vars().into_iter().chain(inst.def_vars()) {
            regs.get(var, &mut proto);
        }
    }

    for inst in &func.instructions {
        match inst {
            IrInst::ForPrep {
                base,
                limit,
                step,
                loop_var,
                ..
            } => {
                let rb = regs.reserve(4, &mut proto);
                regs.bind(*base, rb, &mut proto);
                regs.bind(*limit, rb + 1, &mut proto);
                regs.bind(*step, rb + 2, &mut proto);
                regs.bind(*loop_var, rb + 3, &mut proto);
            }
            IrInst::TForCall {
                base,
                state,
                ctrl,
                vars,
            } => {
                let rb = regs.reserve(3u8.saturating_add(vars.len() as u8), &mut proto);
                regs.bind(*base, rb, &mut proto);
                regs.bind(*state, rb + 1, &mut proto);
                regs.bind(*ctrl, rb + 2, &mut proto);
                for (i, v) in vars.iter().enumerate() {
                    regs.bind(*v, rb + 3 + i as u8, &mut proto);
                }
            }
            _ => {}
        }
    }

    // Reserved last, so the call window is the highest run in the frame.
    let call_slots = func
        .instructions
        .iter()
        .filter_map(|inst| match inst {
            IrInst::Call { args, retc, .. } => {
                Some(1u8.saturating_add((args.len() as u8).max(*retc)))
            }
            // A spread lays out the consuming call, then its fixed arguments,
            // then the producing call, all in the one window.
            IrInst::Spread { source, sink } => {
                let produced = source.slots();
                let consumed = match sink {
                    SpreadSink::Call {
                        fixed_args, retc, ..
                    } => 1u8
                        .saturating_add(fixed_args.len() as u8)
                        .max(1u8.saturating_add(*retc)),
                    SpreadSink::List { .. } | SpreadSink::Return => 0,
                };
                Some(consumed.saturating_add(produced))
            }
            _ => None,
        })
        .max();
    let call_window = call_slots.map(|slots| regs.reserve(slots, &mut proto));
    if regs.exhausted {
        return Err(format!(
            "register allocation failure: {} needs more than 255 registers",
            func.name.as_deref().unwrap_or("<anonymous>")
        ));
    }

    let mut capture_regs: HashMap<u16, Vec<Option<u8>>> = HashMap::new();
    let mut label_positions: HashMap<IrLabel, usize> = HashMap::new();
    let mut jump_patches: Vec<(usize, IrLabel)> = Vec::new();

    // How often each variable is read. A temporary read exactly once, by the
    // very next instruction, can be folded into that instruction.
    let mut use_count: HashMap<IrVar, usize> = HashMap::new();
    for inst in &func.instructions {
        for var in inst.use_vars() {
            *use_count.entry(var).or_insert(0) += 1;
        }
    }
    let single_use = |var: IrVar| use_count.get(&var).copied().unwrap_or(0) == 1;

    // Whether an instruction may leave its result in any register at all.
    // Calls and loops write fixed windows, so only these can be redirected.
    let places_freely = |inst: &IrInst| match inst {
        IrInst::LoadConst { .. }
        | IrInst::LoadNil { .. }
        | IrInst::Move { .. }
        | IrInst::UnOp { .. }
        | IrInst::NewTable { .. }
        | IrInst::GetTable { .. }
        | IrInst::GetGlobal { .. }
        | IrInst::GetUpval { .. }
        | IrInst::Closure { .. } => true,
        IrInst::BinOp { op, .. } => !op.is_comparison(),
        _ => false,
    };

    // A call's callee and arguments have to sit side by side in the call
    // window. A temporary computed just for the call, by an instruction
    // earlier in the same basic block, can be computed into its window slot
    // instead of being moved there. Only calls touch the window (nothing
    // else is ever given a window register), so a slot written early
    // survives until the next call. The search also stops at a label: past
    // one, control may re-enter from a loop back-edge after the call has
    // already clobbered the slot, as happens when the definition was hoisted
    // out of the loop.
    if let Some(cw) = call_window {
        let mut last_window_use: Option<usize> = None;
        for (index, inst) in func.instructions.iter().enumerate() {
            match inst {
                IrInst::Label(_) => last_window_use = Some(index),
                IrInst::Call { callee, args, .. } => {
                    let slot_of = |var: IrVar| -> Option<u8> {
                        if var == *callee {
                            return Some(0);
                        }
                        args.iter()
                            .position(|a| *a == var)
                            .map(|k| 1u8.saturating_add(k as u8))
                    };
                    let floor = last_window_use.map(|i| i + 1).unwrap_or(0);
                    for def in &func.instructions[floor..index] {
                        let Some(var) = def.def_var() else { continue };
                        if !places_freely(def) || !single_use(var) {
                            continue;
                        }
                        if let Some(slot) = slot_of(var) {
                            regs.bind(var, cw.saturating_add(slot), &mut proto);
                        }
                    }
                    last_window_use = Some(index);
                }
                IrInst::Spread { .. } => last_window_use = Some(index),
                _ => {}
            }
        }
    }

    let mut skip_next = false;
    for (index, inst) in func.instructions.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        let next = func.instructions.get(index + 1);

        // `t = <expr>; v = t` with `t` used nowhere else writes `v` directly.
        // Only instructions that may place their result in any register
        // qualify; a call's results land in its window and must be moved.
        if places_freely(inst)
            && let Some(IrInst::Move { dst: v, src: t }) = next
            && v != t
            && inst.def_var() == Some(*t)
            && single_use(*t)
        {
            let rv = regs.get(*v, &mut proto);
            regs.bind(*t, rv, &mut proto);
            skip_next = true;
        }

        match inst {
            IrInst::Label(l) => {
                label_positions.insert(*l, proto.instructions.len());
            }
            IrInst::LoadConst { dst, val } => {
                let r = regs.get(*dst, &mut proto);
                match val {
                    IrConstant::Nil => {
                        proto.emit(Instruction::LoadNil { dst: r }, 1);
                    }
                    IrConstant::Bool(b) => {
                        proto.emit(Instruction::LoadBool { dst: r, val: *b }, 1);
                    }
                    IrConstant::Int(i) => {
                        // Small integers travel as an immediate; only ones
                        // that do not fit go through the constant pool.
                        if let Some(val) = num_traits::ToPrimitive::to_i32(i) {
                            proto.emit(Instruction::LoadInt { dst: r, val }, 1);
                        } else {
                            let k = proto.add_constant(Constant::Int(i.clone()));
                            proto.emit(Instruction::LoadK { dst: r, k }, 1);
                        }
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
                let r = regs.get(*dst, &mut proto);
                proto.emit(Instruction::LoadNil { dst: r }, 1);
            }
            IrInst::Move { dst, src } => {
                let rd = regs.get(*dst, &mut proto);
                let rs = regs.get(*src, &mut proto);
                proto.emit(Instruction::Move { dst: rd, src: rs }, 1);
            }
            IrInst::BinOp { dst, op, lhs, rhs } => {
                let rd = regs.get(*dst, &mut proto);
                let ra = regs.get(*lhs, &mut proto);
                let rb = regs.get(*rhs, &mut proto);
                match op {
                    IrBinaryOp::Eq
                    | IrBinaryOp::Ne
                    | IrBinaryOp::Lt
                    | IrBinaryOp::Le
                    | IrBinaryOp::Gt
                    | IrBinaryOp::Ge => {
                        // A comparison whose only reader is the branch right
                        // after it becomes that branch: the VM's comparison
                        // instructions already jump when the test fails.
                        if let Some(IrInst::JumpIfFalse { cond, target }) = next
                            && cond == dst
                            && single_use(*dst)
                        {
                            let ip = proto.emit(compare_instruction(*op, ra, rb), 1);
                            jump_patches.push((ip, *target));
                            skip_next = true;
                            continue;
                        }
                        let false_jump = match op {
                            IrBinaryOp::Eq => proto.emit(
                                Instruction::Eq {
                                    a: ra,
                                    b: rb,
                                    jump_if_false: 0,
                                },
                                1,
                            ),
                            IrBinaryOp::Ne => proto.emit(
                                Instruction::Ne {
                                    a: ra,
                                    b: rb,
                                    jump_if_false: 0,
                                },
                                1,
                            ),
                            IrBinaryOp::Lt => proto.emit(
                                Instruction::Lt {
                                    a: ra,
                                    b: rb,
                                    jump_if_false: 0,
                                },
                                1,
                            ),
                            IrBinaryOp::Le => proto.emit(
                                Instruction::Le {
                                    a: ra,
                                    b: rb,
                                    jump_if_false: 0,
                                },
                                1,
                            ),
                            IrBinaryOp::Gt => proto.emit(
                                Instruction::Gt {
                                    a: ra,
                                    b: rb,
                                    jump_if_false: 0,
                                },
                                1,
                            ),
                            IrBinaryOp::Ge => proto.emit(
                                Instruction::Ge {
                                    a: ra,
                                    b: rb,
                                    jump_if_false: 0,
                                },
                                1,
                            ),
                            _ => unreachable!(),
                        };
                        proto.emit(Instruction::LoadBool { dst: rd, val: true }, 1);
                        let skip = proto.emit(Instruction::Jump { offset: 1 }, 1);
                        let false_target = proto.instructions.len();
                        let false_offset =
                            (false_target as isize - (false_jump as isize + 1)) as i16;
                        match &mut proto.instructions[false_jump] {
                            Instruction::Eq {
                                jump_if_false: o, ..
                            }
                            | Instruction::Ne {
                                jump_if_false: o, ..
                            }
                            | Instruction::Lt {
                                jump_if_false: o, ..
                            }
                            | Instruction::Le {
                                jump_if_false: o, ..
                            }
                            | Instruction::Gt {
                                jump_if_false: o, ..
                            }
                            | Instruction::Ge {
                                jump_if_false: o, ..
                            } => *o = false_offset,
                            _ => {}
                        }
                        proto.emit(
                            Instruction::LoadBool {
                                dst: rd,
                                val: false,
                            },
                            1,
                        );
                        let skip_target = proto.instructions.len();
                        let skip_offset = (skip_target as isize - (skip as isize + 1)) as i16;
                        if let Instruction::Jump { offset: o } = &mut proto.instructions[skip] {
                            *o = skip_offset;
                        }
                    }
                    _ => {
                        let inst = match op {
                            IrBinaryOp::Add => Instruction::Add {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Sub => Instruction::Sub {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Mul => Instruction::Mul {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Div => Instruction::Div {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::IDiv => Instruction::IDiv {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Mod => Instruction::Mod {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Pow => Instruction::Pow {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::BitAnd => Instruction::BitAnd {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::BitOr => Instruction::BitOr {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::BitXor => Instruction::BitXor {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Shl => Instruction::Shl {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Shr => Instruction::Shr {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::LShl => Instruction::LShl {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::LShr => Instruction::LShr {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Concat => Instruction::Concat {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            IrBinaryOp::Coalesce => Instruction::Coalesce {
                                dst: rd,
                                a: ra,
                                b: rb,
                            },
                            _ => unreachable!(),
                        };
                        proto.emit(inst, 1);
                    }
                }
            }
            IrInst::UnOp { dst, op, src } => {
                let rd = regs.get(*dst, &mut proto);
                let rs = regs.get(*src, &mut proto);
                let inst = match op {
                    IrUnaryOp::Neg => Instruction::Unm { dst: rd, src: rs },
                    IrUnaryOp::Not => Instruction::Not { dst: rd, src: rs },
                    IrUnaryOp::Len => Instruction::Len { dst: rd, src: rs },
                    IrUnaryOp::BitNot => Instruction::BitNot { dst: rd, src: rs },
                };
                proto.emit(inst, 1);
            }
            IrInst::NewTable { dst } => {
                let rd = regs.get(*dst, &mut proto);
                proto.emit(Instruction::NewTable { dst: rd }, 1);
            }
            IrInst::GetTable { dst, table, key } => {
                let rd = regs.get(*dst, &mut proto);
                let rt = regs.get(*table, &mut proto);
                let rk = regs.get(*key, &mut proto);
                proto.emit(
                    Instruction::GetTable {
                        dst: rd,
                        table: rt,
                        key: rk,
                    },
                    1,
                );
            }
            IrInst::SetTable { table, key, val } => {
                let rt = regs.get(*table, &mut proto);
                let rk = regs.get(*key, &mut proto);
                let rv = regs.get(*val, &mut proto);
                proto.emit(
                    Instruction::SetTable {
                        table: rt,
                        key: rk,
                        val: rv,
                    },
                    1,
                );
            }
            IrInst::AppendArray { table, src } => {
                let rt = regs.get(*table, &mut proto);
                let rs = regs.get(*src, &mut proto);
                proto.emit(Instruction::AppendArray { table: rt, src: rs }, 1);
            }
            IrInst::GetGlobal { dst, name } => {
                let rd = regs.get(*dst, &mut proto);
                let k = proto.add_constant(Constant::String(name.clone()));
                proto.emit(Instruction::GetGlobal { dst: rd, name_k: k }, 1);
            }
            IrInst::SetGlobal { name, src } => {
                let rs = regs.get(*src, &mut proto);
                let k = proto.add_constant(Constant::String(name.clone()));
                proto.emit(Instruction::SetGlobal { src: rs, name_k: k }, 1);
            }
            IrInst::GetUpval { dst, index } => {
                let rd = regs.get(*dst, &mut proto);
                proto.emit(
                    Instruction::GetUpval {
                        dst: rd,
                        upval_idx: *index,
                    },
                    1,
                );
            }
            IrInst::SetUpval { index, src } => {
                let rs = regs.get(*src, &mut proto);
                proto.emit(
                    Instruction::SetUpval {
                        src: rs,
                        upval_idx: *index,
                    },
                    1,
                );
            }
            IrInst::Closure {
                dst,
                proto_idx,
                captures,
            } => {
                let rd = regs.get(*dst, &mut proto);
                // Record where each captured variable ended up; the nested
                // prototype's upvalue descriptors are filled in from this.
                let mut resolved = Vec::with_capacity(captures.len());
                for capture in captures {
                    resolved.push(capture.map(|var| regs.get(var, &mut proto)));
                }
                capture_regs.insert(*proto_idx, resolved);
                proto.emit(
                    Instruction::Closure {
                        dst: rd,
                        proto_idx: *proto_idx,
                    },
                    1,
                );
            }
            IrInst::Spread { source, sink } => {
                let cw = call_window.expect("call window reserved when a spread exists");
                // The consuming call sits at the base of the window with its
                // fixed arguments after it; the produced values must follow
                // immediately, so the producer is placed right after them.
                let produce_at = match sink {
                    SpreadSink::Call { fixed_args, .. } => cw + 1 + fixed_args.len() as u8,
                    SpreadSink::List { .. } | SpreadSink::Return => cw,
                };

                emit_spread_source(source, produce_at, &mut regs, &mut proto);

                match sink {
                    SpreadSink::Call {
                        dsts,
                        callee,
                        fixed_args,
                        retc,
                    } => {
                        // The fixed arguments sit below the produced values,
                        // so together they form the one run the call reads.
                        let rc = regs.get(*callee, &mut proto);
                        proto.emit(Instruction::Move { dst: cw, src: rc }, 1);
                        for (i, arg) in fixed_args.iter().enumerate() {
                            let ra = regs.get(*arg, &mut proto);
                            proto.emit(
                                Instruction::Move {
                                    dst: cw + 1 + i as u8,
                                    src: ra,
                                },
                                1,
                            );
                        }
                        proto.emit(
                            Instruction::Call {
                                callee: cw,
                                argc: MULTRET,
                                retc: *retc,
                            },
                            1,
                        );
                        for (slot, d) in dsts.iter().enumerate() {
                            let rd = regs.get(*d, &mut proto);
                            let src = cw + slot as u8;
                            if rd != src {
                                proto.emit(Instruction::Move { dst: rd, src }, 1);
                            }
                        }
                    }
                    SpreadSink::List { table } => {
                        let rt = regs.get(*table, &mut proto);
                        proto.emit(
                            Instruction::SetList {
                                table: rt,
                                base: produce_at,
                                count: MULTRET,
                            },
                            1,
                        );
                    }
                    SpreadSink::Return => {
                        proto.emit(
                            Instruction::Return {
                                base: produce_at,
                                count: MULTRET,
                            },
                            1,
                        );
                    }
                }
            }
            IrInst::Vararg { dst, count } => {
                let rd = regs.get(*dst, &mut proto);
                proto.emit(
                    Instruction::Vararg {
                        dst: rd,
                        count: *count,
                    },
                    1,
                );
            }
            IrInst::ForPrep {
                base,
                limit,
                step,
                loop_var,
                jump,
            } => {
                // `base`, `limit`, `step` and `loop_var` were bound to this
                // loop's window before lowering, so they are already in place.
                let rb = regs.get(*base, &mut proto);
                debug_assert_eq!(regs.get(*limit, &mut proto), rb + 1);
                debug_assert_eq!(regs.get(*step, &mut proto), rb + 2);
                debug_assert_eq!(regs.get(*loop_var, &mut proto), rb + 3);

                let ip = proto.emit(Instruction::ForPrep { base: rb, jump: 0 }, 1);
                jump_patches.push((ip, *jump));
            }
            IrInst::ForLoop { base, jump } => {
                let rb = regs.get(*base, &mut proto);
                let ip = proto.emit(Instruction::ForLoop { base: rb, jump: 0 }, 1);
                jump_patches.push((ip, *jump));
            }
            IrInst::TForCall {
                base,
                state,
                ctrl,
                vars,
            } => {
                // The iterator, its state and the control variable were bound
                // to this loop's window before lowering; copying them here
                // would re-run on every iteration and reset the control
                // variable, so the loop would never end.
                let rb = regs.get(*base, &mut proto);
                debug_assert_eq!(regs.get(*state, &mut proto), rb + 1);
                debug_assert_eq!(regs.get(*ctrl, &mut proto), rb + 2);

                proto.emit(
                    Instruction::TForCall {
                        base: rb,
                        retc: vars.len() as u8,
                    },
                    1,
                );
            }
            IrInst::TForLoop { base, jump } => {
                let rb = regs.get(*base, &mut proto);
                let ip = proto.emit(Instruction::TForLoop { base: rb, jump: 0 }, 1);
                jump_patches.push((ip, *jump));
            }
            IrInst::Call {
                dsts,
                callee,
                args,
                retc,
            } => {
                let cw = call_window.expect("call window reserved when a call exists");
                let rc = regs.get(*callee, &mut proto);
                if rc != cw {
                    proto.emit(Instruction::Move { dst: cw, src: rc }, 1);
                }
                for (i, arg) in args.iter().enumerate() {
                    let ra = regs.get(*arg, &mut proto);
                    let target_reg = cw + 1 + i as u8;
                    if ra != target_reg {
                        proto.emit(
                            Instruction::Move {
                                dst: target_reg,
                                src: ra,
                            },
                            1,
                        );
                    }
                }
                proto.emit(
                    Instruction::Call {
                        callee: cw,
                        argc: args.len() as u8,
                        retc: *retc,
                    },
                    1,
                );
                // Results land in the window starting at the callee's slot.
                for (slot, d) in dsts.iter().enumerate() {
                    let rd = regs.get(*d, &mut proto);
                    let src = cw + slot as u8;
                    if rd != src {
                        proto.emit(Instruction::Move { dst: rd, src }, 1);
                    }
                }
            }
            IrInst::Return(vars) => {
                if vars.is_empty() {
                    let r = if proto.max_registers == 0 {
                        proto.max_registers = 1;
                        0
                    } else {
                        0
                    };
                    proto.emit(Instruction::LoadNil { dst: r }, 1);
                    proto.emit(Instruction::Return { base: r, count: 1 }, 1);
                } else if vars.len() == 1 {
                    let r = regs.get(vars[0], &mut proto);
                    proto.emit(Instruction::Return { base: r, count: 1 }, 1);
                } else {
                    let base = regs.get(vars[0], &mut proto);
                    for (i, v) in vars[1..].iter().enumerate() {
                        let vr = regs.get(*v, &mut proto);
                        let target_r = base + 1 + i as u8;
                        if vr != target_r {
                            proto.emit(
                                Instruction::Move {
                                    dst: target_r,
                                    src: vr,
                                },
                                1,
                            );
                        }
                    }
                    let needed = base as usize + vars.len();
                    if needed > proto.max_registers as usize {
                        proto.max_registers = needed as u8;
                    }
                    proto.emit(
                        Instruction::Return {
                            base,
                            count: vars.len() as u8,
                        },
                        1,
                    );
                }
            }
            IrInst::Jump(label) => {
                let ip = proto.emit(Instruction::Jump { offset: 0 }, 1);
                jump_patches.push((ip, *label));
            }
            IrInst::JumpIfFalse { cond, target } => {
                let r = regs.get(*cond, &mut proto);
                let ip = proto.emit(
                    Instruction::Test {
                        reg: r,
                        jump_if_false: 0,
                    },
                    1,
                );
                jump_patches.push((ip, *target));
            }
            IrInst::Phi { dst, incoming } => {
                // Fallback if SSA deconstruction wasn't run explicitly: move first incoming value
                if let Some((_, src)) = incoming.first() {
                    let r_dst = regs.get(*dst, &mut proto);
                    let r_src = regs.get(*src, &mut proto);
                    if r_dst != r_src {
                        proto.emit(
                            Instruction::Move {
                                dst: r_dst,
                                src: r_src,
                            },
                            1,
                        );
                    }
                }
            }
        }
    }

    for (jump_ip, label) in jump_patches {
        if let Some(&target_ip) = label_positions.get(&label) {
            let offset = (target_ip as isize - (jump_ip as isize + 1)) as i16;
            match &mut proto.instructions[jump_ip] {
                Instruction::Jump { offset: o } => *o = offset,
                Instruction::Test {
                    jump_if_false: o, ..
                }
                | Instruction::Eq {
                    jump_if_false: o, ..
                }
                | Instruction::Ne {
                    jump_if_false: o, ..
                }
                | Instruction::Lt {
                    jump_if_false: o, ..
                }
                | Instruction::Le {
                    jump_if_false: o, ..
                }
                | Instruction::Gt {
                    jump_if_false: o, ..
                }
                | Instruction::Ge {
                    jump_if_false: o, ..
                } => *o = offset,
                Instruction::ForPrep { jump: o, .. } => *o = offset,
                Instruction::ForLoop { jump: o, .. } => *o = offset,
                Instruction::TForLoop { jump: o, .. } => *o = offset,
                _ => {}
            }
        }
    }

    // Lower nested function prototypes
    for (index, child) in func.protos.iter().enumerate() {
        let mut child_proto = ir_function_to_proto(child, depth + 1)?;
        if let Some(resolved) = capture_regs.get(&(index as u16)) {
            for (slot, updesc) in child_proto.upvalues.iter_mut().enumerate() {
                if let Some(Some(reg)) = resolved.get(slot) {
                    updesc.index = *reg;
                }
            }
        }
        proto.protos.push(std::rc::Rc::new(child_proto));
    }

    Ok(proto)
}
