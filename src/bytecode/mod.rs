pub mod deserialize;
pub mod disasm;
pub mod format;
pub mod instruction;
pub mod parser;
pub mod proto;
pub mod serialize;
pub mod verify;

#[allow(unused_imports)]
pub use deserialize::deserialize;
#[allow(unused_imports)]
pub use disasm::disassemble_proto;
#[allow(unused_imports)]
pub use format::{BytecodeHeader, BYTECODE_VERSION, MAGIC};
#[allow(unused_imports)]
pub use instruction::Instruction;
#[allow(unused_imports)]
pub use parser::parse_assembly;
#[allow(unused_imports)]
pub use proto::{Constant, Proto, UpvalueDesc};
#[allow(unused_imports)]
pub use serialize::serialize;
#[allow(unused_imports)]
pub use verify::{verify_proto, BytecodeVerifyError};
