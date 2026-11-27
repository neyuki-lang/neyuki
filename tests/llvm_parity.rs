//! Runs small programs on the register VM and through the LLVM backend, and
//! checks that both print the same thing. Every case runs twice under LLVM:
//!
//! - built-in runtime: `neyuki emit-llvm`, checked with `opt -passes=verify`
//!   and run with `lli`;
//! - external runtime: `neyuki emit-llvm --runtime=extern` (verified too)
//!   and `neyuki build --native`, which links with `crates/nyk_rt`, built
//!   here once with cargo.
//!
//! The LLVM tools are found in `NEYUKI_LLVM_BIN` if set, else on `PATH`, and
//! clang as `neyuki build --native` finds it (`NEYUKI_CLANG` or `clang`).
//! Without them the tests skip, unless `NEYUKI_REQUIRE_LLVM=1` (as in CI)
//! makes a missing tool a failure.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What a program should do under the LLVM backend.
enum Expect {
    /// Print what the VM prints and succeed or fail as it does.
    SameAsVm,
    /// Print exactly this and succeed. For the cases where the VM widens an
    /// integer to a bignum and the backend's 64-bit integers wrap instead.
    Output(&'static str),
    /// Fail like the VM does after printing the same output, with this in
    /// the error message of both.
    Error(&'static str),
}

/// Which LLVM runtime a case runs with.
#[derive(Clone, Copy, Debug)]
enum Mode {
    Builtin,
    Extern,
}

struct Run {
    stdout: String,
    stderr: String,
    success: bool,
}

impl From<Output> for Run {
    fn from(out: Output) -> Self {
        Run {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            success: out.status.success(),
        }
    }
}

fn require_llvm() -> bool {
    std::env::var_os("NEYUKI_REQUIRE_LLVM").is_some_and(|v| v == "1")
}

/// `path` when `path --version` runs, else `None` (after saying so) unless
/// the tools are required.
fn working_tool(path: PathBuf) -> Option<PathBuf> {
    let works = Command::new(&path)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if works {
        return Some(path);
    }
    if require_llvm() {
        panic!(
            "NEYUKI_REQUIRE_LLVM=1 but `{}` does not run",
            path.display()
        );
    }
    eprintln!("skipping: `{}` not found", path.display());
    None
}

/// The LLVM tool `name`.
fn llvm_tool(name: &str) -> Option<PathBuf> {
    working_tool(match std::env::var_os("NEYUKI_LLVM_BIN") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
    })
}

fn clang() -> Option<PathBuf> {
    working_tool(
        std::env::var_os("NEYUKI_CLANG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("clang")),
    )
}

/// `libnyk_rt.a`, built once per test run into Cargo's temporary directory
/// for integration tests.
fn nyk_rt() -> &'static Path {
    static LIB: OnceLock<PathBuf> = OnceLock::new();
    LIB.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nyk_rt");
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let out = Command::new(cargo)
            .args(["build", "--release", "--quiet", "--manifest-path"])
            .arg(root.join("crates/nyk_rt/Cargo.toml"))
            .arg("--target-dir")
            .arg(&target_dir)
            .output()
            .expect("run cargo build for nyk_rt");
        assert!(
            out.status.success(),
            "building nyk_rt failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let name = if cfg!(target_env = "msvc") {
            "nyk_rt.lib"
        } else {
            "libnyk_rt.a"
        };
        target_dir.join("release").join(name)
    })
}

