//! The runtime that executables built by the Neyuki LLVM backend
//! (`neyuki build --native`, or `neyuki emit-llvm --runtime=extern`) link
//! against.
//!
//! The generated code keeps arithmetic, comparisons and `print` inline and
//! calls the functions here for everything that needs the heap or global
//! state. Globals, strings built by `..`, `#` and the `select` builtin work;
//! tables, module imports and varargs are stubs for now.
//!
//! # ABI
//!
//! A value is `%NykVal = type { i8, i64 }` in the IR and [`NykVal`] here:
//! a tag and 64 raw bits (an integer, the bits of a float, or a pointer).
//! Passing such a 16-byte struct by value is not portable: Windows x64
//! passes and returns it through a hidden pointer, where LLVM would split
//! it into registers. So at this boundary, and only here:
//!
//! - a value argument is two scalars, `tag: u8` (`i8 zeroext`) and
//!   `raw: i64`;
//! - a value result is written through an out-pointer, the first
//!   parameter, and the function returns nothing;
//! - argument lists are pointers to arrays of [`NykVal`] in memory, whose
//!   layout matches the IR's under the module's `target datalayout`.
//!
//! `src/compiler/llvm/runtime.rs` in the `neyuki` crate declares the same
//! signatures.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

pub const TAG_NIL: u8 = 0;
pub const TAG_BOOL: u8 = 1;
pub const TAG_INT: u8 = 2;
pub const TAG_FLOAT: u8 = 3;
pub const TAG_STRING: u8 = 4;
pub const TAG_TABLE: u8 = 5;
pub const TAG_FN: u8 = 6;
pub const TAG_BUILTIN_PRINT: u8 = 7;
/// A function of this runtime; `raw` is its index in [`NATIVES`].
pub const TAG_NATIVE: u8 = 8;

/// A Neyuki value, laid out like `%NykVal = type { i8, i64 }`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NykVal {
    pub tag: u8,
    pub raw: i64,
}

impl NykVal {
    pub const NIL: NykVal = NykVal {
        tag: TAG_NIL,
        raw: 0,
    };

    fn new(tag: u8, raw: i64) -> Self {
        NykVal { tag, raw }
    }

    fn int(i: i64) -> Self {
        NykVal::new(TAG_INT, i)
    }

    /// A string value owning a copy of `bytes`. Strings are NUL-terminated
    /// for the generated code and are never freed (there is no collector
    /// yet).
    fn string(bytes: &[u8]) -> Self {
        let mut buf = Vec::with_capacity(bytes.len() + 1);
        buf.extend_from_slice(bytes);
        buf.push(0);
        let ptr = Box::leak(buf.into_boxed_slice()).as_ptr();
        NykVal::new(TAG_STRING, ptr as usize as i64)
    }

    /// The bytes of a string value, without the NUL.
    ///
    /// # Safety
    ///
    /// `self` must be a string value whose `raw` points to a NUL-terminated
    /// string that lives for the rest of the program, as all of them do.
    unsafe fn str_bytes(&self) -> &'static [u8] {
        // SAFETY: guaranteed by the caller.
        unsafe { CStr::from_ptr(self.raw as usize as *const c_char) }.to_bytes()
    }

    fn type_name(&self) -> &'static str {
        match self.tag {
            TAG_NIL => "nil",
            TAG_BOOL => "boolean",
            TAG_INT | TAG_FLOAT => "number",
            TAG_STRING => "string",
            TAG_TABLE => "table",
            _ => "function",
        }
    }
}

unsafe extern "C" {
    fn fflush(stream: *mut c_void) -> c_int;
}

/// Raises a runtime error: prints `runtime error: <msg>` to stderr and exits
/// with status 1, as the VM does.
///
/// # Safety
///
/// `msg` must point to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_error(msg: *const c_char) -> ! {
    // SAFETY: guaranteed by the caller.
    let text = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
    fail(&text)
}

fn fail(msg: &str) -> ! {
    // `print` in the generated code writes through C stdio; flush it so the
    // output comes before the error, as it would on the VM.
    // SAFETY: `fflush(NULL)` flushes every open output stream.
    unsafe { fflush(std::ptr::null_mut()) };
    eprintln!("runtime error: {}", msg);
    std::process::exit(1)
}

