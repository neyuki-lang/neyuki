use std::env;
use std::process;

pub mod ast;
#[path = "../bench/mod.rs"]
mod bench;
mod bytecode;
mod compiler;
mod crypto_lib;
pub mod diagnostics;
mod fs_lib;
#[path = "../fuzz/mod.rs"]
mod fuzz;
mod http_lib;
mod io_lib;
mod lexer;
mod lint;
mod parser;
mod runtime;
pub mod sema;
mod sql_lib;
mod string_lib;
mod tests;
mod vm;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <action> [path]", args[0]);
        process::exit(1);
    }

    let action = &args[1];

    if action == "lint" {
        let path = args
            .get(2)
            .map(String::as_str)
            .unwrap_or("examples/hello.nyk");
        if let Err(err) = lint::lint(path) {
            eprintln!("{}", err);
            process::exit(1);
        }
    } else if action == "compile" {
        // The optimizing IR pipeline is the default; `--direct` keeps the
        // older single-pass compiler reachable for comparison.
        let use_direct = args.iter().any(|a| a == "--direct");
        let non_flag_args: Vec<&String> = args
            .iter()
            .skip(2)
            .filter(|a| !a.starts_with('-'))
            .collect();
        let Some(path) = non_flag_args.first() else {
            eprintln!("Usage: {} compile <path> [output] [--direct]", args[0]);
            process::exit(1);
        };
        let compile_res = if use_direct {
            compiler::compile_file_to_bytecode(path)
        } else {
            compiler::compile_file_to_bytecode_via_ir(path)
        };
        match compile_res {
            Ok(bytecode) => {
                let out_path = non_flag_args
                    .get(1)
                    .map(|s| (*s).clone())
                    .unwrap_or_else(|| format!("{}b", path));
                if let Err(err) = std::fs::write(&out_path, &bytecode) {
                    eprintln!("failed to write bytecode to {}: {}", out_path, err);
                    process::exit(1);
                }
                println!(
                    "compiled to bytecode ({}) via {} with magic 'neyuki!' ({} bytes)",
                    out_path,
                    if use_direct {
                        "direct compiler"
                    } else {
                        "IR CFG pipeline"
                    },
                    bytecode.len()
                );
            }
            Err(err) => {
                eprintln!("{}: {}", path, err);
                process::exit(1);
            }
        }
    } else if action == "run" {
        // Programs run on the register VM. `--tree-walker` selects the
        // original AST interpreter instead; `--opt-ir` is accepted for
        // compatibility and means the default.
        let use_tree_walker = args.iter().any(|a| a == "--tree-walker");
        let non_flag_args: Vec<&String> = args
            .iter()
            .skip(2)
            .filter(|a| !a.starts_with('-'))
            .collect();
        let Some(path) = non_flag_args.first() else {
            eprintln!("Usage: {} run <path> [--tree-walker]", args[0]);
            process::exit(1);
        };
        if use_tree_walker {
            if let Err(err) = runtime::run_file(path) {
                eprintln!("runtime error: {}", err);
                process::exit(1);
            }
        } else if let Err(err) = vm::run_file(path) {
            eprintln!("vm error: {}", err);
            process::exit(1);
        }
    } else if action == "dump-ir" {
        // Prints the IR of every function before and after optimization.
        let Some(path) = args.get(2) else {
            eprintln!("Usage: {} dump-ir <path>", args[0]);
            process::exit(1);
        };
        match compiler::compile_file(path) {
            Ok(stmts) => print!("{}", compiler::dump_ir(&stmts)),
            Err(err) => {
                eprintln!("{}: {}", path, err);
                process::exit(1);
            }
        }
    } else if action == "disasm" {
        let use_direct = args.iter().any(|a| a == "--direct");
        let non_flag_args: Vec<&String> = args
            .iter()
            .skip(2)
            .filter(|a| !a.starts_with('-'))
            .collect();
        let Some(path) = non_flag_args.first() else {
            eprintln!("Usage: {} disasm <path> [--direct]", args[0]);
            process::exit(1);
        };
        // A debugging aid: shows what the compiler emits even for sources
        // the type checker would reject.
        let proto_res = compiler::compile_file(path).and_then(|stmts| {
            if use_direct {
                compiler::try_compile_to_proto(&stmts)
            } else {
                compiler::compile_bundled_to_proto(&stmts)
            }
        });
        match proto_res {
            Ok(proto) => print!("{}", bytecode::disasm::disassemble_proto(&proto, 0)),
            Err(err) => {
                eprintln!("{}: {}", path, err);
                process::exit(1);
            }
        }
    } else if action == "test" {
        tests::run_all_tests();
    } else if action == "fuzz" {
        fuzz::run_all_fuzz_tests();
    } else if action == "bench" {
        bench::run_all_benchmarks();
    } else {
        eprintln!("Unknown action: {}", action);
        process::exit(1);
    }
}