/// A fresh scratch directory for one test case.
fn scratch_dir(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "neyuki-llvm-parity-{}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        name
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn neyuki() -> Command {
    Command::new(env!("CARGO_BIN_EXE_neyuki"))
}

/// Emits `script` as LLVM IR for `mode`, checks it with `opt -passes=verify`
/// and returns the `.ll` path.
fn emit_verified(dir: &Path, script: &Path, opt: &Path, mode: Mode) -> PathBuf {
    let ll = dir.join(format!("{:?}.ll", mode));
    let mut cmd = neyuki();
    cmd.arg("emit-llvm");
    if let Mode::Extern = mode {
        cmd.arg("--runtime=extern");
    }
    let emit = cmd
        .arg(script)
        .arg(&ll)
        .output()
        .expect("run neyuki emit-llvm");
    assert!(
        emit.status.success(),
        "emit-llvm failed: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let verify = Command::new(opt)
        .args(["-passes=verify", "-disable-output"])
        .arg(&ll)
        .output()
        .expect("run opt");
    assert!(
        verify.status.success(),
        "emitted IR does not verify:\n{}\n--- IR ---\n{}",
        String::from_utf8_lossy(&verify.stderr),
        std::fs::read_to_string(&ll).unwrap_or_default()
    );
    ll
}

/// Runs `script` under LLVM in `mode`: the IR with `lli`, or the executable
/// that `neyuki build --native` makes.
fn run_llvm(dir: &Path, script: &Path, opt: &Path, mode: Mode) -> Option<Run> {
    let ll = emit_verified(dir, script, opt, mode);
    match mode {
        Mode::Builtin => {
            let lli = llvm_tool("lli")?;
            Some(Command::new(lli).arg(&ll).output().expect("run lli").into())
        }
        Mode::Extern => {
            let clang = clang()?;
            let exe = dir.join(if cfg!(windows) { "app.exe" } else { "app" });
            let build = neyuki()
                .args(["build", "--native"])
                .arg(script)
                .arg("-o")
                .arg(&exe)
                .env("NEYUKI_CLANG", clang)
                .env("NEYUKI_RT_LIB", nyk_rt())
                .output()
                .expect("run neyuki build --native");
            assert!(
                build.status.success(),
                "build --native failed: {}",
                String::from_utf8_lossy(&build.stderr)
            );
            Some(Command::new(&exe).output().expect("run executable").into())
        }
    }
}

/// Checks `src` against `expect` with both runtimes.
fn check_case(name: &str, src: &str, expect: Expect) {
    check_modes(name, src, &expect, &[Mode::Builtin, Mode::Extern]);
}

/// Checks `src` against `expect` with the external runtime only, for what
/// the built-in stubs cannot do (such as calling a builtin of the runtime).
fn check_extern_case(name: &str, src: &str, expect: Expect) {
    check_modes(name, src, &expect, &[Mode::Extern]);
}

fn check_modes(name: &str, src: &str, expect: &Expect, modes: &[Mode]) {
    check_files(name, &[(&format!("{}.nyk", name), src)], expect, modes);
}

/// Checks a project of several files, the first of which is the entry
/// script, which the VM runs and `neyuki build --native` is given the
/// directory of (so `main.nyk` when there is no `neyuki.toml` among them).
/// A single file is built as a script.
fn check_files(name: &str, files: &[(&str, &str)], expect: &Expect, modes: &[Mode]) {
    let Some(opt) = llvm_tool("opt") else {
        return;
    };
    let dir = scratch_dir(name);
    let mut src = String::new();
    for (file, text) in files {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        std::fs::write(&path, text).expect("write file");
        src.push_str(&format!("-- {}\n{}\n", file, text));
    }
    let entry = files
        .iter()
        .map(|(file, _)| *file)
        .find(|file| *file != "neyuki.toml")
        .expect("an entry script");
    let script = dir.join(entry);
    let input = if files.len() == 1 {
        script.clone()
    } else {
        dir.clone()
    };
    let vm: Run = neyuki().arg(&script).output().expect("run vm").into();

    for &mode in modes {
        let Some(native) = run_llvm(&dir, &input, &opt, mode) else {
            continue;
        };
        let context = format!(
            "case `{}` ({:?} runtime):\n{}\n--- vm stdout ---\n{}--- vm stderr ---\n{}\
             --- llvm stdout ---\n{}--- llvm stderr ---\n{}",
            name, mode, src, vm.stdout, vm.stderr, native.stdout, native.stderr
        );
        match expect {
            Expect::SameAsVm => {
                assert_eq!(
                    vm.success, native.success,
                    "exit status differs, {}",
                    context
                );
                assert_eq!(vm.stdout, native.stdout, "output differs, {}", context);
            }
            Expect::Output(out) => {
                assert!(native.success, "llvm run failed, {}", context);
                assert_eq!(native.stdout, *out, "unexpected output, {}", context);
            }
            Expect::Error(msg) => {
                assert!(
                    !vm.success && !native.success,
                    "expected failure, {}",
                    context
                );
                assert_eq!(vm.stdout, native.stdout, "output differs, {}", context);
                assert!(
                    vm.stderr.contains(msg),
                    "vm error lacks `{}`, {}",
                    msg,
                    context
                );
                assert!(
                    native.stderr.contains(&format!("runtime error: {}", msg)),
                    "llvm error lacks `{}`, {}",
                    msg,
                    context
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// Operands come in through function parameters where the MIR folder would
// otherwise compute the result at compile time, so the runtime helpers run.

#[test]
fn floor_division_and_modulo() {
    check_case(
        "floor_div_mod",
        "\
local function idiv(a, b) return a // b end
local function mod(a, b) return a % b end
local a = idiv(-5, 3)
local b = mod(-5, 3)
local c = mod(5, -3)
local d = idiv(5, -3)
local e = idiv(7, 2)
local f = mod(-6, 3)
print(a, b, c, d, e, f)
a = idiv(5.5, 2)
b = mod(5.5, 2)
c = idiv(-5.5, 2)
d = mod(-5.5, 2)
e = mod(5.5, -2)
print(a, b, c, d, e)
print(-5 // 3, -5 % 3, 5 % -3, 5.5 // 2, 5.5 % 2)
",
        Expect::SameAsVm,
    );
}

#[test]
fn integer_division_by_zero_is_an_error() {
    check_case(
        "idiv_zero",
        "local function idiv(a, b) return a // b end\nprint(\"before\")\nlocal q = idiv(1, 0)\nprint(q)\n",
        Expect::Error("division by zero"),
    );
    check_case(
        "mod_zero",
        "local function mod(a, b) return a % b end\nlocal r = mod(1, 0)\nprint(r)\n",
        Expect::Error("modulo by zero"),
    );
    // The VM rejects a zero divisor for `//` and `%` whatever the types.
    check_case(
        "float_idiv_zero",
        "local function idiv(a, b) return a // b end\nlocal q = idiv(1.5, 0)\nprint(q)\n",
        Expect::Error("division by zero"),
    );
}

#[test]
fn float_division_by_zero_follows_ieee() {
    check_case(
        "float_div_zero",
        "local function div(a, b) return a / b end\nlocal p = div(1, 0)\nlocal n = div(-1, 0)\nprint(p, n)\n",
        Expect::SameAsVm,
    );
}

#[test]
fn int64_min_divided_by_minus_one_wraps() {
    // The VM widens to the bignum 9223372036854775808; 64-bit integers wrap.
    check_case(
        "int64_min_div",
        "\
local function idiv(a, b) return a // b end
local function mod(a, b) return a % b end
local m = -9223372036854775807 - 1
local q = idiv(m, -1)
local r = mod(m, -1)
print(q, r)
print(m // -1, m % -1)
",
        Expect::Output("-9223372036854775808\t0\n-9223372036854775808\t0\n"),
    );
}

#[test]
fn shifts_by_large_and_negative_counts() {
    check_case(
        "shifts_vm",
        "\
local function shl(a, b) return a << b end
local function shr(a, b) return a >> b end
local function lshl(a, b) return a <<< b end
local function lshr(a, b) return a >>> b end
local a = shr(1, 64)
local b = shr(-1, 64)
local c = shr(-8, 100)
local d = lshr(1, 64)
local e = lshr(-1, 64)
local f = lshl(1, 64)
print(a, b, c, d, e, f)
a = shl(1, -1)
b = shr(8, -1)
c = shr(-8, -1)
d = shl(-8, -1)
e = lshl(1, -1)
f = lshr(8, -1)
print(a, b, c, d, e, f)
a = shl(3, 4)
b = shr(-16, 2)
c = lshr(-8, 1)
d = shl(1.9, 2)
print(a, b, c, d)
",
        Expect::SameAsVm,
    );
    // The VM widens `1 << 64` to a bignum; a 64-bit shift gives 0.
    check_case(
        "shl_64",
        "local function shl(a, b) return a << b end\nlocal a = shl(1, 64)\nlocal b = shl(1, 100)\nprint(a, b)\n",
        Expect::Output("0\t0\n"),
    );
}

#[test]
fn bitwise_operands_must_be_numbers() {
    check_case(
        "bitwise_floats",
        "\
local function band(a, b) return a & b end
local function bor(a, b) return a | b end
local function bnot(a) return ~a end
local a = band(1.5, 1)
local b = band(2.0, 1)
local c = bor(3, 4.0)
local d = band(-1.5, 7)
local e = bnot(1.5)
print(a, b, c, d, e)
",
        Expect::SameAsVm,
    );
    check_case(
        "bitwise_string",
        "local function band(a, b) return a & b end\nlocal r = band(\"a\", 1)\nprint(r)\n",
        Expect::Error("expected integer"),
    );
    check_case(
        "shift_string",
        "local function shl(a, b) return a << b end\nlocal r = shl(1, \"x\")\nprint(r)\n",
        Expect::Error("expected integer"),
    );
}

#[test]
fn arithmetic_type_errors_and_power() {
    check_case(
        "pow",
        "\
local function pow(a, b) return a ^ b end
local a = pow(2, 10)
local b = pow(2, -1)
local c = pow(3, 0)
local d = pow(-2, 3)
print(a, b, c, d)
",
        Expect::SameAsVm,
    );
    check_case(
        "add_string",
        "local function add(a, b) return a + b end\nlocal r = add(\"10\", 1)\nprint(r)\n",
        Expect::Error("expected number"),
    );
    check_case(
        "neg_string",
        "local function neg(a) return -a end\nlocal r = neg(\"a\")\nprint(r)\n",
        Expect::Error("unary minus expects a number"),
    );
}

/// Every `.nyk` file under `dir`, recursively.
fn nyk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            nyk_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "nyk") {
            out.push(path);
        }
    }
}

#[test]
fn every_test_and_example_emits_valid_ir() {
    let Some(opt) = llvm_tool("opt") else {
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    nyk_files(&root.join("tests"), &mut files);
    nyk_files(&root.join("examples"), &mut files);
    // The fixtures are modules for other tests to require, not scripts: they
    // name each other relative to the script that requires them.
    files.retain(|file| !file.components().any(|c| c.as_os_str() == "fixtures"));
    files.sort();
    assert!(!files.is_empty(), "no .nyk files found");
    let dir = scratch_dir("all");
    for script in &files {
        emit_verified(&dir, script, &opt, Mode::Builtin);
        emit_verified(&dir, script, &opt, Mode::Extern);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn numbers_compare_by_value() {
    check_case(
        "number_eq",
        "\
local function div(a, b) return a / b end
local nan = div(0, 0)
local one = 1
local onef = div(2, 2)
local big = 9007199254740993
local bigf = 9007199254740992.0
print(one == 1.0, 1 == 1.0, one == onef, one ~= onef, one == 1.5)
print(nan == nan, nan ~= nan, 0.0 == -0.0, big == bigf, big - 1 == bigf)
print(1 < 1.5, 2 <= 2.0, 2.5 > 2, one >= onef)
",
        Expect::SameAsVm,
    );
}

#[test]
fn strings_compare_by_content() {
    check_case(
        "string_eq",
        "\
local function id(x) return x end
local n = id(1)
local a = \"x\" .. n
local b = \"x\" .. \"1\"
local c = id(\"x1\")
print(a == b, a == c, a == \"x1\", a ~= b, a == \"x2\", a, b)
print(\"a\" < \"b\", \"b\" <= \"a\", \"ab\" < \"abc\", \"B\" < \"a\", \"\" < \"a\", a <= b, a < b)
print(\"1\" == 1, true == 1, nil == false)
",
        Expect::SameAsVm,
    );
}

#[test]
fn tables_and_functions_compare_by_identity() {
    check_case(
        "identity_eq",
        "\
local t = {}
local u = {}
local function f() end
local function g() end
local h = f
print(t == t, t == u, t ~= u, f == h, f == g)
",
        Expect::SameAsVm,
    );
}

#[test]
fn ordering_mixed_types_is_an_error() {
    check_case(
        "string_lt_number",
        "local function lt(a, b) return a < b end\nlocal r = lt(\"a\", 1)\nprint(r)\n",
        Expect::Error("expected number"),
    );
    check_case(
        "table_lt_table",
        "local function le(a, b) return a <= b end\nlocal r = le({}, {})\nprint(r)\n",
        Expect::Error("expected number"),
    );
}

#[test]
fn calls_pass_their_arguments() {
    check_case(
        "spread_calls",
        "\
local function add3(a, b, c) return a + b + c end
local function twice(f, x) return f(f(x, 1, 1), 1, 1) end
local function wrap(f, a, b, c) return f(a, b, c) end
print(add3(1, 2, 3))
print(\"sum\", wrap(add3, 10, 20, 30), twice(add3, 0))
local t = { add3(1, 1, 1) }
",
        Expect::SameAsVm,
    );
    check_case(
        "generic_for",
        "\
-- Called as upto3(state, ctrl), with ctrl the previous value (nil at first).
local function upto3(_, i)
  if i == nil then
    i = 0
  end
  if i < 3 then
    return i + 1
  end
  return nil
end
for i in upto3 do
  print(\"i\", i)
end
",
        Expect::SameAsVm,
    );
}

#[test]
fn dynamic_calls_reach_runtime_builtins() {
    // `select` is a builtin of nyk_rt, called through __nyk_call_dyn with
    // all of its arguments; the built-in stubs have no builtins.
    check_extern_case(
        "call_dyn_select",
        "\
local x = select(3, 10, 20, 30)
local y = select(-1, \"a\", \"b\")
local n = select(\"#\", 1, 2, 3)
print(x, y, n)
",
        Expect::SameAsVm,
    );
    check_extern_case(
        "call_nil",
        "local function call(f) return f(1, 2, 3) end\nlocal r = call(nil)\nprint(r)\n",
        Expect::Error("attempted to call non-function (nil)"),
    );
}

#[test]
fn length_of_strings() {
    check_case(
        "len",
        "local function id(x) return x end\nlocal s = id(\"abc\") .. id(\"de\")\nprint(#s, #\"\")\n",
        Expect::SameAsVm,
    );
    check_case(
        "len_number",
        "local function len(x) return #x end\nlocal n = len(5)\nprint(n)\n",
        Expect::Error("len expects string, table or buffer"),
    );
}

#[test]
fn emitted_module_targets_the_host() {
    let (Some(clang), Some(opt)) = (clang(), llvm_tool("opt")) else {
        return;
    };
    let dir = scratch_dir("target");
    let script = dir.join("t.nyk");
    std::fs::write(&script, "print(1)\n").expect("write script");
    let ll = emit_verified(&dir, &script, &opt, Mode::Builtin);
    let ir = std::fs::read_to_string(&ll).expect("read IR");
    assert!(ir.contains("\ntarget triple = \""), "{}", ir);
    assert!(ir.contains("\ntarget datalayout = \""), "{}", ir);
    // clang compiles it as it is, without overriding the module's triple.
    let out = Command::new(clang)
        .arg("-c")
        .arg(&ll)
        .arg("-o")
        .arg(dir.join("t.o"))
        .output()
        .expect("run clang");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{}", stderr);
    assert!(
        !stderr.contains("overriding the module target triple"),
        "{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recursive_closures() {
    check_case(
        "recursive_closure",
        "\
local base = 100
local function fact(n)
  if n <= 1 then
    return 1
  end
  return n * fact(n - 1)
end
local function sum(n)
  if n == 0 then
    return base
  end
  return n + sum(n - 1)
end
local f = fact(10)
local s = sum(3)
print(f, s)
",
        Expect::SameAsVm,
    );
}

#[test]
fn closures_capture_their_own_variables() {
    check_case(
        "closure_instances",
        "\
local function counter()
  local n = 0
  return function()
    n = n + 1
    return n
  end
end
local function adder(k)
  return function(x) return x + k end
end
local a = counter()
local b = counter()
local x = a()
local y = a()
local z = b()
local add1 = adder(1)
local add10 = adder(10)
local p = add1(5)
local q = add10(5)
print(x, y, z, p, q)
",
        Expect::SameAsVm,
    );
}

#[test]
fn closures_share_variables_with_their_parent() {
    check_case(
        "closure_sharing",
        "\
local v = 1
local function get() return v end
local function set(x) v = x end
v = 2
local a = get()
set(3)
local b = get()
print(a, b, v)
-- Each loop iteration has its own `i`.
local f1 = nil
local f2 = nil
for i = 1, 2 do
  local function cur() return i end
  if i == 1 then
    f1 = cur
  else
    f2 = cur
  end
end
local c = f1()
local d = f2()
print(c, d)
-- A variable two functions out reaches the inner one through the middle one.
local function outer()
  local hits = 0
  local function middle()
    local function inner()
      hits = hits + 1
      return hits
    end
    return inner
  end
  local g = middle()
  g()
  g()
  return hits
end
local h = outer()
print(h)
",
        Expect::SameAsVm,
    );
}

// Tables are real only with the external runtime (`nyk_rt`); the built-in
// runtime keeps stubs for them, so these cases run against the native
// build only (see docs/runtime.md).

#[test]
fn table_constructors_and_access() {
    check_extern_case(
        "table_ctor",
        "\
local arr = {10, 20, 30}
local rec = {x = 1, y = \"two\", [5] = true}
local mixed = {1, 2, name = \"m\", 3, [\"k k\"] = 4}
print(arr[1], arr[3], arr[4], #arr)
print(rec.x, rec.y, rec[5], rec.z, #rec)
print(mixed[1], mixed[3], mixed.name, mixed[\"k k\"], #mixed)
local e = {}
print(#e, e[1], e.a)
",
        Expect::SameAsVm,
    );
}

#[test]
fn table_assignment_overwrite_and_nil() {
    check_extern_case(
        "table_set",
        "\
local t = {}
t.a = 1
t[\"b\"] = 2
t[1] = \"one\"
t.a = t.a + 10
print(t.a, t.b, t[1], #t)
t.b = nil
t[1] = nil
print(t.b, t[1], #t)
local k = \"dyn\" .. \"amic\"
t[k] = 3
print(t.dynamic, t[k])
t.a = false
print(t.a, t.a == false)
",
        Expect::SameAsVm,
    );
}

#[test]
fn table_length_and_append() {
    check_extern_case(
        "table_len",
        "\
local t = {}
for i = 1, 10 do
  t[#t + 1] = i * i
end
print(#t, t[10])
t[#t] = nil
print(#t)
local g = {}
g[1] = 1
g[3] = 3
print(#g)
g[2] = 2
print(#g)
g[2] = nil
print(#g)
local h = {1, nil, 3}
print(#h, h[2], h[3])
local n = {1, 2, nil}
print(#n)
local big = {}
big[100] = 1
print(#big, big[100])
",
        Expect::SameAsVm,
    );
}

#[test]
fn table_integer_and_float_keys_are_the_same() {
    check_extern_case(
        "table_float_keys",
        "\
local t = {}
t[1.0] = \"a\"
t[2] = \"b\"
t[2.5] = \"c\"
t[-0.0] = \"zero\"
print(t[1], t[2.0], t[2.5], t[0], t[0.0], #t)
local u = {[1.0] = \"x\", [2.0] = \"y\"}
print(#u, u[1], u[2])
t[1] = nil
print(t[1.0], #t)
",
        Expect::SameAsVm,
    );
}

#[test]
fn nested_tables_and_tables_as_keys() {
    check_extern_case(
        "table_nested",
        "\
local cfg = {win = {w = 640, h = 480, pos = {1, 2}}, list = {{1}, {2, 3}}}
print(cfg.win.w, cfg.win.pos[2], cfg.list[2][2], #cfg.list[2])
cfg.win.pos[2] = 20
cfg.win.title = \"hi\"
print(cfg.win.pos[2], cfg.win.title)
local key1 = {}
local key2 = {}
local m = {}
m[key1] = \"first\"
m[key2] = \"second\"
print(m[key1], m[key2], m[{}])
local function f() end
m[f] = \"fn\"
m[true] = \"yes\"
print(m[f], m[true], m[false])
local holder = {f = f, t = key1}
print(holder.f == f, holder.t == key1, holder.t == key2)
",
        Expect::SameAsVm,
    );
}

#[test]
fn table_identity_and_sharing() {
    check_extern_case(
        "table_identity",
        "\
local function bump(t) t.n = (t.n or 0) + 1 end
local a = {}
local b = a
bump(a)
bump(b)
print(a.n, b.n, a == b, a == {}, {} == {})
local function make() return {id = 1} end
local x, y = make(), make()
print(x == y, x.id == y.id)
",
        Expect::SameAsVm,
    );
}

#[test]
fn table_holds_functions_and_methods() {
    check_extern_case(
        "table_functions",
        "\
local counter = {n = 0}
function counter.inc(self) self.n = self.n + 1 return self.n end
counter.add = function(self, k) self.n = self.n + k return self.n end
print(counter.inc(counter), counter.add(counter, 5))
print(counter:inc(), counter:add(10))
local ops = {function(a) return a + 1 end, function(a) return a * 2 end}
print(ops[1](5), ops[2](5), #ops)
",
        Expect::SameAsVm,
    );
}

#[test]
fn table_errors_match_the_vm() {
    check_extern_case(
        "table_nil_key",
        "local function set(t, k) t[k] = 1 end\nlocal t = {}\nset(t, nil)\nprint(1)\n",
        Expect::Error("table index is nil"),
    );
    check_extern_case(
        "table_nan_key",
        "local function set(t, k) t[k] = 1 end\nlocal t = {}\nset(t, 0 / 0)\nprint(1)\n",
        Expect::Error("table index is NaN"),
    );
    check_extern_case(
        "table_index_nil",
        "local function get(t) return t.x end\nlocal r = get(nil)\nprint(r)\n",
        Expect::Error("attempted to index a nil value"),
    );
    check_extern_case(
        "table_index_number",
        "local function get(t) return t[1] end\nlocal r = get(5)\nprint(r)\n",
        Expect::Error("attempted to index a number value"),
    );
    check_extern_case(
        "table_set_on_nil",
        "local function set(t) t.x = 1 end\nset(nil)\nprint(1)\n",
        Expect::Error("attempted to set a field on a nil value"),
    );
    // Reading a nil or NaN key is not an error.
    check_extern_case(
        "table_read_bad_keys",
        "local function get(t, k) return t[k] end\nlocal t = {1}\nprint(get(t, nil), get(t, 0 / 0))\n",
        Expect::SameAsVm,
    );
}

// Multiple values. Everything here but `select` and tables works with both
// runtimes, because it needs nothing from the runtime but the return area.

#[test]
fn multiple_results_are_padded_and_truncated() {
    check_case(
        "multi_results",
        "\
local function two() return 1, 2 end
local function three() return 1, 2, 3 end
local a, b, c = two()
print(a, b, c)
local d, e = three()
print(d, e)
-- Only the last expression of a list expands; the others give one value.
local f, g, h = three(), 10
print(f, g, h)
local i, j = (three())
print(i, j)
local k, l, m, n = 0, three()
print(k, l, m, n)
-- A call in parentheses gives exactly one value.
print((two()))
print((two()), (three()))
print(two(), (two()))
",
        Expect::SameAsVm,
    );
}

#[test]
fn a_call_spreads_into_the_arguments_of_another() {
    check_case(
        "spread_args",
        "\
local function two() return 1, 2 end
local function three() return 1, 2, 3 end
local function sum3(a, b, c) return a + b + c end
local function show(...) print(...) end
print(two())
print(0, three())
print(three(), 0)
print(two(), two())
print(sum3(three()))
print(sum3(0, two()))
show(three())
show(0, two())
show()
local function id(...) return ... end
print(id(id(id(1, 2, 3))))
print(id(two(), id(three())))
",
        Expect::SameAsVm,
    );
}

#[test]
fn missing_arguments_are_nil_and_extra_ones_are_dropped() {
    check_case(
        "arg_padding",
        "\
local function pad(a, b, c) return a, b, c end
local function none() end
print(pad())
print(pad(1))
print(pad(1, 2))
print(pad(1, 2, 3, 4, 5))
local function wrap(f, ...) return f(...) end
print(wrap(pad))
print(wrap(pad, 1, 2, 3, 4))
-- A function without a `return` gives one nil, as does a bare `return`.
local function bare() return end
print(none())
print(bare())
local x = none()
print(x)
",
        Expect::SameAsVm,
    );
}

#[test]
fn varargs_are_read_and_passed_on() {
    check_case(
        "varargs",
        "\
local function va(...) return ... end
local function first(...)
  local a = ...
  return a
end
local function two(...)
  local a, b = ...
  return a, b
end
local function mid(a, ...) return a, ... end
local function pass(...) return va(...) end
local function deep(...) return pass(pass(pass(...))) end
local function drop(a, b, ...) return ... end
print(va())
print(va(1, 2, 3))
print(first(7, 8, 9), first())
print(two(5), two(5, 6, 7))
print(mid(1), mid(1, 2, 3))
print(deep(4, 5, 6))
print(drop(1, 2, 3, 4), drop(1), drop())
-- nils in the middle and at the end are values.
print(va(nil, 2, nil))
print(va(nil, nil))
print(deep(nil, nil, nil))
-- The vararg values are copied at the call, so the callee may call on.
local function relay(f, ...) return f(...) end
print(relay(va, 1, 2), relay(relay, va, 3, 4))
",
        Expect::SameAsVm,
    );
}

#[test]
fn varargs_and_results_in_closures() {
    check_case(
        "varargs_closures",
        "\
local function va(...) return ... end
local function make(...)
  local a, b = ...
  -- The closure has its own `...`; it cannot see the enclosing one.
  return function(...) return a, b, ... end
end
local f = make(1, 2, 3)
print(f())
print(f(9, 8))
local function counter(first, ...)
  local n = first
  return function(...)
    n = n + 1
    return n, ...
  end
end
local c = counter(10)
print(c())
print(c(\"x\", \"y\"))
print(c(va(1, 2)))
local function pair() return 3, 4 end
local g = function() return pair() end
print(g())
print((g()))
",
        Expect::SameAsVm,
    );
}

#[test]
fn many_values_grow_the_return_area() {
    check_case(
        "many_values",
        "\
-- Prepends one value per call, so the last returns 60 values.
local function count(n, ...)
  if n == 0 then
    return ...
  end
  return count(n - 1, n, ...)
end
print(count(60))
local function last(...)
  local a, b, c = ...
  return c
end
print(last(count(60)))
local function more(...) return ..., 61, 62 end
print(more(count(3)))
print(count(0))
",
        Expect::SameAsVm,
    );
}

#[test]
fn generic_for_takes_several_values_per_step() {
    check_case(
        "generic_for_pairs",
        "\
local function upto(limit)
  local i = 0
  return function()
    i = i + 1
    if i <= limit then
      return i, i * i, \"v\"
    end
  end
end
for i, sq in upto(3) do
  print(i, sq)
end
for i in upto(2) do
  print(i)
end
",
        Expect::SameAsVm,
    );
}

#[test]
fn numeric_for_limit_and_step_can_be_calls() {
    // Evaluating the limit or the step takes temporaries, so the loop's own
    // locals do not follow each other.
    check_case(
        "for_call_limit",
        "\
local function second(a, b) return b end
local function lim(...) return ... end
for i = 1, second(0, 3) do
  print(i)
end
for i = second(0, 10), 1, -second(0, 4) do
  print(i)
end
for i = 1, lim(2) do
  print(i)
end
",
        Expect::SameAsVm,
    );
}

#[test]
fn method_calls_spread_too() {
    check_extern_case(
        "method_multi",
        "\
local obj = {}
function obj.pair(self, a) return a, a + 1 end
function obj.many(self, ...) return ... end
print(obj:pair(1))
print(obj:many(1, 2, 3))
print(obj.pair(obj, 5), obj:pair(5))
local a, b = obj:pair(7)
print(a, b)
",
        Expect::SameAsVm,
    );
}

#[test]
fn select_counts_and_slices_real_varargs() {
    check_extern_case(
        "select_varargs",
        "\
local function va(...) return ... end
local function count(...) return select(\"#\", ...) end
local function from(n, ...) return select(n, ...) end
print(count(), count(nil), count(nil, nil), count(1, 2, 3))
print(count(va()), count(va(1, 2)), count(va(1, 2), 3), count(3, va(1, 2)))
print(from(1, \"a\", \"b\", \"c\"))
print(from(2, \"a\", \"b\", \"c\"))
print(from(3, \"a\", \"b\", \"c\"))
print(from(4, \"a\", \"b\", \"c\"))
print(from(-1, \"a\", \"b\", \"c\"))
print(from(-3, \"a\", \"b\", \"c\"))
print((from(2, \"a\", \"b\", \"c\")))
print(count(from(2, 1, 2, 3)))
print(select(\"#\"))
-- A tail of the values spreads on like any other results.
local function tail(...) return select(2, ...) end
print(tail(1, 2, 3))
print(tail(1))
print(count(tail(1, 2, 3, 4)))
",
        Expect::SameAsVm,
    );
    check_extern_case(
        "select_loop",
        "\
local function sum(...)
  local total = 0
  for i = 1, select(\"#\", ...) do
    local v = select(i, ...)
    total = total + v
  end
  return total
end
print(sum(), sum(1), sum(1, 2, 3, 4))
",
        Expect::SameAsVm,
    );
}

#[test]
fn select_errors_match_the_vm() {
    check_extern_case(
        "select_zero",
        "print(\"before\")\nlocal function f(n, ...) return select(n, ...) end\nprint(f(0, 1))\n",
        Expect::Error("select expects '#' or an index within range"),
    );
    check_extern_case(
        "select_too_negative",
        "local function f(n, ...) return select(n, ...) end\nprint(f(-3, 1, 2))\n",
        Expect::Error("select expects '#' or an index within range"),
    );
    check_extern_case(
        "select_float",
        "local function f(n, ...) return select(n, ...) end\nprint(f(2.0, 1, 2))\n",
        Expect::Error("select expects '#' or an index within range"),
    );
    check_extern_case(
        "select_string",
        "local function f(n, ...) return select(n, ...) end\nprint(f(\"x\", 1, 2))\n",
        Expect::Error("select expects '#' or an index within range"),
    );
    check_extern_case(
        "spread_call_nil",
        "local function two() return 1, 2 end\nlocal f = nil\nlocal function call(g) return g(two()) end\nprint(call(f))\n",
        Expect::Error("attempted to call non-function (nil)"),
    );
}

#[test]
fn varargs_and_results_fill_tables() {
    check_extern_case(
        "table_spread",
        "\
local function two() return 1, 2 end
local function va(...) return ... end
local function none() end
local function len(...) return #{...} end
print(#{two()}, #{two(), two()}, #{(two())}, #{0, two()})
print(len(), len(1), len(1, 2, 3), len(two()), len(two(), 5))
local t = {va(\"a\", \"b\", \"c\")}
print(t[1], t[2], t[3], t[4], #t)
-- A trailing nil from `...` is kept, like the VM does.
print(len(nil), len(1, nil), len(nil, nil))
local u = {none()}
print(#u, u[1])
local v = {x = 1, va(5, 6), y = 2}
print(v[1], v[2], v.x, v.y, #v)
local function pack(...) return {n = select(\"#\", ...), ...} end
local p = pack(1, nil, 3, nil)
print(p.n, p[1], p[2], p[3], p[4])
local function unpack(t, i, j)
  i = i or 1
  j = j or #t
  if i > j then return end
  return t[i], unpack(t, i + 1, j)
end
print(unpack({10, 20, 30}))
print(select(\"#\", unpack({})), select(\"#\", unpack({1, 2})))
",
        Expect::SameAsVm,
    );
}

#[test]
fn builtin_runtime_has_real_varargs_without_select() {
    // The built-in stubs have no builtins, so `select` is nil there; the
    // varargs themselves do not need the runtime.
    check_modes(
        "builtin_varargs",
        "\
local function va(...) return ... end
local function fourth(a, b, c, d) return d end
print(va(1, 2, 3), fourth(va(1, 2, 3, 4)))
",
        &Expect::SameAsVm,
        &[Mode::Builtin],
    );
}

#[test]
fn builtin_runtime_tables_are_distinct_stubs() {
    // Only identity works with the built-in stubs: no storage.
    check_modes(
        "builtin_table_stub",
        "local t = {}\nlocal u = {}\nprint(t == t, t == u, #t)\n",
        &Expect::SameAsVm,
        &[Mode::Builtin],
    );
}

// Projects of several files. They run with the external runtime only: the
// built-in one cannot load modules, nor does it have real tables.

#[test]
fn modules_nested_paths_directories_and_tables_of_functions() {
    check_files(
        "modules_basic",
        &[
            (
                "main.nyk",
                "\
local x = require(\"util/x\")
local lib = require(\"lib\")
print(x.add(2, 3), x.name, x.twice(x.add)(1, 2))
print(lib.greet(\"neyuki\"), lib.count())
print(lib.count())
",
            ),
            (
                "util/x.nyk",
                "\
local M = { name = \"x\" }
function M.add(a, b) return a + b end
function M.twice(f) return function(a, b) return f(f(a, b), b) end end
return M
",
            ),
            (
                "lib/init.nyk",
                "\
local calls = 0
local M = {}
function M.greet(who) return \"hello \" .. who end
function M.count() calls = calls + 1 return calls end
return M
",
            ),
        ],
        &Expect::SameAsVm,
        &[Mode::Extern],
    );
}

#[test]
fn modules_may_return_anything() {
    check_files(
        "modules_return_values",
        &[
            (
                "main.nyk",
                "\
print(require(\"number\"), require(\"text\"), require(\"nothing\"), require(\"flag\"))
print(require(\"fn\") == require(\"fn\"), require(\"fn\")(4))
local first = require(\"many\")
print(first)
",
            ),
            ("number.nyk", "return 12.5\n"),
            ("text.nyk", "return \"hi\"\n"),
            ("nothing.nyk", "local unused = 1\n"),
            ("flag.nyk", "return false\n"),
            ("fn.nyk", "return function(n) return n * n end\n"),
            ("many.nyk", "return 1, 2, 3\n"),
        ],
        &Expect::SameAsVm,
        &[Mode::Extern],
    );
}

#[test]
fn a_module_runs_once_whatever_its_spelling_and_in_order() {
    check_files(
        "modules_once_and_order",
        &[
            (
                "main.nyk",
                "\
print(\"main 1\")
local a = require(\"a\")
print(\"main 2\")
local again = require(\"./a.nyk\")
local dotted = require(\"sub.b\")
local b = require(\"sub/b\")
print(a == again, dotted == b, a.tag, b.tag)
print(\"main 3\")
",
            ),
            (
                "a.nyk",
                "\
print(\"a starts\")
local b = require(\"sub/b\")
print(\"a has\", b.tag)
return { tag = \"A\" }
",
            ),
            (
                "sub/b.nyk",
                "\
print(\"b runs\")
return { tag = \"B\" }
",
            ),
        ],
        &Expect::SameAsVm,
        &[Mode::Extern],
    );
}

#[test]
fn a_require_cycle_gets_an_empty_table_like_in_the_vm() {
    check_files(
        "modules_cycle",
        &[
            (
                "main.nyk",
                "\
local a = require(\"a\")
print(\"main\", a.name, a.seen_b)
print(require(\"b\").name, require(\"b\").saw_a_name)
",
            ),
            (
                "a.nyk",
                "\
print(\"a starts\")
local b = require(\"b\")
print(\"a sees b\", b.name)
return { name = \"a\", seen_b = b.name }
",
            ),
            (
                "b.nyk",
                "\
print(\"b starts\")
local a = require(\"a\")
print(\"b sees a\", a.name, a ~= nil)
return { name = \"b\", saw_a_name = a.name }
",
            ),
        ],
        &Expect::SameAsVm,
        &[Mode::Extern],
    );
}

#[test]
fn modules_with_the_same_names_do_not_clash() {
    check_files(
        "modules_same_names",
        &[
            (
                "main.nyk",
                "\
local function helper() return \"main\" end
local value = 1
local one = require(\"one\")
local two = require(\"two\")
print(helper(), value, one.call(), two.call(), one.next(), one.next(), two.next())
print(one.make()(), two.make()())
",
            ),
            (
                "one.nyk",
                "\
local function helper() return \"one\" end
local value = 10
local n = 0
local M = {}
function M.call() return helper() .. value end
function M.next() n = n + 1 return n end
function M.make() local v = \"made by one\" return function() return v end end
return M
",
            ),
            (
                "two.nyk",
                "\
local function helper() return \"two\" end
local value = 20
local n = 100
local M = {}
function M.call() return helper() .. value end
function M.next() n = n + 1 return n end
function M.make() local v = \"made by two\" return function() return v end end
return M
",
            ),
        ],
        &Expect::SameAsVm,
        &[Mode::Extern],
    );
}

#[test]
fn the_entry_comes_from_neyuki_toml_and_modules_from_its_directory() {
    check_files(
        "modules_manifest",
        &[
            (
                "neyuki.toml",
                "[package]\nname = \"game\"\nversion = \"0.1.0\"\nentry = \"start.nyk\"\n",
            ),
            (
                "start.nyk",
                "print(require(\"deep/er/mod\").path, require(\"deep/helper\"))\n",
            ),
            // Every require is relative to the entry script, not to the file
            // that makes it.
            (
                "deep/er/mod.nyk",
                "return { path = \"deep/er/mod via \" .. require(\"deep/helper\") }\n",
            ),
            ("deep/helper.nyk", "return \"helper\"\n"),
        ],
        &Expect::SameAsVm,
        &[Mode::Extern],
    );
}

#[test]
fn a_dynamic_require_loads_modules_the_project_names_elsewhere() {
    check_files(
        "modules_dynamic",
        &[
            (
                "main.nyk",
                "\
local function load(name) return require(name) end
local known = require(\"known\")
print(load(\"known\") == known, load(\"known\").tag)
",
            ),
            ("known.nyk", "return { tag = \"known\" }\n"),
        ],
        &Expect::SameAsVm,
        &[Mode::Extern],
    );
}

/// Writes `files` into a scratch directory and runs `neyuki args... <dir>
/// -o <dir>/app` there.
fn neyuki_on_project(name: &str, files: &[(&str, &str)], args: &[&str]) -> (PathBuf, Run) {
    let dir = scratch_dir(name);
    for (file, text) in files {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        std::fs::write(&path, text).expect("write file");
    }
    let mut cmd = neyuki();
    cmd.args(args)
        .arg(&dir)
        .arg("-o")
        .arg(dir.join("app"))
        .env("NEYUKI_RT_LIB", nyk_rt());
    if let Some(clang) = std::env::var_os("NEYUKI_CLANG") {
        cmd.env("NEYUKI_CLANG", clang);
    }
    let out = cmd.output().expect("run neyuki");
    (dir, out.into())
}

#[test]
fn a_missing_module_is_a_build_error() {
    // It fails before clang or the runtime library are needed. `emit-llvm`
    // takes the output as a positional argument, so only `build` gets `-o`.
    let files = [(
        "main.nyk",
        "print(1)\nlocal function f() return require(\"nope/missing\") end\n",
    )];
    let (dir, run) = neyuki_on_project("modules_missing", &files, &["build", "--native"]);
    assert!(!run.success);
    assert!(
        run.stderr
            .contains("main.nyk: cannot find module 'nope/missing'"),
        "{}",
        run.stderr
    );
    let out = neyuki()
        .args(["emit-llvm", "--runtime=extern"])
        .arg(&dir)
        .output()
        .expect("run neyuki emit-llvm");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot find module 'nope/missing'"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dynamic_require_of_an_unknown_module_warns_and_fails_when_run() {
    let Some(clang) = clang() else {
        return;
    };
    let files = [
        (
            "main.nyk",
            "\
print(\"start\")
local n = \"oth\" .. \"er\"
local function load(name) return require(name) end
print(load(n))
",
        ),
        ("other.nyk", "return 1\n"),
    ];
    let mut cmd = neyuki();
    let dir = scratch_dir("modules_dynamic_unknown");
    for (file, text) in files {
        std::fs::write(dir.join(file), text).expect("write file");
    }
    let exe = dir.join("app");
    let build: Run = cmd
        .args(["build", "--native"])
        .arg(&dir)
        .arg("-o")
        .arg(&exe)
        .env("NEYUKI_CLANG", clang)
        .env("NEYUKI_RT_LIB", nyk_rt())
        .output()
        .expect("run neyuki")
        .into();
    assert!(build.success, "{}", build.stderr);
    assert!(
        build.stderr.contains("warning:") && build.stderr.contains("not a constant string"),
        "{}",
        build.stderr
    );
    let run: Run = Command::new(&exe).output().expect("run executable").into();
    assert_eq!(run.stdout, "start\n");
    assert!(!run.success);
    assert!(
        run.stderr
            .contains("runtime error: cannot find module 'other'"),
        "{}",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}
