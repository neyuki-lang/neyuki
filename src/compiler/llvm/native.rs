//! The command lines of `neyuki emit-llvm` and `neyuki build --native`, and
//! the native build: emit IR for the external runtime, compile it to an
//! object file with clang and link that with `libnyk_rt.a` (see
//! `crates/nyk_rt`) through clang's system linker.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::compiler::llvm::{LlvmOptions, RuntimeMode, TargetSpec, target::clang_path};
use crate::compiler::{self, OptLevel};

pub const EMIT_USAGE: &str = "\
Usage: neyuki emit-llvm [-O0|-O1] [--runtime=builtin|extern] [--target <triple>] <file> [out.ll]

  --runtime=builtin  define stub runtime functions in the module, so lli and
                     clang can run it on its own (default)
  --runtime=extern   only declare them, for linking with libnyk_rt
  --target <triple>  the target triple (default: the host's, from clang)";

pub const BUILD_USAGE: &str = "\
Usage: neyuki build --native [-O0|-O1] [--target <triple>] [--rt-lib <path>] <file> [-o <out>]

Compiles a script to a native executable through LLVM: clang compiles the
IR and links it with the runtime library libnyk_rt.a. That library is
found with --rt-lib, else $NEYUKI_RT_LIB, else next to the neyuki
executable; build it with
  cargo build --release --manifest-path crates/nyk_rt/Cargo.toml
clang is $NEYUKI_CLANG, else clang on PATH.";

/// Parsed `emit-llvm` / `build --native` arguments.
#[derive(Debug, Default)]
struct Args {
    opt_level: OptLevel,
    runtime: RuntimeMode,
    target: Option<String>,
    rt_lib: Option<PathBuf>,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
}

/// Reads `--name value` or `--name=value` at `args[*idx]`, advancing past it.
fn flag_value(args: &[String], idx: &mut usize, name: &str) -> Result<Option<String>, String> {
    let arg = &args[*idx];
    if let Some(value) = arg.strip_prefix(name).and_then(|v| v.strip_prefix('=')) {
        return Ok(Some(value.to_string()));
    }
    if arg != name {
        return Ok(None);
    }
    *idx += 1;
    args.get(*idx)
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("{} expects a value", name))
}

fn parse_args(args: &[String], native: bool) -> Result<Args, String> {
    let usage = if native { BUILD_USAGE } else { EMIT_USAGE };
    let mut parsed = Args::default();
    let mut idx = 0;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if arg == "-O0" {
            parsed.opt_level = OptLevel::O0;
        } else if arg == "-O1" {
            parsed.opt_level = OptLevel::O1;
        } else if native && arg == "--native" {
            // The caller dispatched on it already.
        } else if let Some(target) = flag_value(args, &mut idx, "--target")? {
            parsed.target = Some(target);
        } else if !native && let Some(runtime) = flag_value(args, &mut idx, "--runtime")? {
            parsed.runtime = match runtime.as_str() {
                "builtin" => RuntimeMode::Builtin,
                "extern" => RuntimeMode::Extern,
                other => {
                    return Err(format!(
                        "unknown runtime '{}' (expected builtin or extern)\n\n{}",
                        other, usage
                    ));
                }
            };
        } else if native && let Some(path) = flag_value(args, &mut idx, "--rt-lib")? {
            parsed.rt_lib = Some(path.into());
        } else if native && (arg == "-o" || arg == "--out") {
            idx += 1;
            let out = args
                .get(idx)
                .ok_or_else(|| format!("{} expects a value\n\n{}", arg, usage))?;
            parsed.output = Some(out.into());
        } else if arg == "-h" || arg == "--help" {
            return Err(usage.to_string());
        } else if arg.starts_with('-') {
            return Err(format!("unknown option '{}'\n\n{}", arg, usage));
        } else if parsed.input.is_none() {
            parsed.input = Some(arg.into());
        } else if !native && parsed.output.is_none() {
            parsed.output = Some(arg.into());
        } else {
            return Err(format!("unexpected argument '{}'\n\n{}", arg, usage));
        }
        idx += 1;
    }
    if parsed.input.is_none() {
        return Err(usage.to_string());
    }
    if native {
        parsed.runtime = RuntimeMode::Extern;
    }
    Ok(parsed)
}