/// Writes `val` to the out-pointer of a function that returns a value.
///
/// # Safety
///
/// `out` must be valid for writing a [`NykVal`].
unsafe fn put(out: *mut NykVal, val: NykVal) {
    // SAFETY: guaranteed by the caller.
    unsafe { out.write(val) }
}

/// The arguments of a call as a slice.
///
/// # Safety
///
/// `argv` must point to `argc` values, or `argc` must be 0.
unsafe fn args<'a>(argc: u32, argv: *const NykVal) -> &'a [NykVal] {
    if argc == 0 || argv.is_null() {
        return &[];
    }
    // SAFETY: guaranteed by the caller.
    unsafe { std::slice::from_raw_parts(argv, argc as usize) }
}

// ---- globals ---------------------------------------------------------------

fn globals() -> &'static Mutex<HashMap<Vec<u8>, NykVal>> {
    static GLOBALS: OnceLock<Mutex<HashMap<Vec<u8>, NykVal>>> = OnceLock::new();
    GLOBALS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reads the global `name`: one the script assigned, else a builtin of
/// this runtime, else nil.
///
/// # Safety
///
/// `out` must be valid for writing and `name` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_global_get(out: *mut NykVal, name: *const c_char) {
    // SAFETY: guaranteed by the caller.
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();
    let assigned = globals()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(name)
        .copied();
    let val = assigned
        .or_else(|| {
            NATIVES
                .iter()
                .position(|(native, _)| native.as_bytes() == name)
                .map(|idx| NykVal::new(TAG_NATIVE, idx as i64))
        })
        .unwrap_or(NykVal::NIL);
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, val) }
}

/// Assigns the global `name`.
///
/// # Safety
///
/// `name` must be a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_global_set(name: *const c_char, tag: u8, raw: i64) {
    // SAFETY: guaranteed by the caller.
    let name = unsafe { CStr::from_ptr(name) }.to_bytes().to_vec();
    globals()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(name, NykVal::new(tag, raw));
}

// ---- upvalues ----------------------------------------------------------------

const UPVAL_SLOTS: usize = 256;

/// Upvalues live in one shared array for now, as in the built-in fallback.
static UPVALS: Mutex<[NykVal; UPVAL_SLOTS]> = Mutex::new([NykVal::NIL; UPVAL_SLOTS]);

/// # Safety
///
/// `out` must be valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_upval_get(out: *mut NykVal, idx: u32) {
    let slots = UPVALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let val = slots.get(idx as usize).copied().unwrap_or(NykVal::NIL);
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, val) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __nyk_upval_set(idx: u32, tag: u8, raw: i64) {
    let mut slots = UPVALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(slot) = slots.get_mut(idx as usize) {
        *slot = NykVal::new(tag, raw);
    }
}

// ---- modules and tables (stubs) ------------------------------------------------

/// # Safety
///
/// `out` must be valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_import_get(
    out: *mut NykVal,
    _module: *const c_char,
    _field: *const c_char,
) {
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::NIL) }
}

/// A new (stub) table. Each one gets its own identity, so two tables are
/// not equal.
///
/// # Safety
///
/// `out` must be valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_table_new(out: *mut NykVal, _array_cap: u32, _fields_cap: u32) {
    static NEXT_ID: AtomicI64 = AtomicI64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::new(TAG_TABLE, id)) }
}

/// # Safety
///
/// `out` must be valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_table_get(
    out: *mut NykVal,
    _t_tag: u8,
    _t_raw: i64,
    _k_tag: u8,
    _k_raw: i64,
) {
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::NIL) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __nyk_table_set(
    _t_tag: u8,
    _t_raw: i64,
    _k_tag: u8,
    _k_raw: i64,
    _v_tag: u8,
    _v_raw: i64,
) {
}

#[unsafe(no_mangle)]
pub extern "C" fn __nyk_table_append(_t_tag: u8, _t_raw: i64, _v_tag: u8, _v_raw: i64) {}

// ---- strings -----------------------------------------------------------------

/// A value as text for `..`, numbers formatted like the generated `print`
/// does (`%lld` and `%g`).
fn to_text(val: NykVal) -> Vec<u8> {
    match val.tag {
        TAG_NIL => b"nil".to_vec(),
        TAG_BOOL => {
            if val.raw != 0 {
                b"true".to_vec()
            } else {
                b"false".to_vec()
            }
        }
        TAG_INT => val.raw.to_string().into_bytes(),
        TAG_FLOAT => format_g(f64::from_bits(val.raw as u64)),
        // SAFETY: the generated code only makes string values from string
        // constants and from strings this runtime allocated.
        TAG_STRING => unsafe { val.str_bytes() }.to_vec(),
        TAG_TABLE => b"table".to_vec(),
        _ => b"function".to_vec(),
    }
}

