// Memory Safety, GC Cycle, and Buffer Overflow Fuzzer for Neyuki.
// Tests integer overflow bypasses in buffers, extreme allocations,
// complex cyclic references in Tricolor GC, and stack depth protections.

pub mod mem_fuzz;