/// Reads and compiles the input of `args` to textual IR.
fn emit(args: &Args) -> Result<(String, TargetSpec), String> {
    let input = args.input.as_deref().unwrap_or(Path::new(""));
    let source = std::fs::read_to_string(input)
        .map_err(|err| format!("failed to read {}: {}", input.display(), err))?;
    let target = TargetSpec::resolve(args.target.as_deref());
    let llvm_options = LlvmOptions {
        runtime: args.runtime,
        target: Some(target.clone()),
    };
    let chunk = input.to_string_lossy();
    let ll = compiler::compile_source_to_llvm_ir(
        &source,
        &chunk,
        compiler::Options::RUN.with_opt_level(args.opt_level),
        &llvm_options,
    )?;
    Ok((ll, target))
}

/// `neyuki emit-llvm`. Errors are returned as the text to print.
pub fn emit_cli(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, false)?;
    let (ll, _) = emit(&args)?;
    let input = args.input.as_deref().unwrap_or(Path::new("")).display();
    match &args.output {
        Some(out) => {
            std::fs::write(out, &ll)
                .map_err(|err| format!("failed to write LLVM IR to {}: {}", out.display(), err))?;
            println!(
                "emitted {} -> {} ({} bytes)",
                input,
                out.display(),
                ll.len()
            );
        }
        None => print!("{}", ll),
    }
    Ok(())
}

/// The system libraries the Rust standard library inside `libnyk_rt.a`
/// needs, as `cargo rustc --release --manifest-path crates/nyk_rt/Cargo.toml
/// -- --print native-static-libs` reports them for `triple`, or
/// `$NEYUKI_RT_NATIVE_LIBS` (separated by spaces) when set.
fn native_libs(triple: &str) -> Vec<String> {
    if let Ok(libs) = std::env::var("NEYUKI_RT_NATIVE_LIBS") {
        return libs.split_whitespace().map(str::to_string).collect();
    }
    let libs: &[&str] = if triple.contains("apple") {
        &["-lSystem", "-lc", "-lm"]
    } else if triple.contains("windows") && triple.contains("gnu") {
        &[
            "-lkernel32",
            "-ladvapi32",
            "-lntdll",
            "-luserenv",
            "-lws2_32",
            "-ldbghelp",
            "-lmingwex",
            "-lmsvcrt",
        ]
    } else if triple.contains("windows") {
        &[
            "-lkernel32",
            "-ladvapi32",
            "-lntdll",
            "-luserenv",
            "-lws2_32",
            "-ldbghelp",
        ]
    } else {
        &[
            "-lgcc_s",
            "-lutil",
            "-lrt",
            "-lpthread",
            "-lm",
            "-ldl",
            "-lc",
        ]
    };
    libs.iter().map(|lib| lib.to_string()).collect()
}

/// Finds `libnyk_rt.a` (`nyk_rt.lib` for msvc): `--rt-lib`, else
/// `$NEYUKI_RT_LIB`, else next to the running `neyuki`.
fn find_rt_lib(explicit: Option<&Path>, triple: &str) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = std::env::var_os("NEYUKI_RT_LIB") {
        return Ok(path.into());
    }
    let name = if triple.contains("windows-msvc") {
        "nyk_rt.lib"
    } else {
        "libnyk_rt.a"
    };
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)));
    match beside_exe {
        Some(path) if path.is_file() => Ok(path),
        _ => Err(format!(
            "cannot find the runtime library {} (pass --rt-lib or set NEYUKI_RT_LIB)\n\n{}",
            name, BUILD_USAGE
        )),
    }
}

/// Runs `cmd`, turning a failure into an error naming `what`.
fn run_tool(mut cmd: Command, what: &str) -> Result<(), String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .output()
        .map_err(|err| format!("failed to run {} ({}): {}", program, what, err))?;
    if !out.status.success() {
        return Err(format!(
            "{} failed ({}):\n{}",
            program,
            what,
            String::from_utf8_lossy(&out.stderr).trim_end()
        ));
    }
    Ok(())
}

