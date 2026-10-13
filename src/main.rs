use std::env;
use std::process;

#[path = "../bench/mod.rs"]
mod bench;
#[path = "../fuzz/mod.rs"]
mod fuzz;

use neyuki::*;

/// Stack for the thread that runs every action. The parser caps nesting, but
/// the passes after it recurse once per level with large debug-build frames,
/// and the main thread only gets 1 MiB on Windows. This only reserves address
/// space; pages are committed as they are touched.
const MAIN_STACK_SIZE: usize = 256 * 1024 * 1024;

fn main() {
    let worker = std::thread::Builder::new()
        .name("main".to_string())
        .stack_size(MAIN_STACK_SIZE)
        .spawn(run_cli)
        .unwrap_or_else(|err| {
            eprintln!("failed to start the main thread: {}", err);
            process::exit(1);
        });
    if worker.join().is_err() {
        process::exit(101);
    }
}

fn run_cli() {
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
                    "compiled {} -> {} ({} bytes)",
                    path,
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
            eprintln!("Usage: {} run <path> [args...]", args[0]);
            process::exit(1);
        };
        if let Err(err) = vm::run_file(path) {
            eprintln!("error: {}", err);
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
        let Some(path) = args.get(2) else {
            eprintln!("Usage: {} disasm <path>", args[0]);
            process::exit(1);
        };
        // A debugging aid: shows what the compiler emits even for sources
        // the type checker would reject.
        let compiled = std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path, err))
            .and_then(|source| {
                compiler::compile_program(&source, path, compiler::Options::UNCHECKED)
            });
        match compiled {
            Ok(proto) => print!("{}", bytecode::disasm::disassemble_proto(&proto, 0)),
            Err(err) => {
                eprintln!("{}: {}", path, err);
                process::exit(1);
            }
        }
    } else if action == "assemble" {
        let Some(path) = args.get(2) else {
            eprintln!("Usage: {} assemble <assembly_file> [output]", args[0]);
            process::exit(1);
        };
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("failed to read {}: {}", path, e);
                process::exit(1);
            }
        };
        let proto = match bytecode::parse_assembly(&text) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("assembly parse error: {}", e);
                process::exit(1);
            }
        };
        let out_path = args
            .get(3)
            .cloned()
            .unwrap_or_else(|| format!("{}.bin", path));
        let serialized = bytecode::serialize(&proto);
        if let Err(e) = std::fs::write(&out_path, &serialized) {
            eprintln!("failed to write {}: {}", out_path, e);
            process::exit(1);
        }
        println!(
            "assembled {} -> {} ({} bytes)",
            path,
            out_path,
            serialized.len()
        );
    } else if action == "test" {
        tests::run_all_tests();
    } else if action == "fuzz" {
        fuzz::run_all_fuzz_tests();
    } else if action == "bench" {
        bench::run_all_benchmarks();
    } else if action == "repl" {
        if let Err(err) = repl::run_repl() {
            eprintln!("repl error: {}", err);
            process::exit(1);
        }
    } else if action == "dap" {
        if let Err(err) = dap::run_dap_session() {
            eprintln!("dap error: {}", err);
            process::exit(1);
        }
    } else if action == "pkg" {
        if let Err(err) = pkg::run_pkg_cli(&args[2..]) {
            eprintln!("pkg error: {}", err);
            process::exit(1);
        }
    } else {
        eprintln!("Unknown action: {}", action);
        process::exit(1);
    }
}
