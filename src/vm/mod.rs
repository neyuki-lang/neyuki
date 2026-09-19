pub mod buffer;
pub mod builtins;
pub mod frame;
pub mod gc;
pub mod hash;
pub mod libs;
pub mod machine;
pub mod ops;
pub mod value;

pub use machine::VM;
use std::fs;
pub use value::Value;

use crate::bytecode::deserialize;

// Execute compiled bytecode buffer
pub fn execute_bytecode(bytes: &[u8]) -> Result<Value, String> {
    let proto = deserialize(bytes)?;
    let mut vm = VM::new();
    vm.execute(proto)
}

/// Runs a `.nyk` source file (or a compiled `.nykb`) on the register VM:
/// parse, check, compile through the optimizing IR pipeline, execute.
pub fn run_file(path: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read {}: {}", path, e))?;
    if bytes.starts_with(crate::bytecode::MAGIC) {
        return execute_bytecode(&bytes).map(|_| ());
    }
    let source = String::from_utf8(bytes)
        .map_err(|_| format!("failed to read {}: file is not valid UTF-8", path))?;
    let stmts =
        crate::compiler::compile_source(&source).map_err(|e| format!("syntax error: {}", e))?;
    let proto = crate::compiler::try_compile_to_proto_via_ir(&stmts)?;
    let mut vm = VM::new();
    vm.execute(proto).map(|_| ())
}
