use std::env;
use std::process;

mod lexer;
mod compiler;
mod lint;
mod parser;
mod runtime;
mod tests;

fn main() {
  let args: Vec<String> = env::args().collect();
  if args.len() < 2 {
    eprintln!("Usage: {} <action> [path]", args[0]);
    process::exit(1);
  }

  let action = &args[1];

  if action == "lint" {
    let path = args.get(2).map(String::as_str).unwrap_or("examples/hello.nyk");
    if let Err(err) = lint::lint(path) {
      eprintln!("{}", err);
      process::exit(1);
    }
  } else if action == "compile" {
    let Some(path) = args.get(2) else {
      eprintln!("Usage: {} compile <path>", args[0]);
      process::exit(1);
    };
    match compiler::compile_file(path) {
      Ok(program) => println!("compiled {} statement(s)", program.len()),
      Err(err) => { eprintln!("{}: {}", path, err); process::exit(1); }
    }
  } else if action == "run" {
    let Some(path) = args.get(2) else {
      eprintln!("Usage: {} run <path>", args[0]);
      process::exit(1);
    };
    if let Err(err) = runtime::run_file(path) {
      eprintln!("runtime error: {}", err);
      process::exit(1);
    }
  } else if action == "test" {
    tests::run_all_tests();
  } else {
    eprintln!("Unknown action: {}", action);
    process::exit(1);
  }
} 