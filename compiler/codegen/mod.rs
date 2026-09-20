// Modular bytecode compiler decomposing compilation into separate stages.

pub mod error;
pub mod expr;
pub(crate) mod state;
pub mod stmt;

pub use error::CompileError;
use state::FuncState;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::{Constant, Proto};
use crate::parser::Stmt;

pub struct Compiler {
    pub(crate) funcs: Vec<FuncState>,
    pub errors: Vec<CompileError>,
}

impl Compiler {
    pub fn new(name: Option<String>, num_params: u8, is_vararg: bool) -> Self {
        Self {
            funcs: vec![FuncState::new(name, num_params, is_vararg)],
            errors: Vec::new(),
        }
    }

    pub(crate) fn current(&self) -> &FuncState {
        self.funcs.last().unwrap()
    }

    pub(crate) fn current_mut(&mut self) -> &mut FuncState {
        self.funcs.last_mut().unwrap()
    }

    pub(crate) fn alloc_reg(&mut self) -> u8 {
        let r = self.current_mut().reg_top;
        if self.current().reg_top == 255 {
            self.emit_error("too many local variables: register limit (255) exceeded");
            return 255;
        }
        self.current_mut().reg_top += 1;
        if self.current().reg_top > self.current().proto.max_registers {
            let new_max = self.current().reg_top;
            self.current_mut().proto.max_registers = new_max;
        }
        r
    }

    pub(crate) fn add_constant(&mut self, c: Constant) -> u16 {
        if self.current().proto.constants.len() >= 65535 {
            self.emit_error("too many constants: constant pool limit (65535) exceeded");
            return 0;
        }
        self.current_mut().add_constant(c)
    }

    pub(crate) fn patch_jump(&mut self, jump_ip: usize) {
        let target_ip = self.current().proto.instructions.len();
        let raw_offset = target_ip as isize - (jump_ip as isize + 1);
        if raw_offset > i16::MAX as isize || raw_offset < i16::MIN as isize {
            self.emit_error(format!(
                "jump offset {} exceeds i16 range — function too large",
                raw_offset
            ));
        }
        let offset = raw_offset as i16;
        let insts = &mut self.current_mut().proto.instructions;
        match &mut insts[jump_ip] {
            Instruction::Jump { offset: o } => *o = offset,
            Instruction::Test {
                jump_if_false: o, ..
            } => *o = offset,
            Instruction::Eq {
                jump_if_false: o, ..
            } => *o = offset,
            Instruction::Ne {
                jump_if_false: o, ..
            } => *o = offset,
            Instruction::Lt {
                jump_if_false: o, ..
            } => *o = offset,
            Instruction::Le {
                jump_if_false: o, ..
            } => *o = offset,
            Instruction::Gt {
                jump_if_false: o, ..
            } => *o = offset,
            Instruction::Ge {
                jump_if_false: o, ..
            } => *o = offset,
            Instruction::TForLoop { jump: o, .. } => *o = offset,
            Instruction::ForPrep { jump: o, .. } => *o = offset,
            Instruction::ForLoop { jump: o, .. } => *o = offset,
            _ => self.emit_error("attempted to patch non-jump instruction"),
        }
    }

    pub(crate) fn emit_error(&mut self, msg: impl Into<String>) {
        let line = self.current().current_line;
        self.errors.push(CompileError {
            message: msg.into(),
            line,
        });
    }

    pub fn finish(mut self) -> Result<Proto, Vec<CompileError>> {
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        Ok(self.funcs.pop().unwrap().finish())
    }

    pub(crate) fn resolve_upval_rec(
        funcs: &mut [FuncState],
        func_idx: usize,
        name: &str,
    ) -> Option<u8> {
        if func_idx == 0 {
            return None;
        }
        let parent_idx = func_idx - 1;
        if let Some(local_reg) = funcs[parent_idx].resolve_local(name) {
            funcs[parent_idx].mark_captured(local_reg);
            return Some(funcs[func_idx].add_upvalue(true, local_reg));
        }
        if let Some(parent_upval) = Self::resolve_upval_rec(funcs, parent_idx, name) {
            return Some(funcs[func_idx].add_upvalue(false, parent_upval));
        }
        None
    }

    pub(crate) fn resolve_variable(&mut self, name: &str) -> (bool, Option<u8>) {
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
}
