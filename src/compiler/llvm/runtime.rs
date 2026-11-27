//! The runtime prelude of an emitted module: the `%NykVal` type, the small
//! helpers that are always defined in the IR (and inline), and the runtime
//! functions, either defined here as a self-contained fallback or declared
//! for an external runtime to provide.
//!
//! # Runtime ABI
//!
//! The helpers (`__nyk_mk_*`, `__nyk_truthy`, arithmetic, comparisons,
//! `print`) are internal to the module, so they take and return `%NykVal`
//! by value. The runtime functions listed in [`EXTERN_DECLS`] cross into
//! another compilation unit (`crates/nyk_rt` in extern mode), where a
//! 16-byte struct by value is not portable: the Windows x64 C ABI passes
//! and returns it through a hidden pointer, while LLVM would split the
//! aggregate into registers. Those functions therefore follow this
//! convention, in both modes so that the call sites are the same:
//!
//! - a value argument is two scalars, `i8 zeroext` tag and `i64` raw bits
//!   (`u8` and `i64` in Rust);
//! - a value result is written through a `ptr` out-parameter, which comes
//!   first, and the function returns `void`;
//! - an argument list is a `ptr` to a `[N x %NykVal]` array in memory, laid
//!   out per the module's `target datalayout` (`#[repr(C)]` in Rust).
//!
//! # Calls and multiple values
//!
//! Every function the compiler generates has the same signature,
//! `i32 @f(ptr %env, ptr %args, i32 %argc)`, so that a direct call, a call
//! through `__nyk_call_dyn` and a call made by the runtime look alike
//! whatever the number of arguments or results:
//!
//! - the arguments are `%argc` values in the `[N x %NykVal]` array at
//!   `%args` (null when there are none). The callee reads its `k`th
//!   parameter as nil when `k >= %argc`, ignores the surplus, and reads `...`
//!   as the values from `%args[nparams]` on. The array belongs to the caller
//!   and outlives the call;
//! - the function returns the number of results `n`, and the results are in
//!   the runtime's *return area*: `n` values at the pointer
//!   `__nyk_ret_buf(n)` gives (`__nyk_ret_buf(0)` is the current one). A
//!   callee reserves room for its results and stores them as its last act,
//!   and the caller reads them before it calls anything that returns results
//!   itself or that grows the area, padding with nil or dropping the extra
//!   as its context wants. A call that is the last in an argument list or
//!   table constructor, or in a `return`, keeps all its results: they are
//!   copied into the new list straight away. The generated code also copies
//!   the arguments of such a call to the stack (with `llvm.stacksave`, as
//!   their number is not known at compile time);
//! - `__nyk_call_dyn(tag, raw, argc, argv)` follows the same convention for
//!   anything that is not a generated function: it returns the number of
//!   results, which are in the return area.
//!
//! A function that returns without values (a bare `return`, or the end of the
//! function) returns one nil, as the VM's bytecode does.
//!
//! # Closures
//!
//! A function value (tag 6) points to a closure object, an array of
//! pointers: the function's code, then one cell per captured variable. A
//! cell is a heap `%NykVal` shared by every closure that captures the
//! variable and by the function that declared it. The generated code reads
//! the code pointer of a closure object to call it, and passes the object
//! as the callee's hidden first parameter `%env`; it reads and writes cells
//! of its own locals directly, and upvalues only through `__nyk_upval_*`.
//! A function that captures nothing has a static object `@<fn>.closure`.

use std::fmt::Write;

use crate::compiler::llvm::types::format_llvm_cstring;

/// Where the runtime functions come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Defined in the module as stubs (plus a working `..`, `#` on strings
    /// and errors), so `lli` and `clang` can run it on its own.
    #[default]
    Builtin,
    /// Only declared; the program is linked against `libnyk_rt.a`.
    Extern,
}

/// Runtime error messages the inlined helpers raise through `__nyk_error`,
/// as `(symbol, message)`. The texts are the register VM's (src/vm/ops.rs).
const ERROR_MESSAGES: &[(&str, &str)] = &[
    ("@.err_number", "expected number"),
    ("@.err_integer", "expected integer"),
    ("@.err_float_to_int", "cannot convert float to integer"),
    ("@.err_div_zero", "division by zero"),
    ("@.err_mod_zero", "modulo by zero"),
    ("@.err_unary_minus", "unary minus expects a number"),
    ("@.err_len", "len expects string, table or buffer"),
];

/// The runtime functions, as an external runtime must provide them (see the
/// ABI notes above). [`BUILTIN_RUNTIME`] defines the same signatures.
pub const EXTERN_DECLS: &str = r#"; Runtime functions, provided by libnyk_rt (crates/nyk_rt)
declare void @__nyk_error(ptr) noreturn cold
declare void @__nyk_global_get(ptr, ptr)
declare void @__nyk_global_set(ptr, i8 zeroext, i64)
declare void @__nyk_upval_get(ptr, ptr, i32)
declare void @__nyk_upval_set(ptr, i32, i8 zeroext, i64)
declare ptr @__nyk_upval_cell(ptr, i32)
declare ptr @__nyk_cell_new(i8 zeroext, i64)
declare void @__nyk_import_get(ptr, ptr, ptr)
declare void @__nyk_table_new(ptr, i32, i32)
declare void @__nyk_table_get(ptr, i8 zeroext, i64, i8 zeroext, i64)
declare void @__nyk_table_set(i8 zeroext, i64, i8 zeroext, i64, i8 zeroext, i64)
declare void @__nyk_table_append(i8 zeroext, i64, i8 zeroext, i64)
declare void @__nyk_concat(ptr, i8 zeroext, i64, i8 zeroext, i64)
declare void @__nyk_len(ptr, i8 zeroext, i64)
declare void @__nyk_closure(ptr, ptr, i32, ptr)
declare ptr @__nyk_ret_buf(i32)
declare i32 @__nyk_call_dyn(i8 zeroext, i64, i32, ptr)
declare void @__nyk_module(ptr, ptr)

"#;

