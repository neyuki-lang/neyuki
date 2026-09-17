use std::env;
use std::process;

pub mod ast;
#[path = "../bench/mod.rs"]
mod bench;
mod bytecode;
mod compiler;
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
        let Some(path) = args.get(2) else {
            eprintln!("Usage: {} compile <path> [output]", args[0]);
            process::exit(1);
        };
        match compiler::compile_file_to_bytecode(path) {
            Ok(bytecode) => {
                let out_path = args.get(3).cloned().unwrap_or_else(|| format!("{}b", path));
                if let Err(err) = std::fs::write(&out_path, &bytecode) {
                    eprintln!("failed to write bytecode to {}: {}", out_path, err);
                    process::exit(1);
                }
                println!(
                    "compiled to bytecode ({}) with magic 'neyuki!' ({} bytes)",
                    out_path,
                    bytecode.len()
                );
            }
            Err(err) => {
                eprintln!("{}: {}", path, err);
                process::exit(1);
            }
        }
    } else if action == "run" {
        let Some(path) = args.get(2) else {
            eprintln!("Usage: {} run <path>", args[0]);
            process::exit(1);
        };
        // Check if file is a compiled binary starting with magic bytes
        let is_bytecode = std::fs::read(path).is_ok_and(|bytes| bytes.starts_with(bytecode::MAGIC));
        if is_bytecode {
            if let Err(err) = vm::execute_bytecode_file(path) {
                eprintln!("vm error: {}", err);
                process::exit(1);
            }
        } else if let Err(err) = runtime::run_file(path) {
            eprintln!("runtime error: {}", err);
            process::exit(1);
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
