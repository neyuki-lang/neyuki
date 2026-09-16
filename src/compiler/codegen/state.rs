// Local variables, loop contexts, and function compilation state for bytecode generator.

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::{Constant, Proto, UpvalueDesc};

#[derive(Clone, Debug)]
pub(crate) struct LocalVar {
    pub(crate) name: String,
    pub(crate) reg: u8,
    pub(crate) depth: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct LoopContext {
    pub(crate) _start_ip: usize,
    pub(crate) break_jumps: Vec<usize>,
    pub(crate) continue_ips: Vec<usize>,
}

pub(crate) struct FuncState {
    pub(crate) proto: Proto,
    pub(crate) locals: Vec<LocalVar>,
    pub(crate) scope_depth: usize,
    pub(crate) reg_top: u8,
    pub(crate) loops: Vec<LoopContext>,
    pub(crate) current_line: u32,
}

impl FuncState {
    pub(crate) fn new(name: Option<String>, num_params: u8, is_vararg: bool) -> Self {
        Self {
            proto: Proto::new(name, num_params, is_vararg),
            locals: Vec::new(),
            scope_depth: 0,
            reg_top: 0,
            loops: Vec::new(),
            current_line: 1,
        }
    }

    pub(crate) fn alloc_reg(&mut self) -> u8 {
        let r = self.reg_top;
        self.reg_top += 1;
        if self.reg_top > self.proto.max_registers {
            self.proto.max_registers = self.reg_top;
        }
        r
    }

    pub(crate) fn free_reg(&mut self, reg: u8) {
        if reg + 1 == self.reg_top {
            self.reg_top -= 1;
        }
    }

    pub(crate) fn emit(&mut self, inst: Instruction) -> usize {
        self.proto.emit(inst, self.current_line)
    }

    pub(crate) fn add_constant(&mut self, c: Constant) -> u16 {
        self.proto.add_constant(c)
    }

    pub(crate) fn enter_scope(&mut self) {
        self.scope_depth += 1;
    }

    pub(crate) fn exit_scope(&mut self) {
        self.scope_depth -= 1;
        while let Some(local) = self.locals.last() {
            if local.depth > self.scope_depth {
                let reg = local.reg;
                let name = local.name.clone();
                let to_pc = self.proto.instructions.len() as u32;
                self.proto.close_local(&name, to_pc);
                self.free_reg(reg);
                self.locals.pop();
            } else {
                break;
            }
        }
    }

    pub(crate) fn add_local(&mut self, name: String, reg: u8) {
        let from_pc = self.proto.instructions.len() as u32;
        self.proto.push_local(name.clone(), from_pc);
        self.locals.push(LocalVar {
            name,
            reg,
            depth: self.scope_depth,
        });
    }

    pub(crate) fn resolve_local(&self, name: &str) -> Option<u8> {
        for local in self.locals.iter().rev() {
            if local.name == name {
                return Some(local.reg);
            }
        }
        None
    }

    pub(crate) fn add_upvalue(&mut self, in_stack: bool, index: u8) -> u8 {
        for (i, upval) in self.proto.upvalues.iter().enumerate() {
            if upval.in_stack == in_stack && upval.index == index {
                return i as u8;
            }
        }
        let idx = self.proto.upvalues.len() as u8;
        self.proto.upvalues.push(UpvalueDesc { in_stack, index });
        idx
    }

    pub(crate) fn finish(mut self) -> Proto {
        let needs_return = !matches!(self.proto.instructions.last(), Some(Instruction::Return { .. }));
        if needs_return {
            let r = self.alloc_reg();
            self.emit(Instruction::LoadNil { dst: r });
            self.emit(Instruction::Return { base: r, count: 1 });
        }
        self.proto
    }
}
