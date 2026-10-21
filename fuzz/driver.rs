//! Time-boxed random fuzzing: `neyuki fuzz <target> --seconds N`.
//!
//! Each target generates inputs from a seeded generator and runs them until
//! its time budget is spent. A case fails when anything panics, even a panic
//! the VM or parser catches and turns into an error (a panic hook counts
//! them), or when one case runs longer than `CASE_TIMEOUT`. The failing input
//! is saved under `fuzz-artifacts/` and can be run again with `--replay`.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::corpus::Corpus;

/// A case running longer than this is reported as a hang.
const CASE_TIMEOUT: Duration = Duration::from_secs(20);

const ARTIFACT_DIR: &str = "fuzz-artifacts";

pub struct Target {
    pub name: &'static str,
    /// File extension for saved inputs.
    pub extension: &'static str,
    /// Seconds to run when `--seconds` is not given.
    pub default_seconds: u64,
    /// The fixed regression cases, run before the random ones.
    pub regressions: fn(),
    pub generate: fn(&Corpus, &mut StdRng) -> Vec<u8>,
    pub run: fn(&[u8]),
}

/// Every target, in the order `all` runs them: the bytecode loader first,
/// since it is the one that reads untrusted files.
pub fn targets() -> Vec<Target> {
    vec![
        super::file::random::TARGET,
        super::tree::random::TARGET,
        super::vm::random::TARGET,
        super::libs::random::TARGET,
        super::memory::random::TARGET,
    ]
}

const USAGE: &str =
    "Usage: neyuki fuzz [<target>[,<target>...]|all] [--seconds N] [--seed N] [--keep-going]
       neyuki fuzz <target> --replay <file>

Without a target, runs the fixed regression cases of every target. With
--keep-going, a crash is saved and fuzzing continues; the run still fails at
the end, after reporting each distinct crash once.
Targets: file (bytecode loader and verifier), tree (parser, checker, compiler,
formatter), vm (generated and mutated programs), libs (standard library
parsers), memory (buffers and the garbage collector).";

static PANICS: AtomicUsize = AtomicUsize::new(0);
/// True while a case runs; a panic anywhere else is the driver's own and is
/// printed as usual.
static IN_CASE: AtomicBool = AtomicBool::new(false);
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// Counts every panic, including ones caught further down, and keeps the
/// message of the latest.
fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        PANICS.fetch_add(1, Ordering::SeqCst);
        // Only the project's own frames: the rest is the panic machinery
        // and the fuzz driver.
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let frames: Vec<&str> = backtrace
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("at ./src/") && !l.contains("/fuzz/driver.rs"))
            .collect();
        let msg = format!("{}\n  {}", info, frames.join("\n  "));
        if !IN_CASE.load(Ordering::SeqCst) {
            eprintln!("{}", msg);
        }
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(msg);
        }
    }));
}

