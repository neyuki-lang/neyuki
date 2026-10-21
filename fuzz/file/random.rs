//! Random bytecode files: mutated compiler output and raw bytes, loaded and,
//! when the verifier accepts them, run in the fuzz sandbox. The verifier is
//! what makes untrusted `.nykb` files safe to run, so anything it lets
//! through must not panic the VM either.

use rand::Rng;
use rand::rngs::StdRng;

use super::super::corpus::{Corpus, mutate_bytes, run_sandboxed};
use super::super::driver::Target;
use crate::bytecode::{MAGIC, deserialize};

pub const TARGET: Target = Target {
    name: "file",
    extension: "nykb",
    default_seconds: 90,
    regressions,
    generate,
    run,
};

fn regressions() {
    use super::bytecode_fuzz::*;
    fuzz_truncated_bytecode();
    fuzz_corrupted_magic();
    fuzz_corrupted_instructions();
    fuzz_out_of_bounds_jumps();
    fuzz_random_byte_streams();
}

fn generate(corpus: &Corpus, rng: &mut StdRng) -> Vec<u8> {
    if rng.gen_ratio(1, 20) {
        // Random bytes behind a valid header, so they get past the magic.
        let mut bytes = MAGIC.to_vec();
        let n = rng.gen_range(0..512);
        bytes.extend((0..n).map(|_| rng.r#gen::<u8>()));
        return bytes;
    }
    let mut bytes = corpus.bytecode(rng).to_vec();
    let other = corpus.bytecode(rng).to_vec();
    mutate_bytes(rng, &mut bytes, &other);
    bytes
}

fn run(input: &[u8]) {
    if let Ok(proto) = deserialize(input) {
        let _ = run_sandboxed(proto);
    }
}