/// The built-in fallback runtime, with the signatures of [`EXTERN_DECLS`].
const BUILTIN_RUNTIME: &str = r#"; Built-in fallback runtime implementations (self-contained for lli/clang)

; Raises a runtime error: prints `msg` to stderr and exits with status 1,
; as the VM does. `exit` rather than `abort` flushes what `print` buffered,
; and does not look like a crash of `lli` itself (which reports SIGABRT as
; an LLVM bug with a stack dump).
@.fmt_error = private unnamed_addr constant [19 x i8] c"runtime error: %s\0A\00"

define internal void @__nyk_error(ptr %msg) noreturn cold {
  call i32 @fflush(ptr null)
  call i32 (i32, ptr, ...) @dprintf(i32 2, ptr @.fmt_error, ptr %msg)
  call void @exit(i32 1)
  unreachable
}

define internal void @__nyk_global_get(ptr %out, ptr %name) {
  store %NykVal zeroinitializer, ptr %out
  ret void
}

define internal void @__nyk_global_set(ptr %name, i8 zeroext %tag, i64 %raw) {
  ret void
}

; A new cell (a heap value) holding the given value.
define internal ptr @__nyk_cell_new(i8 zeroext %tag, i64 %raw) {
  %c = call ptr @malloc(i64 16)
  %v0 = insertvalue %NykVal zeroinitializer, i8 %tag, 0
  %v = insertvalue %NykVal %v0, i64 %raw, 1
  store %NykVal %v, ptr %c
  ret ptr %c
}

; The cell of upvalue `idx` of the closure `env`: word `idx + 1` of it.
define internal ptr @__nyk_upval_cell(ptr %env, i32 %idx) {
  %i = zext i32 %idx to i64
  %word = add i64 %i, 1
  %gep = getelementptr inbounds ptr, ptr %env, i64 %word
  %c = load ptr, ptr %gep
  ret ptr %c
}

define internal void @__nyk_upval_get(ptr %out, ptr %env, i32 %idx) {
  %c = call ptr @__nyk_upval_cell(ptr %env, i32 %idx)
  %v = load %NykVal, ptr %c
  store %NykVal %v, ptr %out
  ret void
}

define internal void @__nyk_upval_set(ptr %env, i32 %idx, i8 zeroext %tag, i64 %raw) {
  %c = call ptr @__nyk_upval_cell(ptr %env, i32 %idx)
  %v0 = insertvalue %NykVal zeroinitializer, i8 %tag, 0
  %v = insertvalue %NykVal %v0, i64 %raw, 1
  store %NykVal %v, ptr %c
  ret void
}

define internal void @__nyk_import_get(ptr %out, ptr %mod, ptr %field) {
  store %NykVal zeroinitializer, ptr %out
  ret void
}

; Tables are stubs here: a hash table is not worth writing in IR, so real
; tables exist only with libnyk_rt (--runtime=extern). Reads give nil and
; writes do nothing, but each table gets its own allocation so that two
; tables are not equal.
define internal void @__nyk_table_new(ptr %out, i32 %arr, i32 %map) {
  %p = call ptr @malloc(i64 16)
  %addr = ptrtoint ptr %p to i64
  %s0 = insertvalue %NykVal { i8 5, i64 0 }, i64 %addr, 1
  store %NykVal %s0, ptr %out
  ret void
}

define internal void @__nyk_table_get(ptr %out, i8 zeroext %t_tag, i64 %t_raw, i8 zeroext %k_tag, i64 %k_raw) {
  store %NykVal zeroinitializer, ptr %out
  ret void
}

define internal void @__nyk_table_set(i8 zeroext %t_tag, i64 %t_raw, i8 zeroext %k_tag, i64 %k_raw, i8 zeroext %v_tag, i64 %v_raw) {
  ret void
}

define internal void @__nyk_table_append(i8 zeroext %t_tag, i64 %t_raw, i8 zeroext %v_tag, i64 %v_raw) {
  ret void
}

; A value as a NUL-terminated string, for `..`: numbers are formatted like
; `print` does, a table is "table" and a function "function". Results are
; malloc'd and never freed (this fallback has no collector).
define internal ptr @__nyk_tostr(i8 %tag, i64 %raw) {
  switch i8 %tag, label %s_fn [
    i8 0, label %s_nil
    i8 1, label %s_bool
    i8 2, label %s_int
    i8 3, label %s_float
    i8 4, label %s_str
    i8 5, label %s_table
  ]
s_nil:
  ret ptr @.fmt_nil
s_bool:
  %b = icmp ne i64 %raw, 0
  %bs = select i1 %b, ptr @.fmt_true, ptr @.fmt_false
  ret ptr %bs
s_int:
  %ibuf = call ptr @malloc(i64 24)
  call i32 (ptr, i64, ptr, ...) @snprintf(ptr %ibuf, i64 24, ptr @.fmt_int, i64 %raw)
  ret ptr %ibuf
s_float:
  %f = bitcast i64 %raw to double
  %fbuf = call ptr @malloc(i64 32)
  call i32 (ptr, i64, ptr, ...) @snprintf(ptr %fbuf, i64 32, ptr @.fmt_float, double %f)
  ret ptr %fbuf
s_str:
  %p = inttoptr i64 %raw to ptr
  ret ptr %p
s_table:
  ret ptr @.str_table
s_fn:
  ret ptr @.str_function
}

@.str_table = private unnamed_addr constant [6 x i8] c"table\00"
@.str_function = private unnamed_addr constant [9 x i8] c"function\00"

define internal void @__nyk_concat(ptr %out, i8 zeroext %a_tag, i64 %a_raw, i8 zeroext %b_tag, i64 %b_raw) {
  %sa = call ptr @__nyk_tostr(i8 %a_tag, i64 %a_raw)
  %sb = call ptr @__nyk_tostr(i8 %b_tag, i64 %b_raw)
  %la = call i64 @strlen(ptr %sa)
  %lb = call i64 @strlen(ptr %sb)
  %len = add i64 %la, %lb
  %size = add i64 %len, 1
  %buf = call ptr @malloc(i64 %size)
  call void @llvm.memcpy.p0.p0.i64(ptr %buf, ptr %sa, i64 %la, i1 false)
  %tail = getelementptr inbounds i8, ptr %buf, i64 %la
  call void @llvm.memcpy.p0.p0.i64(ptr %tail, ptr %sb, i64 %lb, i1 false)
  %end = getelementptr inbounds i8, ptr %buf, i64 %len
  store i8 0, ptr %end
  %r = call %NykVal @__nyk_mk_str(ptr %buf)
  store %NykVal %r, ptr %out
  ret void
}

