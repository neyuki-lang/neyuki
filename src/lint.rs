use std::fs;
use std::panic;

use crate::lexer::Lexer;
use crate::parser::Parser;

pub fn lint(path: &str) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;

    let mut lexer = Lexer::new(&source);
    let tokens = lexer.tokenize();
    for token in &tokens {
        println!("{} @{}: {}", token.kind, token.line, token.value);
    }

    let mut parser = Parser::new(&source);
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| parser.parse_program()));

    let program = result.map_err(|_| format!("parse failed for {}", path))?;
    println!("parsed statements: {}", program.len());
    Ok(())
}
