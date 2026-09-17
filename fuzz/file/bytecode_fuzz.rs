#![allow(dead_code)]

// Bytecode binary deserializer and verifier fuzz tests.
// Ensures corrupt, truncated, or hostile binary inputs never cause panics or memory crashes.

use crate::bytecode::{deserialize, serialize, MAGIC};
use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::Proto;

pub fn fuzz_truncated_bytecode() {
    let mut proto = Proto::new(Some("test".to_string()), 0, false);
    proto.max_registers = 4;
    proto.instructions.push(Instruction::LoadInt { dst: 0, val: 42 });
    proto.instructions.push(Instruction::Return { base: 0, count: 1 });
    let valid_bytes = serialize(&proto);

    // Truncate binary at every single byte index from 0 to full length
    for len in 0..valid_bytes.len() {
        let truncated = &valid_bytes[..len];
        let res = deserialize(truncated);
        assert!(res.is_err(), "truncated bytecode of len {} must be rejected", len);
    }
}

pub fn fuzz_corrupted_magic() {
    let mut proto = Proto::new(Some("test".to_string()), 0, false);
    proto.max_registers = 2;
    proto.instructions.push(Instruction::LoadNil { dst: 0 });
    let mut bytes = serialize(&proto);

    // Flip each byte of the magic header
    for i in 0..MAGIC.len() {
        bytes[i] ^= 0xFF;
        let res = deserialize(&bytes);
        assert!(res.is_err(), "corrupted magic at index {} must be rejected", i);
        bytes[i] ^= 0xFF; // restore
    }
}

pub fn fuzz_corrupted_instructions() {
    let mut proto = Proto::new(Some("test".to_string()), 0, false);
    proto.max_registers = 1; // Only R0 is valid
    // Instruction accesses R10 which is > max_registers (1)
    proto.instructions.push(Instruction::LoadNil { dst: 10 });
    let bytes = serialize(&proto);

    let res = deserialize(&bytes);
    assert!(res.is_err(), "bytecode with dst register out of bounds must fail verification");
}

pub fn fuzz_out_of_bounds_jumps() {
    let mut proto = Proto::new(Some("test".to_string()), 0, false);
    proto.max_registers = 2;
    // Jump forward by +1000 past the instruction stream
    proto.instructions.push(Instruction::Jump { offset: 1000 });
    proto.instructions.push(Instruction::Return { base: 0, count: 1 });
    let bytes = serialize(&proto);

    let res = deserialize(&bytes);
    assert!(res.is_err(), "out-of-bounds jump offset must fail verification");
}

pub fn fuzz_random_byte_streams() {
    // 50 iterations of pseudo-random byte streams of varying lengths
    let mut seed = 0x12345678u64;
    for len in [0, 1, 4, 7, 16, 32, 64, 128, 256, 512] {
        let mut random_bytes = Vec::with_capacity(len);
        for _ in 0..len {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            random_bytes.push((seed >> 32) as u8);
        }
        // Must never panic on random input
        let _ = deserialize(&random_bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_truncated_bytecode() {
        fuzz_truncated_bytecode();
    }

    #[test]
    fn test_file_corrupted_magic() {
        fuzz_corrupted_magic();
    }

    #[test]
    fn test_file_corrupted_instructions() {
        fuzz_corrupted_instructions();
    }

    #[test]
    fn test_file_out_of_bounds_jumps() {
        fuzz_out_of_bounds_jumps();
    }

    #[test]
    fn test_file_random_byte_streams() {
        fuzz_random_byte_streams();
    }
}