/// `neyuki fuzz ...`. Exits non-zero when a case fails.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    let mut names: Option<String> = None;
    let mut seconds: Option<u64> = None;
    let mut seed: Option<u64> = None;
    let mut replay: Option<String> = None;
    let mut keep_going = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |flag: &str| {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("{} needs a value\n\n{}", flag, USAGE))
        };
        match arg.as_str() {
            "--seconds" => {
                let v = value("--seconds")?;
                seconds = Some(
                    v.parse()
                        .map_err(|_| format!("invalid --seconds '{}'", v))?,
                );
            }
            "--seed" => {
                let v = value("--seed")?;
                seed = Some(v.parse().map_err(|_| format!("invalid --seed '{}'", v))?);
            }
            "--replay" => replay = Some(value("--replay")?),
            "--keep-going" => keep_going = true,
            "-h" | "--help" => {
                println!("{}", USAGE);
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option '{}'\n\n{}", other, USAGE));
            }
            other => names = Some(other.to_string()),
        }
    }

    let Some(names) = names else {
        super::run_all_fuzz_tests();
        return Ok(());
    };
    let all = targets();
    let selected: Vec<&Target> = if names == "all" {
        all.iter().collect()
    } else {
        names
            .split(',')
            .map(|name| {
                all.iter()
                    .find(|t| t.name == name)
                    .ok_or_else(|| format!("unknown fuzz target '{}'\n\n{}", name, USAGE))
            })
            .collect::<Result<_, _>>()?
    };

    install_panic_hook();
    if let Some(path) = replay {
        let [target] = selected[..] else {
            return Err("--replay takes exactly one target".to_string());
        };
        let input = std::fs::read(&path).map_err(|e| format!("failed to read {}: {}", path, e))?;
        return run_case(target, &input)
            .map_err(|msg| format!("{} crashed on {}:\n{}", target.name, path, msg));
    }

    let seed = seed.unwrap_or_else(|| rand::thread_rng().r#gen());
    println!(
        "fuzz seed {} (pass --seed {} to repeat this run)",
        seed, seed
    );
    let corpus = Corpus::load();
    let mut crashes = Vec::new();
    for target in selected {
        let seconds = seconds.unwrap_or(target.default_seconds);
        crashes.extend(fuzz_target(target, &corpus, seed, seconds, keep_going)?);
    }
    if crashes.is_empty() {
        return Ok(());
    }
    for crash in &crashes {
        eprintln!("\n{}", crash);
    }
    Err(format!("{} distinct crash(es) found", crashes.len()))
}

/// The panic's `file:line:column`, which tells crashes apart.
fn panic_location(msg: &str) -> &str {
    msg.lines()
        .next()
        .and_then(|line| line.strip_prefix("panicked at "))
        .unwrap_or(msg)
}

/// Runs one input, returning the panic it caused, if any.
fn run_case(target: &Target, input: &[u8]) -> Result<(), String> {
    let before = PANICS.load(Ordering::SeqCst);
    IN_CASE.store(true, Ordering::SeqCst);
    let result = panic::catch_unwind(AssertUnwindSafe(|| (target.run)(input)));
    IN_CASE.store(false, Ordering::SeqCst);
    if result.is_err() || PANICS.load(Ordering::SeqCst) != before {
        let msg = LAST_PANIC
            .lock()
            .ok()
            .and_then(|mut last| last.take())
            .unwrap_or_else(|| "panic".to_string());
        return Err(msg);
    }
    Ok(())
}