; `#v`: the byte length of a string, 0 for a (stub) table.
define internal void @__nyk_len(ptr %out, i8 zeroext %tag, i64 %raw) {
  switch i8 %tag, label %bad [
    i8 4, label %str
    i8 5, label %table
  ]
str:
  %p = inttoptr i64 %raw to ptr
  %n = call i64 @strlen(ptr %p)
  %rs = call %NykVal @__nyk_mk_int(i64 %n)
  store %NykVal %rs, ptr %out
  ret void
table:
  store %NykVal { i8 2, i64 0 }, ptr %out
  ret void
bad:
  call void @__nyk_error(ptr @.err_len)
  unreachable
}

; A closure object: `fn_ptr` followed by the `ncaps` cells at `caps`.
define internal void @__nyk_closure(ptr %out, ptr %fn_ptr, i32 %ncaps, ptr %caps) {
  %n = zext i32 %ncaps to i64
  %ptr_end = getelementptr ptr, ptr null, i64 1
  %ptr_size = ptrtoint ptr %ptr_end to i64
  %words = add i64 %n, 1
  %size = mul i64 %words, %ptr_size
  %obj = call ptr @malloc(i64 %size)
  store ptr %fn_ptr, ptr %obj
  %cells = getelementptr inbounds ptr, ptr %obj, i64 1
  %cap_bytes = mul i64 %n, %ptr_size
  call void @llvm.memcpy.p0.p0.i64(ptr %cells, ptr %caps, i64 %cap_bytes, i1 false)
  %addr = ptrtoint ptr %obj to i64
  %s0 = insertvalue %NykVal { i8 6, i64 0 }, i64 %addr, 1
  store %NykVal %s0, ptr %out
  ret void
}

; Calling something that is not a generated function: no builtins here, so
; it does nothing and returns no results.
define internal i32 @__nyk_call_dyn(i8 zeroext %tag, i64 %raw, i32 %argc, ptr %argv) {
  ret i32 0
}

; Modules are registered for `require` by libnyk_rt only; a program of
; several modules needs --runtime=extern.
define internal void @__nyk_module(ptr %name, ptr %closure) {
  ret void
}

; The return area (see "Calls and multiple values"): room for at least `n`
; values, growing and keeping what it holds.
@__nyk_ret_area = internal global ptr null
@__nyk_ret_cap = internal global i64 0

define internal ptr @__nyk_ret_buf(i32 %n) {
  %need = zext i32 %n to i64
  %cap = load i64, ptr @__nyk_ret_cap
  %grow = icmp ugt i64 %need, %cap
  br i1 %grow, label %grow_bb, label %done
grow_bb:
  %big = icmp ugt i64 %need, 16
  %newcap = select i1 %big, i64 %need, i64 16
  %old = load ptr, ptr @__nyk_ret_area
  %val_end = getelementptr %NykVal, ptr null, i64 1
  %val_size = ptrtoint ptr %val_end to i64
  %bytes = mul i64 %newcap, %val_size
  %new = call ptr @realloc(ptr %old, i64 %bytes)
  store ptr %new, ptr @__nyk_ret_area
  store i64 %newcap, ptr @__nyk_ret_cap
  br label %done
done:
  %p = load ptr, ptr @__nyk_ret_area
  ret ptr %p
}

"#;

