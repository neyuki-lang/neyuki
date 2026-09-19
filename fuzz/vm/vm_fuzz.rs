#![allow(dead_code)]

// VM Execution, Arithmetic, Coroutine, and Metatable Fuzz Suite.
// Fuzzes instruction boundaries, arithmetic exceptions (divide-by-zero, shifts),
// coroutine lifecycle, metatable loop prevention, and sorting robustness.

use crate::compiler::{compile_source, compile_to_proto};
use crate::vm::machine::VM;
use crate::vm::value::Value;

fn execute_source(source: &str) -> Result<Value, String> {
    let stmts = compile_source(source)?;
    let proto = compile_to_proto(&stmts);
    let mut vm = VM::new();
    vm.execute(proto)
}

pub fn fuzz_arithmetic_div_by_zero_and_overflow() {
    let zero_div_snippets = [
        "local x = 10 / 0; return x",
        "local x = 10 // 0; return x",
        "local x = 10 % 0; return x",
        "local x = 0 / 0; return x",
        "local x = 0 // 0; return x",
        "local x = 0 % 0; return x",
        "local x = -10 / 0; return x",
        "local x = -10 // 0; return x",
        "local x = -10 % 0; return x",
        "local x = 1.5 / 0.0; return x",
        "local x = -1.5 / 0.0; return x",
        "local x = 0.0 / 0.0; return x",
    ];

    for snippet in &zero_div_snippets {
        // Must execute or fail cleanly with error, never cause a hardware/OS division panic
        let _ = execute_source(snippet);
    }

    // Fuzz extreme shifts
    let shift_snippets = [
        "local x = 1 << 0; return x",
        "local x = 1 >> 0; return x",
        "local x = 1 << 31; return x",
        "local x = 1 << 32; return x",
        "local x = 1 << 63; return x",
        "local x = 1 << 64; return x",
        "local x = 1 >> 64; return x",
        "local x = 1 << 1000; return x",
        "local x = 1 >> 1000; return x",
        "local x = 1 << -1; return x",
        "local x = 1 >> -1; return x",
        "local x = 1 << -64; return x",
    ];

    for snippet in &shift_snippets {
        let _ = execute_source(snippet);
    }
}

pub fn fuzz_coroutine_lifecycle_and_reentrancy() {
    // 1. Resuming dead coroutine
    let dead_co_script = r#"
        local co = require("@neyuki/coroutine")
        local t = co.create(function(x) return x + 1 end)
        local ok1, res1 = co.resume(t, 10)
        assert(ok1 == true and res1 == 11)
        assert(co.status(t) == "dead")
        local ok2, err2 = co.resume(t, 20)
        assert(ok2 == false)
        assert(co.status(t) == "dead")
    "#;
    assert!(execute_source(dead_co_script).is_ok());

    // 2. Yielding outside coroutine
    let yield_outside = r#"
        local co = require("@neyuki/coroutine")
        co.yield(123)
    "#;
    assert!(execute_source(yield_outside).is_err());

    // 3. Coroutine error propagation
    let err_co = r#"
        local co = require("@neyuki/coroutine")
        local t = co.create(function()
            error("custom coroutine failure")
        end)
        local ok, msg = co.resume(t)
        assert(ok == false)
        assert(co.status(t) == "dead")
    "#;
    assert!(execute_source(err_co).is_ok());

    // 4. Multi-value yield and resume
    let multi_val_co = r#"
        local co = require("@neyuki/coroutine")
        local t = co.create(function(a, b, c)
            local x, y = co.yield(a * 2, b * 3, c * 4)
            return x + y
        end)
        local ok, r1, r2, r3 = co.resume(t, 1, 2, 3)
        assert(ok == true and r1 == 2 and r2 == 6 and r3 == 12)
        local ok2, r4 = co.resume(t, 10, 20)
        assert(ok2 == true and r4 == 30)
    "#;
    assert!(execute_source(multi_val_co).is_ok());

    // 5. Nested coroutines
    let nested_co = r#"
        local co = require("@neyuki/coroutine")
        local inner = co.create(function()
            co.yield(42)
            return 99
        end)
        local outer = co.create(function()
            local ok1, val1 = co.resume(inner)
            co.yield(val1 * 2)
            local ok2, val2 = co.resume(inner)
            return val2 * 2
        end)
        local ok1, v1 = co.resume(outer)
        assert(ok1 == true and v1 == 84)
        local ok2, v2 = co.resume(outer)
        assert(ok2 == true and v2 == 198)
    "#;
    assert!(execute_source(nested_co).is_ok());
}

pub fn fuzz_metatable_recursion_and_cycles() {
    // 1. Mutually recursive __index metatables
    let index_cycle = r#"
        local a = {}
        local b = {}
        setmetatable(a, { __index = b })
        setmetatable(b, { __index = a })
        local x = a.non_existent_key
        return x
    "#;
    // Must return nil or error gracefully on recursion limit, never stack overflow
    let _ = execute_source(index_cycle);

    // 2. Metatable __call on non-function
    let bad_call = r#"
        local t = {}
        setmetatable(t, { __call = 123 })
        t()
    "#;
    assert!(execute_source(bad_call).is_err());
}

pub fn fuzz_table_sort_inconsistent_comparators() {
    // 1. Comparator always returning true (violates strict weak ordering)
    let bad_comp1 = r#"
        local table = require("@neyuki/table")
        local t = {5, 2, 8, 1, 9, 3, 7, 4, 6}
        table.sort(t, function(a, b) return true end)
        return #t
    "#;
    assert!(execute_source(bad_comp1).is_ok());

    // 2. Comparator always returning false
    let bad_comp2 = r#"
        local table = require("@neyuki/table")
        local t = {5, 2, 8, 1, 9, 3, 7, 4, 6}
        table.sort(t, function(a, b) return false end)
        return #t
    "#;
    assert!(execute_source(bad_comp2).is_ok());

    // 3. Sorting 1000 items with valid reverse comparator
    let large_sort = r#"
        local table = require("@neyuki/table")
        local t = {}
        for i = 1, 1000 do
            t[i] = 1000 - i
        end
        table.sort(t)
        assert(t[1] == 0)
        assert(t[1000] == 999)
        return #t
    "#;
    assert!(execute_source(large_sort).is_ok());
}

pub fn fuzz_instruction_boundary_and_stack() {
    // 1. Invoking non-callable types directly
    let non_callables = [
        "local f = nil; f()",
        "local f = 123; f()",
        "local f = true; f()",
        "local f = \"hello\"; f()",
        "local f = {}; f()",
    ];
    for snippet in &non_callables {
        assert!(
            execute_source(snippet).is_err(),
            "calling non-callable must fail cleanly"
        );
    }

    // 2. Varargs passing and unpacking
    let vararg_code = r#"
        local function sum(a, b, ...)
            local c = ...
            return a + b + c
        end
        assert(sum(10, 20, 30) == 60)
        assert(sum(1, 2, 3, 4, 5) == 6)
    "#;
    let res = execute_source(vararg_code);
    assert!(res.is_ok(), "vararg error: {:?}", res.err());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_fuzz_all() {
        fuzz_arithmetic_div_by_zero_and_overflow();
        fuzz_coroutine_lifecycle_and_reentrancy();
        fuzz_metatable_recursion_and_cycles();
        fuzz_table_sort_inconsistent_comparators();
        fuzz_instruction_boundary_and_stack();
    }
}