/// `f` as C's `printf("%g")` writes it: 6 significant digits, trailing
/// zeros dropped, and exponent notation (`1e+06`) when the exponent is
/// below -4 or at least 6. Written out here rather than calling the C
/// library's `snprintf`, which the Windows UCRT only has as an inline
/// function, so there is no symbol to link.
fn format_g(f: f64) -> Vec<u8> {
    const PRECISION: i32 = 6;
    if f.is_nan() {
        let sign = if f.is_sign_negative() { "-" } else { "" };
        return format!("{}nan", sign).into_bytes();
    }
    if f.is_infinite() {
        return if f < 0.0 {
            b"-inf".to_vec()
        } else {
            b"inf".to_vec()
        };
    }
    if f == 0.0 {
        return if f.is_sign_negative() {
            b"-0".to_vec()
        } else {
            b"0".to_vec()
        };
    }
    // The exponent of the value rounded to PRECISION digits decides the form.
    let sci = format!("{:.*e}", (PRECISION - 1) as usize, f);
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let text = if !(-4..PRECISION).contains(&exp) {
        format!(
            "{}e{}{:02}",
            trim_fraction(mantissa),
            if exp < 0 { '-' } else { '+' },
            exp.unsigned_abs()
        )
    } else {
        let decimals = (PRECISION - 1 - exp).max(0) as usize;
        trim_fraction(&format!("{:.*}", decimals, f)).to_string()
    };
    text.into_bytes()
}

/// `digits` without trailing zeros after the decimal point (nor the point).
fn trim_fraction(digits: &str) -> &str {
    if digits.contains('.') {
        digits.trim_end_matches('0').trim_end_matches('.')
    } else {
        digits
    }
}

/// `a .. b`.
///
/// # Safety
///
/// `out` must be valid for writing, and string operands must be strings of
/// the program.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_concat(
    out: *mut NykVal,
    a_tag: u8,
    a_raw: i64,
    b_tag: u8,
    b_raw: i64,
) {
    let mut text = to_text(NykVal::new(a_tag, a_raw));
    text.extend_from_slice(&to_text(NykVal::new(b_tag, b_raw)));
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::string(&text)) }
}

/// `#v`: the byte length of a string; 0 for a (stub) table.
///
/// # Safety
///
/// `out` must be valid for writing, and a string operand a string of the
/// program.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_len(out: *mut NykVal, tag: u8, raw: i64) {
    let val = NykVal::new(tag, raw);
    let len = match tag {
        // SAFETY: guaranteed by the caller.
        TAG_STRING => unsafe { val.str_bytes() }.len() as i64,
        TAG_TABLE => 0,
        _ => fail("len expects string, table or buffer"),
    };
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::int(len)) }
}

// ---- functions -----------------------------------------------------------------

/// A function value for `fn_ptr`. Captured variables are not stored yet;
/// they go through the shared upvalue slots.
///
/// # Safety
///
/// `out` must be valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_closure(out: *mut NykVal, fn_ptr: *const c_void, _ncaps: u32) {
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::new(TAG_FN, fn_ptr as usize as i64)) }
}

/// A builtin of this runtime: its arguments in, its first result out.
type Native = fn(&[NykVal]) -> NykVal;

/// The builtins this runtime provides, as `(global name, function)`.
static NATIVES: &[(&str, Native)] = &[("select", native_select)];

/// `select(n, ...)`: the `n`th of the remaining arguments (counting from
/// the end when negative), or `select("#", ...)`: how many there are. Only
/// the first result is returned, like every call in this backend.
fn native_select(args: &[NykVal]) -> NykVal {
    let Some((selector, rest)) = args.split_first() else {
        fail("select expects '#' or an index within range");
    };
    // SAFETY: string values come from the program.
    if selector.tag == TAG_STRING && unsafe { selector.str_bytes() } == b"#" {
        return NykVal::int(rest.len() as i64);
    }
    let n = match selector.tag {
        TAG_INT => selector.raw,
        TAG_FLOAT => f64::from_bits(selector.raw as u64) as i64,
        _ => fail("select expects '#' or an index within range"),
    };
    let len = rest.len() as i64;
    let idx = if n > 0 {
        n - 1
    } else if n < 0 && -n <= len {
        len + n
    } else {
        fail("select expects '#' or an index within range");
    };
    rest.get(idx as usize).copied().unwrap_or(NykVal::NIL)
}

