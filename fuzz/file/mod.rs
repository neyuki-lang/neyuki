// File and Binary Bytecode Deserializer Fuzzer for Neyuki.
// Tests resilience against corrupted magic headers, truncated byte streams,
// invalid opcodes, out-of-range registers, and illegal jump targets.

pub mod bytecode_fuzz;