/// Emits the `%NykVal` type, the libc and intrinsic declarations, the
/// runtime functions for `mode` and the inlined helpers.
pub fn emit_runtime_prelude(out: &mut String, mode: RuntimeMode) {
    out.push_str(
        r#"%NykVal = type { i8, i64 }

declare i32 @printf(ptr, ...)
declare i32 @dprintf(i32, ptr, ...)
declare i32 @fflush(ptr)
declare i32 @snprintf(ptr, i64, ptr, ...)
declare i32 @strcmp(ptr, ptr)
declare i64 @strlen(ptr)
declare ptr @malloc(i64)
declare ptr @realloc(ptr, i64)
declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)
declare void @llvm.memmove.p0.p0.i64(ptr, ptr, i64, i1)
declare ptr @llvm.stacksave.p0()
declare void @llvm.stackrestore.p0(ptr)
declare void @exit(i32) noreturn
declare double @llvm.fabs.f64(double)
declare double @llvm.floor.f64(double)
declare double @llvm.pow.f64(double, double)
declare i64 @llvm.fptosi.sat.i64.f64(double)

"#,
    );
    for (sym, msg) in ERROR_MESSAGES {
        let (escaped, len) = format_llvm_cstring(msg);
        let _ = writeln!(
            out,
            "{} = private unnamed_addr constant [{} x i8] {}",
            sym, len, escaped
        );
    }
    out.push('\n');
    out.push_str(match mode {
        RuntimeMode::Builtin => BUILTIN_RUNTIME,
        RuntimeMode::Extern => EXTERN_DECLS,
    });
    out.push_str(
        r#"@.fmt_int = private unnamed_addr constant [5 x i8] c"%lld\00"
@.fmt_float = private unnamed_addr constant [3 x i8] c"%g\00"
@.fmt_str = private unnamed_addr constant [3 x i8] c"%s\00"
@.fmt_true = private unnamed_addr constant [5 x i8] c"true\00"
@.fmt_false = private unnamed_addr constant [6 x i8] c"false\00"
@.fmt_nil = private unnamed_addr constant [4 x i8] c"nil\00"
@.fmt_nl = private unnamed_addr constant [2 x i8] c"\0A\00"
@.fmt_sp = private unnamed_addr constant [2 x i8] c"\09\00"

define internal %NykVal @__nyk_mk_nil() alwaysinline {
  ret %NykVal zeroinitializer
}

define internal %NykVal @__nyk_mk_bool(i1 %b) alwaysinline {
  %ext = zext i1 %b to i64
  %s0 = insertvalue %NykVal { i8 1, i64 0 }, i64 %ext, 1
  ret %NykVal %s0
}

define internal %NykVal @__nyk_mk_int(i64 %v) alwaysinline {
  %s0 = insertvalue %NykVal { i8 2, i64 0 }, i64 %v, 1
  ret %NykVal %s0
}

define internal %NykVal @__nyk_mk_float(double %f) alwaysinline {
  %bits = bitcast double %f to i64
  %s0 = insertvalue %NykVal { i8 3, i64 0 }, i64 %bits, 1
  ret %NykVal %s0
}

define internal %NykVal @__nyk_mk_str(ptr %p) alwaysinline {
  %addr = ptrtoint ptr %p to i64
  %s0 = insertvalue %NykVal { i8 4, i64 0 }, i64 %addr, 1
  ret %NykVal %s0
}

define internal %NykVal @__nyk_mk_fn(ptr %p) alwaysinline {
  %addr = ptrtoint ptr %p to i64
  %s0 = insertvalue %NykVal { i8 6, i64 0 }, i64 %addr, 1
  ret %NykVal %s0
}

define internal i1 @__nyk_truthy(%NykVal %v) alwaysinline {
  %tag = extractvalue %NykVal %v, 0
  %is_nil = icmp eq i8 %tag, 0
  br i1 %is_nil, label %ret_false, label %check_bool
check_bool:
  %is_bool = icmp eq i8 %tag, 1
  br i1 %is_bool, label %chk_val, label %ret_true
chk_val:
  %raw = extractvalue %NykVal %v, 1
  %nz = icmp ne i64 %raw, 0
  ret i1 %nz
ret_false:
  ret i1 false
ret_true:
  ret i1 true
}

; Arithmetic follows the register VM (src/vm/ops.rs) on 64-bit integers:
; `//` and `%` round toward negative infinity, a zero divisor is an error,
; and shifts by 64 or more bits are defined. Where the VM would widen an
; integer to a bignum this backend wraps instead.

define internal double @__nyk_as_f64(%NykVal %v) alwaysinline {
  %tag = extractvalue %NykVal %v, 0
  %raw = extractvalue %NykVal %v, 1
  switch i8 %tag, label %bad [
    i8 2, label %as_int
    i8 3, label %as_flt
  ]
as_flt:
  %f = bitcast i64 %raw to double
  ret double %f
as_int:
  %fi = sitofp i64 %raw to double
  ret double %fi
bad:
  call void @__nyk_error(ptr @.err_number)
  unreachable
}

; The integer operand of a bitwise operator: floats are truncated toward zero
; (saturating at the i64 range), like the VM; anything else is an error.
define internal i64 @__nyk_as_i64(%NykVal %v) alwaysinline {
  %tag = extractvalue %NykVal %v, 0
  %raw = extractvalue %NykVal %v, 1
  switch i8 %tag, label %bad [
    i8 2, label %as_int
    i8 3, label %as_flt
  ]
as_int:
  ret i64 %raw
as_flt:
  %f = bitcast i64 %raw to double
  %fabs = call double @llvm.fabs.f64(double %f)
  %finite = fcmp one double %fabs, 0x7FF0000000000000
  br i1 %finite, label %conv, label %not_finite
conv:
  %i = call i64 @llvm.fptosi.sat.i64.f64(double %f)
  ret i64 %i
not_finite:
  call void @__nyk_error(ptr @.err_float_to_int)
  unreachable
bad:
  call void @__nyk_error(ptr @.err_integer)
  unreachable
}

define internal %NykVal @__nyk_add(%NykVal %a, %NykVal %b) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %tb = extractvalue %NykVal %b, 0
  %ai = icmp eq i8 %ta, 2
  %bi = icmp eq i8 %tb, 2
  %both_int = and i1 %ai, %bi
  br i1 %both_int, label %int_path, label %flt_path
int_path:
  %va = extractvalue %NykVal %a, 1
  %vb = extractvalue %NykVal %b, 1
  %sum = add i64 %va, %vb
  %ri = call %NykVal @__nyk_mk_int(i64 %sum)
  ret %NykVal %ri
flt_path:
  %fa = call double @__nyk_as_f64(%NykVal %a)
  %fb = call double @__nyk_as_f64(%NykVal %b)
  %fsum = fadd double %fa, %fb
  %rf = call %NykVal @__nyk_mk_float(double %fsum)
  ret %NykVal %rf
}

define internal %NykVal @__nyk_sub(%NykVal %a, %NykVal %b) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %tb = extractvalue %NykVal %b, 0
  %ai = icmp eq i8 %ta, 2
  %bi = icmp eq i8 %tb, 2
  %both_int = and i1 %ai, %bi
  br i1 %both_int, label %int_path, label %flt_path
int_path:
  %va = extractvalue %NykVal %a, 1
  %vb = extractvalue %NykVal %b, 1
  %diff = sub i64 %va, %vb
  %ri = call %NykVal @__nyk_mk_int(i64 %diff)
  ret %NykVal %ri
flt_path:
  %fa = call double @__nyk_as_f64(%NykVal %a)
  %fb = call double @__nyk_as_f64(%NykVal %b)
  %fdiff = fsub double %fa, %fb
  %rf = call %NykVal @__nyk_mk_float(double %fdiff)
  ret %NykVal %rf
}

define internal %NykVal @__nyk_mul(%NykVal %a, %NykVal %b) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %tb = extractvalue %NykVal %b, 0
  %ai = icmp eq i8 %ta, 2
  %bi = icmp eq i8 %tb, 2
  %both_int = and i1 %ai, %bi
  br i1 %both_int, label %int_path, label %flt_path