/// Fuzzes one target for `seconds`. Returns the crashes found, or stops at
/// the first one unless `keep_going`.
fn fuzz_target(
    target: &Target,
    corpus: &Corpus,
    seed: u64,
    seconds: u64,
    keep_going: bool,
) -> Result<Vec<String>, String> {
    println!("-> {}: regression cases", target.name);
    let before = PANICS.load(Ordering::SeqCst);
    if panic::catch_unwind(target.regressions).is_err() || PANICS.load(Ordering::SeqCst) != before {
        let msg = LAST_PANIC
            .lock()
            .ok()
            .and_then(|mut l| l.take())
            .unwrap_or_default();
        return Err(format!(
            "{}: a regression case failed:\n{}",
            target.name, msg
        ));
    }

    println!("-> {}: random cases for {}s", target.name, seconds);
    let current: Arc<Mutex<(u64, Vec<u8>)>> = Arc::new(Mutex::new((0, Vec::new())));
    let started = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let clock = Instant::now();
    let watchdog = spawn_watchdog(
        target,
        Arc::clone(&current),
        Arc::clone(&started),
        Arc::clone(&done),
        clock,
    );

    // Every target gets its own stream, so a target's cases don't depend on
    // which targets ran before it.
    let mut rng = StdRng::seed_from_u64(seed ^ fnv(target.name));
    let deadline = clock + Duration::from_secs(seconds);
    let mut cases = 0u64;
    let mut crashes: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // Each input is written here before it runs, so one that aborts the
    // process (a failed allocation, a stack overflow) is left behind.
    let in_flight = format!(
        "{}/{}-current.{}",
        ARTIFACT_DIR, target.name, target.extension
    );
    let _ = std::fs::create_dir_all(ARTIFACT_DIR);
    let outcome = loop {
        if Instant::now() >= deadline {
            break Ok(());
        }
        let input = (target.generate)(corpus, &mut rng);
        if let Ok(mut cur) = current.lock() {
            *cur = (cases, input.clone());
        }
        let _ = std::fs::write(&in_flight, &input);
        started.store(clock.elapsed().as_millis() as u64 + 1, Ordering::SeqCst);
        let result = run_case(target, &input);
        started.store(0, Ordering::SeqCst);
        cases += 1;
        if let Err(msg) = result {
            if !seen.insert(panic_location(&msg).to_string()) {
                continue;
            }
            let path = save_artifact(target, &input);
            let report = format!(
                "{} crashed on case {} (seed {}), input saved to {}\n\
                 replay with: neyuki fuzz {} --replay {}\n{}",
                target.name,
                cases - 1,
                seed,
                path,
                target.name,
                path,
                msg
            );
            if !keep_going {
                break Err(report);
            }
            eprintln!("   crash at {}, saved to {}", panic_location(&msg), path);
            crashes.push(report);
        }
    };
    done.store(true, Ordering::SeqCst);
    let _ = watchdog.join();
    let _ = std::fs::remove_file(&in_flight);
    let _ = std::fs::remove_dir(ARTIFACT_DIR);
    outcome?;
    println!(
        "   {} cases in {:.1}s, {} distinct crash(es)",
        cases,
        clock.elapsed().as_secs_f64(),
        crashes.len()
    );
    Ok(crashes)
}

/// Reports a case that hangs, then exits: a hung case can't be stopped
/// from outside its thread.
fn spawn_watchdog(
    target: &Target,
    current: Arc<Mutex<(u64, Vec<u8>)>>,
    started: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
    clock: Instant,
) -> std::thread::JoinHandle<()> {
    let name = target.name;
    let extension = target.extension;
    std::thread::spawn(move || {
        while !done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(250));
            let since = started.load(Ordering::SeqCst);
            if since == 0 {
                continue;
            }
            let running = clock.elapsed().as_millis() as u64 + 1 - since;
            if running > CASE_TIMEOUT.as_millis() as u64 && !done.load(Ordering::SeqCst) {
                let (case, input) = current.lock().map(|c| c.clone()).unwrap_or_default();
                let path = write_artifact(name, extension, &input);
                eprintln!(
                    "{} hung on case {} for over {}s, input saved to {}\n\
                     replay with: neyuki fuzz {} --replay {}",
                    name,
                    case,
                    CASE_TIMEOUT.as_secs(),
                    path,
                    name,
                    path
                );
                std::process::exit(1);
            }
        }
    })
}

fn save_artifact(target: &Target, input: &[u8]) -> String {
    write_artifact(target.name, target.extension, input)
}

fn write_artifact(name: &str, extension: &str, input: &[u8]) -> String {
    let path = format!(
        "{}/{}-{:016x}.{}",
        ARTIFACT_DIR,
        name,
        fnv_bytes(input),
        extension
    );
    if let Err(err) =
        std::fs::create_dir_all(ARTIFACT_DIR).and_then(|_| std::fs::write(&path, input))
    {
        eprintln!("failed to save {}: {}", path, err);
    }
    path
}

fn fnv(s: &str) -> u64 {
    fnv_bytes(s.as_bytes())
}

fn fnv_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Runs every target briefly, so `cargo test` keeps the generators and
/// harnesses working.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_runs_random_cases() {
        let corpus = Corpus::load();
        for target in targets() {
            let mut rng = StdRng::seed_from_u64(7);
            for _ in 0..40 {
                let input = (target.generate)(&corpus, &mut rng);
                let result = panic::catch_unwind(AssertUnwindSafe(|| (target.run)(&input)));
                assert!(result.is_ok(), "{} panicked", target.name);
            }
        }
    }
}