/// `neyuki build --native`: compiles the script to an executable and
/// returns its path.
pub fn build_native_cli(args: &[String]) -> Result<PathBuf, String> {
    let args = parse_args(args, true)?;
    let input = args.input.clone().unwrap_or_default();
    let (ll, target) = emit(&args)?;
    let rt_lib = find_rt_lib(args.rt_lib.as_deref(), &target.triple)?;

    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".to_string());
    let output = args.output.clone().unwrap_or_else(|| {
        let exe = if target.triple.contains("windows") {
            format!("{}.exe", stem)
        } else {
            stem.clone()
        };
        PathBuf::from(exe)
    });

    let work = std::env::temp_dir().join(format!(
        "neyuki-native-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&work)
        .map_err(|err| format!("failed to create {}: {}", work.display(), err))?;
    let result = (|| {
        let ll_path = work.join(format!("{}.ll", stem));
        let obj_path = work.join(format!("{}.o", stem));
        std::fs::write(&ll_path, &ll)
            .map_err(|err| format!("failed to write {}: {}", ll_path.display(), err))?;

        // The same `-target` as the module's triple, so clang has nothing to
        // override.
        let clang_opt = match args.opt_level {
            OptLevel::O0 => "-O0",
            OptLevel::O1 => "-O2",
        };
        let mut compile = Command::new(clang_path());
        compile
            .arg("-target")
            .arg(&target.triple)
            .arg(clang_opt)
            .arg("-c")
            .arg(&ll_path)
            .arg("-o")
            .arg(&obj_path);
        run_tool(compile, "compiling the LLVM IR")?;

        let mut link = Command::new(clang_path());
        link.arg("-target")
            .arg(&target.triple)
            .arg(&obj_path)
            .arg(&rt_lib)
            .args(native_libs(&target.triple))
            .arg("-o")
            .arg(&output);
        run_tool(link, "linking with the runtime")
    })();
    let _ = std::fs::remove_dir_all(&work);
    result.map(|()| output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn test_parse_emit_args() {
        let args = parse_args(
            &strings(&[
                "-O0",
                "--runtime=extern",
                "--target",
                "x86_64-pc-linux-gnu",
                "a.nyk",
                "a.ll",
            ]),
            false,
        )
        .expect("valid arguments");
        assert_eq!(args.opt_level, OptLevel::O0);
        assert_eq!(args.runtime, RuntimeMode::Extern);
        assert_eq!(args.target.as_deref(), Some("x86_64-pc-linux-gnu"));
        assert_eq!(args.input.as_deref(), Some(Path::new("a.nyk")));
        assert_eq!(args.output.as_deref(), Some(Path::new("a.ll")));

        let args = parse_args(&strings(&["a.nyk"]), false).expect("valid arguments");
        assert_eq!(args.runtime, RuntimeMode::Builtin);
        assert_eq!(args.target, None);

        assert!(parse_args(&strings(&["--runtime=wasm", "a.nyk"]), false).is_err());
        assert!(parse_args(&strings(&["--rt-lib", "x", "a.nyk"]), false).is_err());
        assert!(parse_args(&strings(&[]), false).is_err());
    }

    #[test]
    fn test_parse_build_native_args() {
        let args = parse_args(
            &strings(&[
                "--native",
                "app.nyk",
                "-o",
                "out/app",
                "--rt-lib=lib.a",
                "--target=aarch64-apple-darwin",
            ]),
            true,
        )
        .expect("valid arguments");
        assert_eq!(args.runtime, RuntimeMode::Extern);
        assert_eq!(args.input.as_deref(), Some(Path::new("app.nyk")));
        assert_eq!(args.output.as_deref(), Some(Path::new("out/app")));
        assert_eq!(args.rt_lib.as_deref(), Some(Path::new("lib.a")));
        assert_eq!(args.target.as_deref(), Some("aarch64-apple-darwin"));
        assert!(parse_args(&strings(&["--native", "a.nyk", "b.nyk"]), true).is_err());
        assert!(parse_args(&strings(&["--native", "--runtime=builtin", "a.nyk"]), true).is_err());
    }
}