int_path:
  %va = extractvalue %NykVal %a, 1
  %vb = extractvalue %NykVal %b, 1
  %prod = mul i64 %va, %vb
  %ri = call %NykVal @__nyk_mk_int(i64 %prod)
  ret %NykVal %ri
flt_path:
  %fa = call double @__nyk_as_f64(%NykVal %a)
  %fb = call double @__nyk_as_f64(%NykVal %b)
  %fprod = fmul double %fa, %fb
  %rf = call %NykVal @__nyk_mk_float(double %fprod)
  ret %NykVal %rf
}

; `/` is always floating point and follows IEEE 754: dividing by zero gives
; an infinity or NaN, not an error.
define internal %NykVal @__nyk_div(%NykVal %a, %NykVal %b) alwaysinline {
  %fa = call double @__nyk_as_f64(%NykVal %a)
  %fb = call double @__nyk_as_f64(%NykVal %b)
  %q = fdiv double %fa, %fb
  %rf = call %NykVal @__nyk_mk_float(double %q)
  ret %NykVal %rf
}

; Floor division: the quotient rounded toward negative infinity.
define internal %NykVal @__nyk_idiv(%NykVal %a, %NykVal %b) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %tb = extractvalue %NykVal %b, 0
  %ai = icmp eq i8 %ta, 2
  %bi = icmp eq i8 %tb, 2
  %both_int = and i1 %ai, %bi
  br i1 %both_int, label %int_path, label %flt_path
int_path:
  %va = extractvalue %NykVal %a, 1
  %vb = extractvalue %NykVal %b, 1
  %is_zero = icmp eq i64 %vb, 0
  br i1 %is_zero, label %by_zero, label %nonzero
nonzero:
  ; sdiv is undefined for INT64_MIN / -1, so `a // -1` negates instead
  ; (wrapping INT64_MIN to itself).
  %is_m1 = icmp eq i64 %vb, -1
  br i1 %is_m1, label %by_m1, label %int_div
by_m1:
  %neg = sub i64 0, %va
  %rn = call %NykVal @__nyk_mk_int(i64 %neg)
  ret %NykVal %rn
int_div:
  %q = sdiv i64 %va, %vb
  %r = srem i64 %va, %vb
  ; sdiv truncates; step down when the remainder and divisor differ in sign.
  %r_nz = icmp ne i64 %r, 0
  %signs = xor i64 %r, %vb
  %signs_differ = icmp slt i64 %signs, 0
  %adjust = and i1 %r_nz, %signs_differ
  %adjust64 = zext i1 %adjust to i64
  %fq = sub i64 %q, %adjust64
  %ri = call %NykVal @__nyk_mk_int(i64 %fq)
  ret %NykVal %ri
flt_path:
  %fa = call double @__nyk_as_f64(%NykVal %a)
  %fb = call double @__nyk_as_f64(%NykVal %b)
  %f_zero = fcmp oeq double %fb, 0.0
  br i1 %f_zero, label %by_zero, label %flt_div
flt_div:
  %fq0 = fdiv double %fa, %fb
  %ffq = call double @llvm.floor.f64(double %fq0)
  %rf = call %NykVal @__nyk_mk_float(double %ffq)
  ret %NykVal %rf
by_zero:
  call void @__nyk_error(ptr @.err_div_zero)
  unreachable
}

; Floor modulo: a % b == a - (a // b) * b, so the result has the sign of b.
define internal %NykVal @__nyk_mod(%NykVal %a, %NykVal %b) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %tb = extractvalue %NykVal %b, 0
  %ai = icmp eq i8 %ta, 2
  %bi = icmp eq i8 %tb, 2
  %both_int = and i1 %ai, %bi
  br i1 %both_int, label %int_path, label %flt_path
int_path:
  %va = extractvalue %NykVal %a, 1
  %vb = extractvalue %NykVal %b, 1
  %is_zero = icmp eq i64 %vb, 0
  br i1 %is_zero, label %by_zero, label %nonzero
nonzero:
  ; srem is undefined for INT64_MIN % -1; anything % -1 is 0.
  %is_m1 = icmp eq i64 %vb, -1
  br i1 %is_m1, label %by_m1, label %int_mod
by_m1:
  ret %NykVal { i8 2, i64 0 }
int_mod:
  %r = srem i64 %va, %vb
  %r_nz = icmp ne i64 %r, 0
  %signs = xor i64 %r, %vb
  %signs_differ = icmp slt i64 %signs, 0
  %adjust = and i1 %r_nz, %signs_differ
  %rb = add i64 %r, %vb
  %fr = select i1 %adjust, i64 %rb, i64 %r
  %ri = call %NykVal @__nyk_mk_int(i64 %fr)
  ret %NykVal %ri
flt_path:
  %fa = call double @__nyk_as_f64(%NykVal %a)
  %fb = call double @__nyk_as_f64(%NykVal %b)
  %f_zero = fcmp oeq double %fb, 0.0
  br i1 %f_zero, label %by_zero, label %flt_mod
flt_mod:
  %fq0 = fdiv double %fa, %fb
  %ffq = call double @llvm.floor.f64(double %fq0)
  %prod = fmul double %ffq, %fb
  %fr0 = fsub double %fa, %prod
  %rf = call %NykVal @__nyk_mk_float(double %fr0)
  ret %NykVal %rf
by_zero:
  call void @__nyk_error(ptr @.err_mod_zero)
  unreachable
}

; base ^ exp for a non-negative integer exponent, by squaring (wrapping).
define internal i64 @__nyk_ipow(i64 %base, i64 %exp) {
entry:
  br label %loop
loop:
  %acc = phi i64 [ 1, %entry ], [ %acc2, %step ]
  %b = phi i64 [ %base, %entry ], [ %b2, %step ]
  %e = phi i64 [ %exp, %entry ], [ %e2, %step ]
  %done = icmp eq i64 %e, 0
  br i1 %done, label %exit, label %step
step:
  %bit = and i64 %e, 1
  %odd = icmp ne i64 %bit, 0
  %m = mul i64 %acc, %b
  %acc2 = select i1 %odd, i64 %m, i64 %acc
  %b2 = mul i64 %b, %b
  %e2 = lshr i64 %e, 1
  br label %loop
exit:
  ret i64 %acc
}