/// Calls a value that is not a function of the generated code: a builtin
/// of this runtime. Anything else is an error.
///
/// # Safety
///
/// `out` must be valid for writing and `argv` must point to `argc` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_call_dyn(
    out: *mut NykVal,
    tag: u8,
    raw: i64,
    argc: u32,
    argv: *const NykVal,
) {
    let callee = NykVal::new(tag, raw);
    let native = match tag {
        TAG_NATIVE => usize::try_from(raw).ok().and_then(|idx| NATIVES.get(idx)),
        _ => None,
    };
    let Some((_, native)) = native else {
        fail(&format!(
            "attempted to call non-function ({})",
            callee.type_name()
        ));
    };
    // SAFETY: guaranteed by the caller.
    let result = native(unsafe { args(argc, argv) });
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, result) }
}

/// `...`, which is always empty for now.
///
/// # Safety
///
/// `out` must be valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_vararg(out: *mut NykVal) {
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::NIL) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: &[NykVal]) -> NykVal {
        let cname = std::ffi::CString::new(name).unwrap();
        let mut callee = NykVal::NIL;
        let mut out = NykVal::NIL;
        // SAFETY: valid out-pointers, a NUL-terminated name and `args.len()`
        // values at `args`.
        unsafe {
            __nyk_global_get(&mut callee, cname.as_ptr());
            __nyk_call_dyn(
                &mut out,
                callee.tag,
                callee.raw,
                args.len() as u32,
                args.as_ptr(),
            );
        }
        out
    }

    #[test]
    fn format_g_matches_printf() {
        let cases: &[(f64, &str)] = &[
            (1.5, "1.5"),
            (2.0, "2"),
            (-0.5, "-0.5"),
            (1.0 / 3.0, "0.333333"),
            (100000.0, "100000"),
            (123456.7, "123457"),
            (999999.5, "1e+06"),
            (1e6, "1e+06"),
            (1234567.0, "1.23457e+06"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-10, "1.5e-10"),
            (1e300, "1e+300"),
            (-0.0, "-0"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
        ];
        for &(f, want) in cases {
            assert_eq!(String::from_utf8(format_g(f)).unwrap(), want, "{}", f);
        }
    }

    #[test]
    fn nyk_val_matches_the_ir_layout() {
        // `{ i8, i64 }`: the tag at offset 0, the payload 8-aligned.
        assert_eq!(std::mem::offset_of!(NykVal, tag), 0);
        assert_eq!(
            std::mem::offset_of!(NykVal, raw),
            std::mem::align_of::<i64>()
        );
    }

    #[test]
    fn select_picks_an_argument() {
        let a = [
            NykVal::int(3),
            NykVal::int(10),
            NykVal::int(20),
            NykVal::int(30),
        ];
        assert_eq!(call("select", &a), NykVal::int(30));
        let b = [NykVal::int(-1), NykVal::int(10), NykVal::int(20)];
        assert_eq!(call("select", &b), NykVal::int(20));
        let c = [NykVal::string(b"#"), NykVal::NIL, NykVal::NIL];
        assert_eq!(call("select", &c), NykVal::int(2));
        let d = [NykVal::int(5), NykVal::int(1)];
        assert_eq!(call("select", &d), NykVal::NIL);
    }

    #[test]
    fn globals_round_trip_and_concat_builds_strings() {
        let name = c"answer";
        let mut out = NykVal::NIL;
        // SAFETY: valid out-pointers and NUL-terminated strings.
        unsafe {
            __nyk_global_set(name.as_ptr(), TAG_INT, 42);
            __nyk_global_get(&mut out, name.as_ptr());
        }
        assert_eq!(out, NykVal::int(42));

        let x = NykVal::string(b"x");
        // SAFETY: as above; `x` is a string this runtime allocated.
        let s = unsafe {
            __nyk_concat(&mut out, x.tag, x.raw, TAG_FLOAT, 1.5f64.to_bits() as i64);
            out.str_bytes()
        };
        assert_eq!(s, b"x1.5");
    }
}
