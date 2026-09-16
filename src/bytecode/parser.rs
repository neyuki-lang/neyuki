// Bytecode textual assembly parser for loading disassemblies back into Prototypes.

use num_bigint::BigInt;
use std::str::FromStr;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::{Constant, Proto};

#[allow(dead_code)]
pub fn parse_assembly(text: &str) -> Result<Proto, String> {
    let mut proto = Proto::new(Some("main".to_string()), 0, false);
    let mut current_line = 1u32;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }

        if let Some(after_proto) = line.strip_prefix(".proto") {
            // Header information: e.g. .proto main (params: 0, registers: 4, vararg: false)
            let after_proto = after_proto.trim();
            if let Some(paren) = after_proto.find('(') {
                let name = after_proto[..paren].trim();
                if !name.is_empty() {
                    proto.name = Some(name.to_string());
                }
            }
            if let Some(params_start) = line.find("params: ") {
                let rest = &line[params_start + 8..];
                if let Some(comma) = rest.find(',') {
                    proto.num_params = rest[..comma].trim().parse().unwrap_or(0);
                }
            }
            if let Some(reg_start) = line.find("registers: ") {
                let rest = &line[reg_start + 11..];
                if let Some(comma) = rest.find(',') {
                    proto.max_registers = rest[..comma].trim().parse().unwrap_or(0);
                }
            }
            continue;
        }

        if line.starts_with(".const") {
            // E.g. .const K0 = int 10
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            if parts.len() >= 4 && parts[2] == "=" {
                let type_and_val = parts[3];
                if type_and_val == "nil" {
                    proto.constants.push(Constant::Nil);
                } else if let Some(rest) = type_and_val.strip_prefix("boolean ") {
                    let b = rest.trim() == "true";
                    proto.constants.push(Constant::Bool(b));
                } else if let Some(rest) = type_and_val.strip_prefix("int ") {
                    let s = rest.trim();
                    let bi = BigInt::from_str(s).unwrap_or_default();
                    proto.constants.push(Constant::Int(bi));
                } else if let Some(rest) = type_and_val.strip_prefix("float ") {
                    let s = rest.trim();
                    let f = s.parse::<f64>().unwrap_or(0.0);
                    proto.constants.push(Constant::Float(f));
                } else if let Some(rest) = type_and_val.strip_prefix("string ") {
                    let s = rest.trim();
                    let unquoted = s.trim_matches('"');
                    proto.constants.push(Constant::String(unquoted.to_string()));
                }
            }
            continue;
        }

        // Instruction line: e.g. 0000 [L001] LOADINT R0, 10
        let inst_str = if let Some(bracket_end) = line.find(']') {
            line[bracket_end + 1..].trim()
        } else {
            line
        };

        if let Some(inst) = parse_instruction_line(inst_str)? {
            proto.emit(inst, current_line);
            current_line += 1;
        }
    }

    Ok(proto)
}

fn parse_reg(token: &str) -> Result<u8, String> {
    let clean = token.trim().trim_end_matches(',');
    if let Some(reg_num) = clean.strip_prefix('R') {
        reg_num
            .parse::<u8>()
            .map_err(|_| format!("invalid register {}", clean))
    } else {
        Err(format!("expected register prefix 'R', got {}", clean))
    }
}

fn parse_const_idx(token: &str) -> Result<u16, String> {
    let clean = token.trim().trim_end_matches(',');
    if let Some(idx_str) = clean.strip_prefix('K') {
        idx_str
            .parse::<u16>()
            .map_err(|_| format!("invalid constant index {}", clean))
    } else {
        Err(format!("expected constant prefix 'K', got {}", clean))
    }
}

fn parse_instruction_line(line: &str) -> Result<Option<Instruction>, String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(None);
    }

    let op = parts[0];
    match op {
        "LOADNIL" => {
            let dst = parse_reg(parts[1])?;
            Ok(Some(Instruction::LoadNil { dst }))
        }
        "LOADBOOL" => {
            let dst = parse_reg(parts[1])?;
            let val = parts[2].trim().trim_end_matches(',') == "true";
            Ok(Some(Instruction::LoadBool { dst, val }))
        }
        "LOADINT" => {
            let dst = parse_reg(parts[1])?;
            let val = parts[2].trim().trim_end_matches(',').parse::<i32>().map_err(|e| e.to_string())?;
            Ok(Some(Instruction::LoadInt { dst, val }))
        }
        "LOADK" => {
            let dst = parse_reg(parts[1])?;
            let k = parse_const_idx(parts[2])?;
            Ok(Some(Instruction::LoadK { dst, k }))
        }
        "MOVE" => {
            let dst = parse_reg(parts[1])?;
            let src = parse_reg(parts[2])?;
            Ok(Some(Instruction::Move { dst, src }))
        }
        "ADD" => {
            let dst = parse_reg(parts[1])?;
            let a = parse_reg(parts[2])?;
            let b = parse_reg(parts[3])?;
            Ok(Some(Instruction::Add { dst, a, b }))
        }
        "SUB" => {
            let dst = parse_reg(parts[1])?;
            let a = parse_reg(parts[2])?;
            let b = parse_reg(parts[3])?;
            Ok(Some(Instruction::Sub { dst, a, b }))
        }
        "MUL" => {
            let dst = parse_reg(parts[1])?;
            let a = parse_reg(parts[2])?;
            let b = parse_reg(parts[3])?;
            Ok(Some(Instruction::Mul { dst, a, b }))
        }
        "DIV" => {
            let dst = parse_reg(parts[1])?;
            let a = parse_reg(parts[2])?;
            let b = parse_reg(parts[3])?;
            Ok(Some(Instruction::Div { dst, a, b }))
        }
        "BITAND" => {
            let dst = parse_reg(parts[1])?;
            let a = parse_reg(parts[2])?;
            let b = parse_reg(parts[3])?;
            Ok(Some(Instruction::BitAnd { dst, a, b }))
        }
        "BITOR" => {
            let dst = parse_reg(parts[1])?;
            let a = parse_reg(parts[2])?;
            let b = parse_reg(parts[3])?;
            Ok(Some(Instruction::BitOr { dst, a, b }))
        }
        "BITXOR" => {
            let dst = parse_reg(parts[1])?;
            let a = parse_reg(parts[2])?;
            let b = parse_reg(parts[3])?;
            Ok(Some(Instruction::BitXor { dst, a, b }))
        }
        "BITNOT" => {
            let dst = parse_reg(parts[1])?;
            let src = parse_reg(parts[2])?;
            Ok(Some(Instruction::BitNot { dst, src }))
        }
        "RETURN" => {
            let base = parse_reg(parts[1])?;
            let count = if parts.len() > 3 && parts[2] == "count" {
                parts[3].parse::<u8>().unwrap_or(1)
            } else {
                1
            };
            Ok(Some(Instruction::Return { base, count }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::disasm::disassemble_proto;

    #[test]
    fn test_disasm_and_parse_assembly() {
        let mut proto = Proto::new(Some("test_func".to_string()), 2, false);
        proto.max_registers = 8;
        proto.instructions.push(Instruction::LoadInt { dst: 0, val: 42 });
        proto.instructions.push(Instruction::Return { base: 0, count: 1 });

        let asm = disassemble_proto(&proto, 0);
        let parsed = parse_assembly(&asm).expect("failed to parse assembly");
        assert_eq!(parsed.name, proto.name);
        assert_eq!(parsed.max_registers, proto.max_registers);
        assert_eq!(parsed.instructions.len(), 2);
    }
}