; An integer raised to a non-negative integer stays an integer; everything
; else is a float power.
define internal %NykVal @__nyk_pow(%NykVal %a, %NykVal %b) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %tb = extractvalue %NykVal %b, 0
  %ai = icmp eq i8 %ta, 2
  %bi = icmp eq i8 %tb, 2
  %both_int = and i1 %ai, %bi
  br i1 %both_int, label %int_path, label %flt_path
int_path:
  %va = extractvalue %NykVal %a, 1
  %vb = extractvalue %NykVal %b, 1
  %exp_ok = icmp sge i64 %vb, 0
  br i1 %exp_ok, label %int_pow, label %flt_path
int_pow:
  %p = call i64 @__nyk_ipow(i64 %va, i64 %vb)
  %ri = call %NykVal @__nyk_mk_int(i64 %p)
  ret %NykVal %ri
flt_path:
  %fa = call double @__nyk_as_f64(%NykVal %a)
  %fb = call double @__nyk_as_f64(%NykVal %b)
  %fp = call double @llvm.pow.f64(double %fa, double %fb)
  %rf = call %NykVal @__nyk_mk_float(double %fp)
  ret %NykVal %rf
}

define internal %NykVal @__nyk_bitand(%NykVal %a, %NykVal %b) alwaysinline {
  %va = call i64 @__nyk_as_i64(%NykVal %a)
  %vb = call i64 @__nyk_as_i64(%NykVal %b)
  %res = and i64 %va, %vb
  %ri = call %NykVal @__nyk_mk_int(i64 %res)
  ret %NykVal %ri
}

define internal %NykVal @__nyk_bitor(%NykVal %a, %NykVal %b) alwaysinline {
  %va = call i64 @__nyk_as_i64(%NykVal %a)
  %vb = call i64 @__nyk_as_i64(%NykVal %b)
  %res = or i64 %va, %vb
  %ri = call %NykVal @__nyk_mk_int(i64 %res)
  ret %NykVal %ri
}

define internal %NykVal @__nyk_bitxor(%NykVal %a, %NykVal %b) alwaysinline {
  %va = call i64 @__nyk_as_i64(%NykVal %a)
  %vb = call i64 @__nyk_as_i64(%NykVal %b)
  %res = xor i64 %va, %vb
  %ri = call %NykVal @__nyk_mk_int(i64 %res)
  ret %NykVal %ri
}

; LLVM shifts by >= 64 bits are poison. These count-checked forms give 0
; (or the sign for an arithmetic right shift) instead; a negative count, as
; an unsigned number, is >= 64 too.
define internal i64 @__nyk_shl_i64(i64 %a, i64 %n) alwaysinline {
  %big = icmp uge i64 %n, 64
  %m = and i64 %n, 63
  %s = shl i64 %a, %m
  %r = select i1 %big, i64 0, i64 %s
  ret i64 %r
}

define internal i64 @__nyk_ashr_i64(i64 %a, i64 %n) alwaysinline {
  %big = icmp uge i64 %n, 64
  %m = and i64 %n, 63
  %s = ashr i64 %a, %m
  %sign = ashr i64 %a, 63
  %r = select i1 %big, i64 %sign, i64 %s
  ret i64 %r
}

define internal i64 @__nyk_lshr_i64(i64 %a, i64 %n) alwaysinline {
  %big = icmp uge i64 %n, 64
  %m = and i64 %n, 63
  %s = lshr i64 %a, %m
  %r = select i1 %big, i64 0, i64 %s
  ret i64 %r
}

; `<<` and `>>` shift the other way for a negative count, as in the VM.
define internal %NykVal @__nyk_shl(%NykVal %a, %NykVal %b) alwaysinline {
  %va = call i64 @__nyk_as_i64(%NykVal %a)
  %vb = call i64 @__nyk_as_i64(%NykVal %b)
  %is_neg = icmp slt i64 %vb, 0
  %nb = sub i64 0, %vb
  %left = call i64 @__nyk_shl_i64(i64 %va, i64 %vb)
  %right = call i64 @__nyk_ashr_i64(i64 %va, i64 %nb)
  %res = select i1 %is_neg, i64 %right, i64 %left
  %ri = call %NykVal @__nyk_mk_int(i64 %res)
  ret %NykVal %ri
}

define internal %NykVal @__nyk_shr(%NykVal %a, %NykVal %b) alwaysinline {
  %va = call i64 @__nyk_as_i64(%NykVal %a)
  %vb = call i64 @__nyk_as_i64(%NykVal %b)
  %is_neg = icmp slt i64 %vb, 0
  %nb = sub i64 0, %vb
  %right = call i64 @__nyk_ashr_i64(i64 %va, i64 %vb)
  %left = call i64 @__nyk_shl_i64(i64 %va, i64 %nb)
  %res = select i1 %is_neg, i64 %left, i64 %right
  %ri = call %NykVal @__nyk_mk_int(i64 %res)
  ret %NykVal %ri
}

; `<<<` and `>>>` are logical shifts on the 64-bit word: any count outside
; 0..63, negative included, gives 0.
define internal %NykVal @__nyk_lshl(%NykVal %a, %NykVal %b) alwaysinline {
  %va = call i64 @__nyk_as_i64(%NykVal %a)
  %vb = call i64 @__nyk_as_i64(%NykVal %b)
  %res = call i64 @__nyk_shl_i64(i64 %va, i64 %vb)
  %ri = call %NykVal @__nyk_mk_int(i64 %res)
  ret %NykVal %ri
}

define internal %NykVal @__nyk_lshr(%NykVal %a, %NykVal %b) alwaysinline {
  %va = call i64 @__nyk_as_i64(%NykVal %a)
  %vb = call i64 @__nyk_as_i64(%NykVal %b)
  %res = call i64 @__nyk_lshr_i64(i64 %va, i64 %vb)
  %ri = call %NykVal @__nyk_mk_int(i64 %res)
  ret %NykVal %ri
}

