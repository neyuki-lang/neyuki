//! Mutated source text through the front end: parser, checker, compiler
//! and formatter. Whatever the compiler emits must also pass the bytecode
//! verifier, or `neyuki compile` would write files `neyuki run` rejects.

use rand::Rng;
use rand::rngs::StdRng;

use super::super::corpus::{Corpus, mutate_text};
use super::super::driver::Target;
use crate::compiler::{Options, compile_program};

pub const TARGET: Target = Target {
    name: "tree",
    extension: "nyk",
    default_seconds: 60,
    regressions,
    generate,
    run,
};

fn regressions() {
    use super::ast_fuzz::*;
    fuzz_nested_parentheses();
    fuzz_operator_chains();
    fuzz_malformed_interpolations();
    fuzz_unbalanced_delimiters();
}

fn generate(corpus: &Corpus, rng: &mut StdRng) -> Vec<u8> {
    let source = corpus.source(rng).to_string();
    let other = corpus.source(rng).to_string();
    let mut text = mutate_text(rng, &source, &other);
    if rng.gen_ratio(1, 4) {
        text = mutate_text(rng, &text, &other);
    }
    text.into_bytes()
}

fn run(input: &[u8]) {
    let source = String::from_utf8_lossy(input);
    let _ = crate::lint::lint_source(&source);
    let _ = crate::fmt::format_source(&source);
    // With and without inlining; the checker already ran above.
    let debug = Options {
        check: false,
        ..Options::DEBUG
    };
    for options in [Options::UNCHECKED, debug] {
        if let Ok(proto) = compile_program(&source, "fuzz", options) {
            let bytes = crate::bytecode::serialize(&proto);
            if let Err(err) = crate::bytecode::deserialize(&bytes) {
                panic!("compiled bytecode fails to load: {}", err);
            }
        }
    }
}
