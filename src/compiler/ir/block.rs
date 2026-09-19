// Control Flow Graph (CFG) and BasicBlock definitions for IR.

use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::{IrLabel, IrVar};

/// Where a captured value comes from, as seen by the function capturing it.
#[derive(Clone, Debug, PartialEq)]
pub enum UpvalSource {
    /// A register of the immediately enclosing function.
    ParentLocal(IrVar),
    /// An upvalue of the immediately enclosing function.
    ParentUpvalue,
    /// A variable further out. The enclosing function has to capture it first,
    /// which happens when this function is attached to it.
    Pending(IrVar),
}
use crate::parser::Param;

#[derive(Clone, Debug, PartialEq)]
pub struct BasicBlock {
    pub label: IrLabel,
    pub instructions: Vec<IrInst>,
    pub predecessors: Vec<IrLabel>,
    pub successors: Vec<IrLabel>,
}

impl BasicBlock {
    pub fn new(label: IrLabel) -> Self {
        Self {
            label,
            instructions: Vec::new(),
            predecessors: Vec::new(),
            successors: Vec::new(),
        }
    }

    pub fn terminator(&self) -> Option<&IrInst> {
        self.instructions.iter().rev().find(|i| i.is_terminator())
    }

    pub fn terminator_mut(&mut self) -> Option<&mut IrInst> {
        self.instructions
            .iter_mut()
            .rev()
            .find(|i| i.is_terminator())
    }

    pub fn add_instruction(&mut self, inst: IrInst) {
        self.instructions.push(inst);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControlFlowGraph {
    pub blocks: Vec<BasicBlock>,
    pub entry_label: IrLabel,
}

impl ControlFlowGraph {
    pub fn new(entry_label: IrLabel) -> Self {
        Self {
            blocks: Vec::new(),
            entry_label,
        }
    }

    pub fn find_block(&self, label: IrLabel) -> Option<&BasicBlock> {
        self.blocks.iter().find(|b| b.label == label)
    }

    pub fn find_block_mut(&mut self, label: IrLabel) -> Option<&mut BasicBlock> {
        self.blocks.iter_mut().find(|b| b.label == label)
    }

    pub fn block_index(&self, label: IrLabel) -> Option<usize> {
        self.blocks.iter().position(|b| b.label == label)
    }

    pub fn entry_block(&self) -> Option<&BasicBlock> {
        self.find_block(self.entry_label)
    }

    pub fn to_flat_instructions(&self) -> Vec<IrInst> {
        let mut out = Vec::new();
        for block in &self.blocks {
            out.push(IrInst::Label(block.label));
            for inst in &block.instructions {
                if !matches!(inst, IrInst::Label(_)) {
                    out.push(inst.clone());
                }
            }
        }
        out
    }
}

use crate::bytecode::proto::UpvalueDesc;

#[derive(Clone, Debug)]
pub struct IrFunction {
    pub name: Option<String>,
    pub params: Vec<Param>,
    pub num_params: u8,
    pub is_vararg: bool,
    pub instructions: Vec<IrInst>,
    pub protos: Vec<IrFunction>,
    pub upvalues: Vec<UpvalueDesc>,
    /// Parallel to `upvalues`: where each captured value comes from. A
    /// descriptor's `index` is meaningless until it is resolved from this.
    pub upvalue_vars: Vec<UpvalSource>,
    pub cfg: Option<ControlFlowGraph>,
}

impl IrFunction {
    pub fn new(name: Option<String>, num_params: u8, is_vararg: bool) -> Self {
        Self {
            name,
            params: Vec::new(),
            num_params,
            is_vararg,
            instructions: Vec::new(),
            protos: Vec::new(),
            upvalues: Vec::new(),
            upvalue_vars: Vec::new(),
            cfg: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct IrModule {
    pub main: IrFunction,
}