define internal %NykVal @__nyk_neg(%NykVal %a) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %va = extractvalue %NykVal %a, 1
  switch i8 %ta, label %bad [
    i8 2, label %intp
    i8 3, label %flt
  ]
flt:
  %fa = bitcast i64 %va to double
  %fneg = fneg double %fa
  %rf = call %NykVal @__nyk_mk_float(double %fneg)
  ret %NykVal %rf
intp:
  %ineg = sub i64 0, %va
  %ri = call %NykVal @__nyk_mk_int(i64 %ineg)
  ret %NykVal %ri
bad:
  call void @__nyk_error(ptr @.err_unary_minus)
  unreachable
}

define internal %NykVal @__nyk_not(%NykVal %a) alwaysinline {
  %t = call i1 @__nyk_truthy(%NykVal %a)
  %n = xor i1 %t, true
  %rb = call %NykVal @__nyk_mk_bool(i1 %n)
  ret %NykVal %rb
}

define internal %NykVal @__nyk_bitnot(%NykVal %a) alwaysinline {
  %va = call i64 @__nyk_as_i64(%NykVal %a)
  %res = xor i64 %va, -1
  %ri = call %NykVal @__nyk_mk_int(i64 %res)
  ret %NykVal %ri
}

; Strings are NUL-terminated byte strings and compare by content (with libc
; `strcmp`, after a pointer check), so a string built at run time equals a
; literal with the same bytes. Tables and functions compare by identity.
define internal i1 @__nyk_str_eq(ptr %a, ptr %b) alwaysinline {
  %same = icmp eq ptr %a, %b
  br i1 %same, label %yes, label %cmp
cmp:
  %c = call i32 @strcmp(ptr %a, ptr %b)
  %eq = icmp eq i32 %c, 0
  ret i1 %eq
yes:
  ret i1 true
}

; An integer equals a float only when the float holds exactly that integer,
; as in the VM (so 2^53 + 1 ~= 2^53 as a float).
define internal i1 @__nyk_int_eq_float(i64 %i, double %f) alwaysinline {
  %ge_min = fcmp oge double %f, 0xC3E0000000000000
  %lt_max = fcmp olt double %f, 0x43E0000000000000
  %in_range = and i1 %ge_min, %lt_max
  %fi = call i64 @llvm.fptosi.sat.i64.f64(double %f)
  %back = sitofp i64 %fi to double
  %integral = fcmp oeq double %back, %f
  %same = icmp eq i64 %fi, %i
  %r0 = and i1 %in_range, %integral
  %r = and i1 %r0, %same
  ret i1 %r
}

define internal i1 @__nyk_eq_i1(%NykVal %a, %NykVal %b) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %tb = extractvalue %NykVal %b, 0
  %va = extractvalue %NykVal %a, 1
  %vb = extractvalue %NykVal %b, 1
  %same_tag = icmp eq i8 %ta, %tb
  br i1 %same_tag, label %same, label %diff
same:
  switch i8 %ta, label %bits [
    i8 3, label %flt
    i8 4, label %str
  ]
bits:
  %eq = icmp eq i64 %va, %vb
  ret i1 %eq
flt:
  ; fcmp oeq: NaN ~= NaN and 0.0 == -0.0.
  %fa = bitcast i64 %va to double
  %fb = bitcast i64 %vb to double
  %feq = fcmp oeq double %fa, %fb
  ret i1 %feq
str:
  %pa = inttoptr i64 %va to ptr
  %pb = inttoptr i64 %vb to ptr
  %seq = call i1 @__nyk_str_eq(ptr %pa, ptr %pb)
  ret i1 %seq
diff:
  %a_int = icmp eq i8 %ta, 2
  %b_flt = icmp eq i8 %tb, 3
  %int_flt = and i1 %a_int, %b_flt
  br i1 %int_flt, label %if, label %diff2
if:
  %fb2 = bitcast i64 %vb to double
  %ifeq = call i1 @__nyk_int_eq_float(i64 %va, double %fb2)
  ret i1 %ifeq
diff2:
  %a_flt = icmp eq i8 %ta, 3
  %b_int = icmp eq i8 %tb, 2
  %flt_int = and i1 %a_flt, %b_int
  br i1 %flt_int, label %fi, label %ne
fi:
  %fa2 = bitcast i64 %va to double
  %fieq = call i1 @__nyk_int_eq_float(i64 %vb, double %fa2)
  ret i1 %fieq
ne:
  ret i1 false
}

define internal %NykVal @__nyk_eq(%NykVal %a, %NykVal %b) alwaysinline {
  %eq = call i1 @__nyk_eq_i1(%NykVal %a, %NykVal %b)
  %rb = call %NykVal @__nyk_mk_bool(i1 %eq)
  ret %NykVal %rb
}

define internal %NykVal @__nyk_ne(%NykVal %a, %NykVal %b) alwaysinline {
  %eq = call i1 @__nyk_eq_i1(%NykVal %a, %NykVal %b)
  %ne = xor i1 %eq, true
  %rb = call %NykVal @__nyk_mk_bool(i1 %ne)
  ret %NykVal %rb
}

