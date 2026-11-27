//! Runs small programs on the register VM and through the LLVM backend
//! (`neyuki emit-llvm`, checked with `opt -passes=verify`, run with `lli`)
//! and checks that both print the same thing.
//!
//! The LLVM tools are found in `NEYUKI_LLVM_BIN` if set, else on `PATH`.
//! Without them the tests skip, unless `NEYUKI_REQUIRE_LLVM=1` (as in CI)
//! makes a missing tool a failure.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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

/// The LLVM tool `name`, or `None` (after saying so) when it is missing and
/// not required.
fn llvm_tool(name: &str) -> Option<PathBuf> {
    let path = match std::env::var_os("NEYUKI_LLVM_BIN") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
    };
    let works = Command::new(&path)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if works {
        return Some(path);
    }
    if std::env::var_os("NEYUKI_REQUIRE_LLVM").is_some_and(|v| v == "1") {
        panic!(
            "NEYUKI_REQUIRE_LLVM=1 but `{}` does not run",
            path.display()
        );
    }
    eprintln!("skipping: LLVM tool `{}` not found", name);
    None
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

/// Emits `script` as LLVM IR, checks it with `opt -passes=verify` and returns
/// the `.ll` path.
fn emit_verified(dir: &Path, script: &Path, opt: &Path) -> PathBuf {
    let ll = dir.join("out.ll");
    let emit = neyuki()
        .arg("emit-llvm")
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

fn check_case(name: &str, src: &str, expect: Expect) {
    let (Some(opt), Some(lli)) = (llvm_tool("opt"), llvm_tool("lli")) else {
        return;
    };
    let dir = scratch_dir(name);
    let script = dir.join(format!("{}.nyk", name));
    std::fs::write(&script, src).expect("write script");

    let vm: Run = neyuki().arg(&script).output().expect("run vm").into();
    let ll = emit_verified(&dir, &script, &opt);
    let native: Run = Command::new(&lli)
        .arg(&ll)
        .output()
        .expect("run lli")
        .into();

    let context = format!(
        "case `{}`:\n{}\n--- vm stdout ---\n{}--- vm stderr ---\n{}--- lli stdout ---\n{}--- lli stderr ---\n{}",
        name, src, vm.stdout, vm.stderr, native.stdout, native.stderr
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
            assert!(native.success, "lli failed, {}", context);
            assert_eq!(native.stdout, out, "unexpected output, {}", context);
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
                "lli error lacks `{}`, {}",
                msg,
                context
            );
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
    files.sort();
    assert!(!files.is_empty(), "no .nyk files found");
    let dir = scratch_dir("all");
    for script in &files {
        emit_verified(&dir, script, &opt);
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
