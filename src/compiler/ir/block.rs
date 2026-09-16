// Control Flow Graph (CFG) and BasicBlock definitions for IR.

use crate::compiler::ir::inst::IrInst;
use crate::compiler::ir::types::IrLabel;
use crate::parser::Param;

#[derive(Clone, Debug)]
pub struct BasicBlock {
    pub label: IrLabel,
    pub instructions: Vec<IrInst>,
    pub predecessors: Vec<IrLabel>,
    pub successors: Vec<IrLabel>,
}

#[derive(Clone, Debug)]
pub struct ControlFlowGraph {
    pub blocks: Vec<BasicBlock>,
    pub entry_label: IrLabel,
}

#[derive(Clone, Debug)]
pub struct IrFunction {
    pub name: Option<String>,
    pub params: Vec<Param>,
    pub num_params: u8,
    pub is_vararg: bool,
    pub instructions: Vec<IrInst>,
    pub protos: Vec<IrFunction>,
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
        }
    }
}

#[derive(Clone, Debug)]
pub struct IrModule {
    pub main: IrFunction,
}