; Ordering: two integers compare as integers, two strings byte by byte
; (`strcmp`), other numbers as floats; any other pair is an error, as in the
; VM. `%strict` picks `<` (true) or `<=` (false).
define internal i1 @__nyk_order(%NykVal %a, %NykVal %b, i1 %strict) alwaysinline {
  %ta = extractvalue %NykVal %a, 0
  %tb = extractvalue %NykVal %b, 0
  %ai = icmp eq i8 %ta, 2
  %bi = icmp eq i8 %tb, 2
  %both_int = and i1 %ai, %bi
  br i1 %both_int, label %int_path, label %not_int
int_path:
  %va = extractvalue %NykVal %a, 1
  %vb = extractvalue %NykVal %b, 1
  %ilt = icmp slt i64 %va, %vb
  %ile = icmp sle i64 %va, %vb
  %icmp = select i1 %strict, i1 %ilt, i1 %ile
  ret i1 %icmp
not_int:
  %as = icmp eq i8 %ta, 4
  %bs = icmp eq i8 %tb, 4
  %both_str = and i1 %as, %bs
  br i1 %both_str, label %str_path, label %num_path
str_path:
  %ra = extractvalue %NykVal %a, 1
  %rb = extractvalue %NykVal %b, 1
  %pa = inttoptr i64 %ra to ptr
  %pb = inttoptr i64 %rb to ptr
  %c = call i32 @strcmp(ptr %pa, ptr %pb)
  %slt = icmp slt i32 %c, 0
  %sle = icmp sle i32 %c, 0
  %scmp = select i1 %strict, i1 %slt, i1 %sle
  ret i1 %scmp
num_path:
  ; __nyk_as_f64 raises "expected number" for anything but a number.
  %fa = call double @__nyk_as_f64(%NykVal %a)
  %fb = call double @__nyk_as_f64(%NykVal %b)
  %flt = fcmp olt double %fa, %fb
  %fle = fcmp ole double %fa, %fb
  %fcmp = select i1 %strict, i1 %flt, i1 %fle
  ret i1 %fcmp
}

define internal %NykVal @__nyk_lt(%NykVal %a, %NykVal %b) alwaysinline {
  %r = call i1 @__nyk_order(%NykVal %a, %NykVal %b, i1 true)
  %rb = call %NykVal @__nyk_mk_bool(i1 %r)
  ret %NykVal %rb
}

define internal %NykVal @__nyk_le(%NykVal %a, %NykVal %b) alwaysinline {
  %r = call i1 @__nyk_order(%NykVal %a, %NykVal %b, i1 false)
  %rb = call %NykVal @__nyk_mk_bool(i1 %r)
  ret %NykVal %rb
}

define internal %NykVal @__nyk_gt(%NykVal %a, %NykVal %b) alwaysinline {
  %res = call %NykVal @__nyk_lt(%NykVal %b, %NykVal %a)
  ret %NykVal %res
}

define internal %NykVal @__nyk_ge(%NykVal %a, %NykVal %b) alwaysinline {
  %res = call %NykVal @__nyk_le(%NykVal %b, %NykVal %a)
  ret %NykVal %res
}

define internal void @__nyk_print_one(%NykVal %v) {
  %tag = extractvalue %NykVal %v, 0
  %raw = extractvalue %NykVal %v, 1
  switch i8 %tag, label %p_nil [
    i8 1, label %p_bool
    i8 2, label %p_int
    i8 3, label %p_float
    i8 4, label %p_str
  ]
p_nil:
  call i32 (ptr, ...) @printf(ptr @.fmt_nil)
  ret void
p_bool:
  %b = icmp ne i64 %raw, 0
  %s = select i1 %b, ptr @.fmt_true, ptr @.fmt_false
  call i32 (ptr, ...) @printf(ptr @.fmt_str, ptr %s)
  ret void
p_int:
  call i32 (ptr, ...) @printf(ptr @.fmt_int, i64 %raw)
  ret void
p_float:
  %f = bitcast i64 %raw to double
  call i32 (ptr, ...) @printf(ptr @.fmt_float, double %f)
  ret void
p_str:
  %p = inttoptr i64 %raw to ptr
  call i32 (ptr, ...) @printf(ptr @.fmt_str, ptr %p)
  ret void
}

; `print(...)` of the `argc` values at `argv`, for a call whose number of
; arguments is not known at compile time.
define internal void @__nyk_print_list(ptr %argv, i32 %argc) {
entry:
  br label %loop
loop:
  %i = phi i32 [ 0, %entry ], [ %next, %show ]
  %more = icmp ult i32 %i, %argc
  br i1 %more, label %body, label %done
body:
  %first = icmp eq i32 %i, 0
  br i1 %first, label %show, label %sep
sep:
  call void @__nyk_print_sp()
  br label %show
show:
  %iz = zext i32 %i to i64
  %slot = getelementptr %NykVal, ptr %argv, i64 %iz
  %v = load %NykVal, ptr %slot
  call void @__nyk_print_one(%NykVal %v)
  %next = add i32 %i, 1
  br label %loop
done:
  call void @__nyk_print_nl()
  ret void
}

@__nyk_nilval = private constant %NykVal zeroinitializer

; Element `i` of the `n` values at `p`, nil past the end (`p` may be null
; when `n` is 0).
define internal %NykVal @__nyk_list_get(ptr %p, i32 %n, i32 %i) alwaysinline {
  %has = icmp ult i32 %i, %n
  %iz = zext i32 %i to i64
  %slot = getelementptr %NykVal, ptr %p, i64 %iz
  %src = select i1 %has, ptr %slot, ptr @__nyk_nilval
  %v = load %NykVal, ptr %src
  ret %NykVal %v
}

; Appends the `n` values at `p` to the table `t`, as a table constructor
; does with a trailing call or `...`.
define internal void @__nyk_append_list(%NykVal %t, ptr %p, i32 %n) {
entry:
  %tt = extractvalue %NykVal %t, 0
  %tr = extractvalue %NykVal %t, 1
  br label %loop
loop:
  %i = phi i32 [ 0, %entry ], [ %next, %body ]
  %more = icmp ult i32 %i, %n
  br i1 %more, label %body, label %done
body:
  %iz = zext i32 %i to i64
  %slot = getelementptr %NykVal, ptr %p, i64 %iz
  %v = load %NykVal, ptr %slot
  %vt = extractvalue %NykVal %v, 0
  %vr = extractvalue %NykVal %v, 1
  call void @__nyk_table_append(i8 zeroext %tt, i64 %tr, i8 zeroext %vt, i64 %vr)
  %next = add i32 %i, 1
  br label %loop
done:
  ret void
}

define internal void @__nyk_print_nl() alwaysinline {
  call i32 (ptr, ...) @printf(ptr @.fmt_nl)
  ret void
}

define internal void @__nyk_print_sp() alwaysinline {
  call i32 (ptr, ...) @printf(ptr @.fmt_sp)
  ret void
}

"#,
    );
}
