// Comprehensive Fuzzing Suite for Neyuki Programming Language.
// Covers AST tree fuzzing, malformed bytecode files, memory safety / buffer overflow attacks,
// VM execution robustness (divide-by-zero, shifts, coroutines, metatables, sorting),
// and standard library edge cases (JSON, UTF-8, tonumber, crypto, sandbox).

pub mod file;
pub mod libs;
pub mod memory;
pub mod tree;
pub mod vm;

#[allow(dead_code)]
pub fn run_all_fuzz_tests() {
    println!("=== Running Neyuki Fuzz Suite ===");

    println!("-> Fuzzing AST syntax trees...");
    tree::ast_fuzz::fuzz_nested_parentheses();
    tree::ast_fuzz::fuzz_operator_chains();
    tree::ast_fuzz::fuzz_malformed_interpolations();
    tree::ast_fuzz::fuzz_unbalanced_delimiters();

    println!("-> Fuzzing Bytecode binary formats...");
    file::bytecode_fuzz::fuzz_truncated_bytecode();
    file::bytecode_fuzz::fuzz_corrupted_magic();
    file::bytecode_fuzz::fuzz_corrupted_instructions();
    file::bytecode_fuzz::fuzz_out_of_bounds_jumps();
    file::bytecode_fuzz::fuzz_random_byte_streams();

    println!("-> Fuzzing Memory safety and Cyclic GC...");
    memory::mem_fuzz::fuzz_buffer_integer_overflow();
    memory::mem_fuzz::fuzz_buffer_copy_boundaries();
    memory::mem_fuzz::fuzz_gc_cyclic_stress();
    memory::mem_fuzz::fuzz_vm_recursion_protection();

    println!("-> Fuzzing VM execution & arithmetic robustness...");
    vm::vm_fuzz::fuzz_arithmetic_div_by_zero_and_overflow();
    vm::vm_fuzz::fuzz_coroutine_lifecycle_and_reentrancy();
    vm::vm_fuzz::fuzz_metatable_recursion_and_cycles();
    vm::vm_fuzz::fuzz_table_sort_inconsistent_comparators();
    vm::vm_fuzz::fuzz_instruction_boundary_and_stack();

    println!("-> Fuzzing Standard Libraries & Sandbox...");
    libs::lib_fuzz::fuzz_json_parser_and_nesting();
    libs::lib_fuzz::fuzz_string_and_utf8_edge_cases();
    libs::lib_fuzz::fuzz_number_parsing_and_radix();
    libs::lib_fuzz::fuzz_crypto_sha256_boundaries();
    libs::lib_fuzz::fuzz_os_getenv_sandbox_leakage();

    println!("=== All Fuzz Tests Passed Cleanly ===");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_all_fuzz_suite() {
        run_all_fuzz_tests();
    }
}
