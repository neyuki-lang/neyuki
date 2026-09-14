use std::env;
use std::process;

mod lexer;
mod lint;
mod parser;
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
    lint::lint(path);
  } else if action == "test" {
    tests::run_all_tests();
  } else {
    eprintln!("Unknown action: {}", action);
    process::exit(1);
  }
} 