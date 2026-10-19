pub mod buffer;
pub mod builtins;
pub mod frame;
pub mod gc;
pub mod hash;
pub mod libs;
pub mod machine;
pub mod ops;
pub mod table;
pub mod value;

pub use machine::VM;
use std::fs;
pub use table::TableKey;
pub use value::Value;

use crate::bytecode::deserialize;

// Execute compiled bytecode buffer
pub fn execute_bytecode(bytes: &[u8]) -> Result<Value, String> {
    let proto = deserialize(bytes)?;
    let mut vm = VM::new();
    vm.execute_program(proto)
}

pub fn run_file_with_options(path: &str, options: crate::compiler::Options) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read {}: {}", path, e))?;
    if bytes.starts_with(crate::bytecode::MAGIC) {
        return execute_bytecode(&bytes).map(|_| ());
    }
    let source = String::from_utf8(bytes)
        .map_err(|_| format!("failed to read {}: file is not valid UTF-8", path))?;
    let proto = crate::compiler::compile_program(&source, path, options)?;
    let mut vm = VM::new();
    vm.execute_program(proto)
        .map(|_| ())
        .map_err(|err| vm.describe_error(&err))
}

/// Runs a `.nyk` source file (or a compiled `.nykb`) on the register VM:
/// parse, check, compile through the optimizing IR pipeline, execute.
pub fn run_file(path: &str) -> Result<(), String> {
    run_file_with_options(path, crate::compiler::Options::RUN)
}
