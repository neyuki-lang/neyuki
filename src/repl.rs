// Interactive REPL: read-eval-print loop over a persistent VM.
// Line-oriented: type a chunk, see its result. Unfinished chunks
// (truncated constructs, unterminated strings) continue on `>> `.

use std::io::{self, BufRead, Write};

use crate::vm::machine::VM;
use crate::vm::value::Value;

/// True when `err` looks like truncated input (hit EOF mid-construct)
/// rather than a finished-but-wrong chunk. Callers keep reading on true
/// and report the error on false. Leans toward continuing: a wrongly
/// continued chunk costs one more prompt line, while a wrongly reported
/// truncation breaks multi-line input entirely.
pub fn is_incomplete_input(err: &str) -> bool {
    err.contains("Unfinished")
        || err.contains("unfinished")
        || err.trim_end().ends_with("got \"\"")
        || (err.contains("expected ") && !err.contains("got \""))
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "internal error".to_string()
    }
}

/// Compiles and runs one chunk on a persistent VM, returning the printed
/// line for value-producing chunks (`None` for statements). Globals (and
/// the standard library) survive across chunks. Uses the unchecked IR
/// pipeline on purpose: each chunk cannot be whole-program analyzed, so
/// unknown names resolve at runtime (nil when missing), exactly like the
/// Lua REPL. Even panics from malformed input (e.g. the lexer on
/// unterminated strings) surface as errors: an interactive loop must never
/// die on input.
pub fn eval_chunk(vm: &mut VM, source: &str) -> Result<Option<String>, String> {
    let run = || -> Result<Option<String>, String> {
        let stmts = crate::compiler::compile_source(source).map_err(|e| e.to_string())?;
        let proto = crate::compiler::compile_bundled_to_proto(&stmts)?;
        let val = vm.execute(proto)?;
        Ok(match val {
            Value::Nil => None,
            v => Some(v.to_string()),
        })
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(res) => res,
        Err(payload) => Err(panic_message(payload)),
    }
}

/// Runs the interactive loop on stdin/stdout until EOF.
pub fn run_repl() -> Result<(), String> {
    let stdin = io::stdin();
    let mut out = io::stdout();
    let mut vm = VM::new();
    let mut buf = String::new();
    let mut cont = false;
    loop {
        write!(out, "{}", if cont { ">> " } else { "> " })
            .map_err(|e| format!("output error: {}", e))?;
        out.flush().map_err(|e| format!("output error: {}", e))?;
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("input error: {}", e)),
        }
        buf.push_str(&line);
        match eval_chunk(&mut vm, &buf) {
            Ok(echo) => {
                if let Some(s) = echo {
                    writeln!(out, "{}", s).map_err(|e| format!("output error: {}", e))?;
                }
                buf.clear();
                cont = false;
            }
            Err(e) if is_incomplete_input(&e) => {
                cont = true;
            }
            Err(e) => {
                writeln!(out, "Error: {}", e).map_err(|e| format!("output error: {}", e))?;
                buf.clear();
                cont = false;
            }
        }
    }
    Ok(())
}

// The test module intentionally iterates over a one-element array to keep
// the test table uniform with the other cases. Keep this lint suppression
// scoped to the module; production code and CI configuration are unchanged.
#[cfg(test)]
#[allow(clippy::for_loops_over_fallibles)]
mod tests {
    use super::*;

    fn parse_err(src: &str) -> String {
        crate::compiler::compile_source(src).unwrap_err()
    }

    #[test]
    fn incomplete_truncated_inputs_continue() {
        for src in [
            "function f(",
            "if true then",
            "local x = {1, 2",
            "print(\"hi\"",
            "local x = 1 +",
            "f(",
        ] {
            assert!(
                is_incomplete_input(&parse_err(src)),
                "should continue: {:?}",
                src
            );
        }
    }

    #[test]
    fn complete_errors_report_immediately() {
        for src in ["local = 1"] {
            assert!(
                !is_incomplete_input(&parse_err(src)),
                "should report: {:?}",
                src
            );
        }
    }

    #[test]
    fn eval_chunk_keeps_globals_across_chunks() {
        // Bare assignment writes the VM globals map, which outlives any
        // single chunk (unlike `global`, which is module-scoped).
        let mut vm = VM::new();
        eval_chunk(&mut vm, "repl_probe_xyz = 41").expect("chunk 1");
        let out = eval_chunk(&mut vm, "return repl_probe_xyz + 1").expect("chunk 2");
        assert_eq!(out.as_deref(), Some("42"));
    }

    #[test]
    fn eval_chunk_function_via_bare_assignment_persists() {
        // `name = function...` goes through SetGlobal, so it survives
        // across chunks. (The `function name...` statement form binds
        // chunk-local and does not persist: pre-existing semantics.)
        let mut vm = VM::new();
        eval_chunk(&mut vm, "f_repl_xyz = function(x)\nreturn x * 2\nend").expect("def");
        let out = eval_chunk(&mut vm, "return f_repl_xyz(21)").expect("call");
        assert_eq!(out.as_deref(), Some("42"));
    }

    #[test]
    fn eval_chunk_echo() {
        let mut vm = VM::new();
        assert_eq!(
            eval_chunk(&mut vm, "return 1 + 2")
                .expect("expr")
                .as_deref(),
            Some("3")
        );
        assert_eq!(eval_chunk(&mut vm, "local q = 1").expect("stmt"), None);
    }
}
