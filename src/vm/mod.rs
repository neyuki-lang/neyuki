// Register-based Virtual Machine module for Neyuki.

pub mod buffer;
pub mod frame;
pub mod gc;
pub mod libs;
pub mod machine;
pub mod value;

use std::fs;
pub use machine::VM;
pub use value::Value;

use crate::bytecode::deserialize;

// Execute compiled bytecode buffer
pub fn execute_bytecode(bytes: &[u8]) -> Result<Value, String> {
    let proto = deserialize(bytes)?;
    let mut vm = VM::new();
    vm.execute(proto)
}

// Execute compiled bytecode file
pub fn execute_bytecode_file(path: &str) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read bytecode file {}: {}", path, e))?;
    execute_bytecode(&bytes)
}
