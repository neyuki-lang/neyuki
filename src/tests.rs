use std::fs;
use std::path::Path;
use std::process;

use crate::lint::lint;

pub fn run_all_tests() {
    let tests_dir = Path::new("tests");
    let mut files: Vec<_> = fs::read_dir(tests_dir)
        .expect("failed to read tests directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "nyk"))
        .collect();

    files.sort();

    if files.is_empty() {
        println!("No .nyk test files found in tests/");
        return;
    }

    let mut failures = 0usize;
    println!("Running {} test file(s) from tests/", files.len());

    for path in files {
        println!("\n=== {} ===", path.display());
        match lint(path.to_str().expect("path should be valid utf-8")) {
            Ok(()) => println!("PASS"),
            Err(err) => {
                println!("FAIL: {}", err);
                failures += 1;
            }
        }
    }

    if failures > 0 {
        eprintln!("{} test file(s) failed.", failures);
        process::exit(1);
    }
}
