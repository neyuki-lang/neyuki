pub mod disasm;
pub mod instruction;
pub mod parser;
pub mod proto;
pub mod serialize;

#[allow(unused_imports)]
pub use disasm::disassemble_proto;
#[allow(unused_imports)]
pub use instruction::Instruction;
#[allow(unused_imports)]
pub use parser::parse_assembly;
#[allow(unused_imports)]
pub use proto::{Constant, Proto, UpvalueDesc};
#[allow(unused_imports)]
pub use serialize::{deserialize, serialize, BYTECODE_VERSION, MAGIC};
