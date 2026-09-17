// Commercial-grade integration test runner for Neyuki `.nyk` test scripts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::Instant;

use crate::runtime;

#[derive(Debug)]
#[allow(dead_code)]
pub struct TestResult {
    pub path: PathBuf,
    pub duration_ms: u128,
    pub success: bool,
    pub error: Option<String>,
}

pub fn run_test_file(path: &Path) -> TestResult {
    let start = Instant::now();
    let path_str = path.to_str().expect("path should be valid UTF-8");
    let res = runtime::run_file(path_str);
    let duration_ms = start.elapsed().as_millis();

    match res {
        Ok(()) => TestResult {
            path: path.to_path_buf(),
            duration_ms,
            success: true,
            error: None,
        },
        Err(err) => TestResult {
            path: path.to_path_buf(),
            duration_ms,
            success: false,
            error: Some(err),
        },
    }
}

pub fn collect_test_files(tests_dir: &Path, filter: Option<&str>) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(tests_dir)
        .expect("failed to read tests directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "nyk"))
        .filter(|path| {
            if let Some(pattern) = filter {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|s| s.contains(pattern))
            } else {
                true
            }
        })
        .collect();

    files.sort();
    files
}

pub fn run_all_tests() {
    let tests_dir = Path::new("tests");
    let files = collect_test_files(tests_dir, None);

    if files.is_empty() {
        println!("No .nyk test files found in tests/");
        return;
    }

    println!(
        "Running {} integration test file(s) from tests/:",
        files.len()
    );
    let suite_start = Instant::now();
    let mut passed = 0;
    let mut failed = 0;

    for path in &files {
        let result = run_test_file(path);
        let name = path.display();
        if result.success {
            passed += 1;
            println!("  test {:<40} ... ok ({}ms)", name, result.duration_ms);
        } else {
            failed += 1;
            println!("  test {:<40} ... FAILED", name);
            if let Some(err) = &result.error {
                eprintln!("    --> error: {}", err);
            }
        }
    }

    let elapsed = suite_start.elapsed().as_secs_f64();
    println!(
        "\nTest result: {}. {} passed; {} failed; finished in {:.2}s\n",
        if failed == 0 { "ok" } else { "FAILED" },
        passed,
        failed,
        elapsed
    );

    if failed > 0 {
        process::exit(1);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_all_nyk_integration_files() {
        let tests_dir = Path::new("tests");
        let files = collect_test_files(tests_dir, None);
        assert!(!files.is_empty(), "expected tests/*.nyk files to exist");

        for path in files {
            let result = run_test_file(&path);
            assert!(
                result.success,
                "Test script '{}' failed: {:?}",
                path.display(),
                result.error
            );
        }
    }

    #[test]
    fn test_cargo_fmt_check() {
        let output = std::process::Command::new("cargo")
            .args(["fmt", "--all", "--", "--check"])
            .output();
        if let Ok(out) = output {
            assert!(
                out.status.success(),
                "cargo fmt --all -- --check failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
