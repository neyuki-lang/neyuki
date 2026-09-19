#![allow(dead_code)]

// AST and Syntax Tree fuzz tests.
// Exercises parser resilience against unbounded depth, invalid syntax,
// malformed interpolation, and token corruption without panicking.

use crate::compiler::compile_source;
use crate::parser::Parser;

pub fn fuzz_nested_parentheses() {
    // Tests depths from 1 up to 200
    for depth in [10, 50, 100, 120, 150, 200] {
        let mut expr = "42".to_string();
        for _ in 0..depth {
            expr = format!("({})", expr);
        }
        let res = compile_source(&expr);
        if depth >= 120 {
            assert!(
                res.is_err(),
                "depth {} should be rejected by depth limit",
                depth
            );
        } else {
            assert!(res.is_ok(), "depth {} should parse successfully", depth);
        }
    }
}

pub fn fuzz_operator_chains() {
    // Tests long chains of binary arithmetic and bitwise expressions
    for count in [10, 50, 100, 200] {
        let mut expr = "1".to_string();
        for i in 1..count {
            let op = match i % 5 {
                0 => "+",
                1 => "-",
                2 => "*",
                3 => "&",
                _ => "|",
            };
            expr.push_str(&format!(" {} {}", op, i));
        }
        let res = compile_source(&format!("local x = {}", expr));
        assert!(
            res.is_ok(),
            "long operator chain with {} ops should compile cleanly",
            count
        );
    }
}

pub fn fuzz_malformed_interpolations() {
    let bad_inputs = [
        "local s = `hello {world`",
        "local s = `hello {`",
        "local s = `hello {a; b}`",
        "local s = `hello {if x then end}`",
        "local s = `hello {a, b, c}`",
        "local s = `hello {for i in t do end}`",
        "local s = `hello {{{{{{`",
    ];

    for input in &bad_inputs {
        let res = compile_source(input);
        assert!(
            res.is_err(),
            "malformed interpolation '{}' must fail compilation",
            input
        );
    }
}

pub fn fuzz_unbalanced_delimiters() {
    let unbalanced = [
        "(",
        ")",
        "((())",
        "{",
        "}",
        "{{}",
        "[",
        "]",
        "if true then",
        "while true do",
        "for i = 1, 10 do",
        "function foo(",
        "local t = { a = 1, b = ",
    ];

    for snippet in &unbalanced {
        let mut parser = Parser::new(snippet);
        let res = parser.parse_program();
        assert!(
            res.is_err(),
            "unbalanced code '{}' must return Err, never panic",
            snippet
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_nested_parentheses() {
        fuzz_nested_parentheses();
    }

    #[test]
    fn test_tree_operator_chains() {
        fuzz_operator_chains();
    }

    #[test]
    fn test_tree_malformed_interpolations() {
        fuzz_malformed_interpolations();
    }

    #[test]
    fn test_tree_unbalanced_delimiters() {
        fuzz_unbalanced_delimiters();
    }
}
