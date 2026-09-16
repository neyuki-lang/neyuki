// Bytecode verification pass ensuring integrity and safety of deserialized Prototypes.
// Prevents out-of-bounds register accesses, invalid constant pool indices,
// corrupted branch offsets, and invalid nested prototype references.

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::Proto;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytecodeVerifyError {
    pub message: String,
    pub proto_name: Option<String>,
    pub pc: Option<usize>,
}

impl std::fmt::Display for BytecodeVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.proto_name.as_deref().unwrap_or("<anonymous>");
        if let Some(pc) = self.pc {
            write!(f, "bytecode verification failed in '{}' at pc [{}]: {}", name, pc, self.message)
        } else {
            write!(f, "bytecode verification failed in '{}': {}", name, self.message)
        }
    }
}

impl std::error::Error for BytecodeVerifyError {}

pub fn verify_proto(proto: &Proto) -> Result<(), BytecodeVerifyError> {
    let name = proto.name.clone();
    let num_constants = proto.constants.len();
    let num_protos = proto.protos.len();
    let num_upvalues = proto.upvalues.len();
    let num_insts = proto.instructions.len();

    let check_reg = |reg: u8, pc: usize| -> Result<(), BytecodeVerifyError> {
        if proto.max_registers > 0 && reg >= proto.max_registers {
            return Err(BytecodeVerifyError {
                message: format!("register R{} exceeds prototype max_registers ({})", reg, proto.max_registers),
                proto_name: name.clone(),
                pc: Some(pc),
            });
        }
        Ok(())
    };

    let check_const = |k: u16, pc: usize| -> Result<(), BytecodeVerifyError> {
        if k as usize >= num_constants {
            return Err(BytecodeVerifyError {
                message: format!("constant index K{} exceeds constant pool size ({})", k, num_constants),
                proto_name: name.clone(),
                pc: Some(pc),
            });
        }
        Ok(())
    };

    let check_jump = |offset: i16, pc: usize| -> Result<(), BytecodeVerifyError> {
        let target_ip = pc as isize + 1 + offset as isize;
        if target_ip < 0 || target_ip > num_insts as isize {
            return Err(BytecodeVerifyError {
                message: format!("jump offset {} targets invalid instruction index {} (total: {})", offset, target_ip, num_insts),
                proto_name: name.clone(),
                pc: Some(pc),
            });
        }
        Ok(())
    };

    for (pc, inst) in proto.instructions.iter().enumerate() {
        match inst {
            Instruction::LoadNil { dst } => {
                check_reg(*dst, pc)?;
            }
            Instruction::LoadBool { dst, .. } => {
                check_reg(*dst, pc)?;
            }
            Instruction::LoadInt { dst, .. } => {
                check_reg(*dst, pc)?;
            }
            Instruction::LoadK { dst, k } => {
                check_reg(*dst, pc)?;
                check_const(*k, pc)?;
            }
            Instruction::Move { dst, src } => {
                check_reg(*dst, pc)?;
                check_reg(*src, pc)?;
            }
            Instruction::Add { dst, a, b }
            | Instruction::Sub { dst, a, b }
            | Instruction::Mul { dst, a, b }
            | Instruction::Div { dst, a, b }
            | Instruction::IDiv { dst, a, b }
            | Instruction::Mod { dst, a, b }
            | Instruction::Pow { dst, a, b }
            | Instruction::BitAnd { dst, a, b }
            | Instruction::BitOr { dst, a, b }
            | Instruction::BitXor { dst, a, b }
            | Instruction::Shl { dst, a, b }
            | Instruction::Shr { dst, a, b }
            | Instruction::Concat { dst, a, b } => {
                check_reg(*dst, pc)?;
                check_reg(*a, pc)?;
                check_reg(*b, pc)?;
            }
            Instruction::Unm { dst, src }
            | Instruction::Not { dst, src }
            | Instruction::Len { dst, src }
            | Instruction::BitNot { dst, src } => {
                check_reg(*dst, pc)?;
                check_reg(*src, pc)?;
            }
            Instruction::Coalesce { dst, a, b } => {
                check_reg(*dst, pc)?;
                check_reg(*a, pc)?;
                check_reg(*b, pc)?;
            }
            Instruction::NewTable { dst } => {
                check_reg(*dst, pc)?;
            }
            Instruction::GetTable { dst, table, key } => {
                check_reg(*dst, pc)?;
                check_reg(*table, pc)?;
                check_reg(*key, pc)?;
            }
            Instruction::SetTable { table, key, val } => {
                check_reg(*table, pc)?;
                check_reg(*key, pc)?;
                check_reg(*val, pc)?;
            }
            Instruction::GetTableK { dst, table, key_k } => {
                check_reg(*dst, pc)?;
                check_reg(*table, pc)?;
                check_const(*key_k, pc)?;
            }
            Instruction::SetTableK { table, key_k, val } => {
                check_reg(*table, pc)?;
                check_const(*key_k, pc)?;
                check_reg(*val, pc)?;
            }
            Instruction::AppendArray { table, src } => {
                check_reg(*table, pc)?;
                check_reg(*src, pc)?;
            }
            Instruction::SetList { table, base, count } => {
                check_reg(*table, pc)?;
                if (*base as usize) + (*count as usize) > 256 {
                    return Err(BytecodeVerifyError {
                        message: format!("SetList base {} + count {} exceeds register address space", base, count),
                        proto_name: name.clone(),
                        pc: Some(pc),
                    });
                }
            }
            Instruction::GetGlobal { dst, name_k } => {
                check_reg(*dst, pc)?;
                check_const(*name_k, pc)?;
            }
            Instruction::SetGlobal { src, name_k } => {
                check_reg(*src, pc)?;
                check_const(*name_k, pc)?;
            }
            Instruction::GetUpval { dst, upval_idx } => {
                check_reg(*dst, pc)?;
                if *upval_idx as usize >= num_upvalues {
                    return Err(BytecodeVerifyError {
                        message: format!("upvalue index {} exceeds prototype upvalues count ({})", upval_idx, num_upvalues),
                        proto_name: name.clone(),
                        pc: Some(pc),
                    });
                }
            }
            Instruction::SetUpval { src, upval_idx } => {
                check_reg(*src, pc)?;
                if *upval_idx as usize >= num_upvalues {
                    return Err(BytecodeVerifyError {
                        message: format!("upvalue index {} exceeds prototype upvalues count ({})", upval_idx, num_upvalues),
                        proto_name: name.clone(),
                        pc: Some(pc),
                    });
                }
            }
            Instruction::Closure { dst, proto_idx } => {
                check_reg(*dst, pc)?;
                if *proto_idx as usize >= num_protos {
                    return Err(BytecodeVerifyError {
                        message: format!("closure prototype index {} exceeds protos count ({})", proto_idx, num_protos),
                        proto_name: name.clone(),
                        pc: Some(pc),
                    });
                }
            }
            Instruction::Call { callee, argc, retc } => {
                check_reg(*callee, pc)?;
                let max_slot = (*callee as usize) + (*argc as usize).max(*retc as usize);
                if max_slot > 256 {
                    return Err(BytecodeVerifyError {
                        message: format!("call callee R{} with argc {} / retc {} exceeds register space", callee, argc, retc),
                        proto_name: name.clone(),
                        pc: Some(pc),
                    });
                }
            }
            Instruction::Return { base, count } => {
                if *count > 0 {
                    check_reg(*base, pc)?;
                }
            }
            Instruction::Vararg { dst, count: _ } => {
                check_reg(*dst, pc)?;
            }
            Instruction::Jump { offset } => {
                check_jump(*offset, pc)?;
            }
            Instruction::Test { reg, jump_if_false } => {
                check_reg(*reg, pc)?;
                check_jump(*jump_if_false, pc)?;
            }
            Instruction::Eq { a, b, jump_if_false }
            | Instruction::Ne { a, b, jump_if_false }
            | Instruction::Lt { a, b, jump_if_false }
            | Instruction::Le { a, b, jump_if_false }
            | Instruction::Gt { a, b, jump_if_false }
            | Instruction::Ge { a, b, jump_if_false } => {
                check_reg(*a, pc)?;
                check_reg(*b, pc)?;
                check_jump(*jump_if_false, pc)?;
            }
            Instruction::ForPrep { base, jump } => {
                check_reg(*base, pc)?;
                check_jump(*jump, pc)?;
            }
            Instruction::ForLoop { base, jump } => {
                check_reg(*base, pc)?;
                check_jump(*jump, pc)?;
            }
            Instruction::TForCall { base, retc: _ } => {
                check_reg(*base, pc)?;
            }
            Instruction::TForLoop { base, jump } => {
                check_reg(*base, pc)?;
                check_jump(*jump, pc)?;
            }
        }
    }

    // Recursively verify all nested prototypes
    for child in &proto.protos {
        verify_proto(child)?;
    }

    Ok(())
}
