//! The runtime that executables built by the Neyuki LLVM backend
//! (`neyuki build --native`, or `neyuki emit-llvm --runtime=extern`) link
//! against.
//!
//! The generated code keeps arithmetic, comparisons and `print` inline and
//! calls the functions here for everything that needs the heap or global
//! state. Globals, closures, tables, strings built by `..`, `#` and the
//! `select` builtin work; module imports and varargs are stubs for now.
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
//! # Closures
//!
//! A function value (tag 6) points to a closure object: an array of
//! pointers holding the function's code and then one cell per captured
//! variable. A cell is a heap [`NykVal`], shared by the closures that
//! capture the variable and the function that declared it. The generated
//! code calls through word 0 and reads and writes the cells of its own
//! locals; closures and cells, like strings, are never freed yet.
//!
//! # Tables
//!
//! A table (tag 5) points to a leaked [`Table`]: a growable array part for
//! keys `1..=n` and a hash map for every other key, split and trimmed like
//! the VM's so `#t`, nil keys, NaN keys and `t[1.0]` behave the same. Its
//! address is its identity, so `==` stays pointer equality, and its
//! elements are plain [`NykVal`]s, so it holds strings, functions and other
//! tables alike. Tables are never freed yet, like strings and closures.
//! Not supported: metatables, frozen tables, and indexing a string (reads
//! nil).
//!
//! `src/compiler/llvm/runtime.rs` in the `neyuki` crate declares the same
//! signatures.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
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

// ---- cells and upvalues -------------------------------------------------------

/// A new cell holding `(tag, raw)`.
#[unsafe(no_mangle)]
pub extern "C" fn __nyk_cell_new(tag: u8, raw: i64) -> *mut NykVal {
    Box::into_raw(Box::new(NykVal::new(tag, raw)))
}

/// The cell of upvalue `idx` of the closure object `env`.
///
/// # Safety
///
/// `env` must be a closure object with more than `idx` cells.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_upval_cell(env: *const *mut NykVal, idx: u32) -> *mut NykVal {
    // SAFETY: word 0 is the code, and the caller guarantees cell `idx`.
    unsafe { *env.add(idx as usize + 1) }
}

/// Reads upvalue `idx` of the closure `env`.
///
/// # Safety
///
/// `out` must be valid for writing, and `env` a closure object with more
/// than `idx` cells.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_upval_get(out: *mut NykVal, env: *const *mut NykVal, idx: u32) {
    // SAFETY: guaranteed by the caller; cells stay valid forever.
    unsafe { put(out, *__nyk_upval_cell(env, idx)) }
}

/// Assigns upvalue `idx` of the closure `env`.
///
/// # Safety
///
/// `env` must be a closure object with more than `idx` cells.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_upval_set(env: *const *mut NykVal, idx: u32, tag: u8, raw: i64) {
    // SAFETY: guaranteed by the caller; cells stay valid forever.
    unsafe { *__nyk_upval_cell(env, idx) = NykVal::new(tag, raw) }
}

// ---- modules (stub) --------------------------------------------------------------

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

// ---- tables ------------------------------------------------------------------------

/// A key of the hash part. Strings compare by content (they are leaked, so
/// the slice lives forever), numbers by value after [`normalize_key`], and
/// everything else by identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Bool(bool),
    Int(i64),
    /// The bits of a float that is not an integer.
    Float(u64),
    Str(&'static [u8]),
    /// A table, function or builtin: its tag and its pointer or index.
    Ref(u8, i64),
}

impl Key {
    /// The key of a value that [`normalize_key`] accepted.
    fn of(val: NykVal) -> Key {
        match val.tag {
            TAG_BOOL => Key::Bool(val.raw != 0),
            TAG_INT => Key::Int(val.raw),
            TAG_FLOAT => Key::Float(val.raw as u64),
            // SAFETY: string values come from the program.
            TAG_STRING => Key::Str(unsafe { val.str_bytes() }),
            tag => Key::Ref(tag, val.raw),
        }
    }
}

/// The key `key` stands for, or why it is not one. As in the VM, a float
/// with an integer value is the same key as that integer (`t[1.0]` is
/// `t[1]`, and `-0.0` is `0`), and nil and NaN are not keys.
fn normalize_key(key: NykVal) -> Result<NykVal, &'static str> {
    match key.tag {
        TAG_NIL => Err("table index is nil"),
        TAG_FLOAT => {
            let f = f64::from_bits(key.raw as u64);
            if f.is_nan() {
                Err("table index is NaN")
            } else if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                Ok(NykVal::int(f as i64))
            } else {
                // `0.0` and `-0.0` were integers above, so the bits are unique.
                Ok(key)
            }
        }
        _ => Ok(key),
    }
}

/// A table, laid out like the VM's: an array part holding keys `1..=len`
/// and a hash part for every other key. `#t` is the length of the array
/// part, so it matches the VM, `nil` holes included.
///
/// A table lives in a leaked allocation and its [`NykVal`] (tag
/// [`TAG_TABLE`]) holds the address, which never changes: two values are
/// the same table exactly when their `raw` is equal. The table can hold any
/// [`NykVal`], including tables and functions. Nothing is freed yet. Generated
/// code is single-threaded, so there is no locking.
#[derive(Default)]
struct Table {
    array: Vec<NykVal>,
    /// `(key as the script gave it, value)`, never holding a nil value.
    hash: HashMap<Key, (NykVal, NykVal)>,
}

impl Table {
    fn get(&self, key: NykVal) -> NykVal {
        let Ok(key) = normalize_key(key) else {
            return NykVal::NIL;
        };
        if key.tag == TAG_INT
            && let Some(slot) = array_slot(key.raw, self.array.len())
        {
            return self.array[slot];
        }
        self.hash
            .get(&Key::of(key))
            .map_or(NykVal::NIL, |&(_, val)| val)
    }

    /// Assigns `t[key] = val`; assigning nil removes the entry.
    fn set(&mut self, key: NykVal, val: NykVal) -> Result<(), &'static str> {
        let key = normalize_key(key)?;
        if key.tag == TAG_INT && key.raw >= 1 {
            let len = self.array.len();
            if let Some(slot) = array_slot(key.raw, len) {
                self.array[slot] = val;
                if slot + 1 == len {
                    self.trim();
                }
                return Ok(());
            }
            if key.raw as u64 == len as u64 + 1 {
                if val.tag == TAG_NIL {
                    self.hash.remove(&Key::of(key));
                } else {
                    self.array.push(val);
                    self.absorb();
                }
                return Ok(());
            }
        }
        if val.tag == TAG_NIL {
            self.hash.remove(&Key::of(key));
        } else {
            self.hash.insert(Key::of(key), (key, val));
        }
        Ok(())
    }

    /// Drops trailing nils off the array part, so `#t` shrinks with
    /// `t[#t] = nil`.
    fn trim(&mut self) {
        while self.array.last().is_some_and(|v| v.tag == TAG_NIL) {
            self.array.pop();
        }
    }

    /// Moves the integer keys that now continue the array part into it.
    fn absorb(&mut self) {
        while !self.hash.is_empty() {
            let next = Key::Int(self.array.len() as i64 + 1);
            match self.hash.remove(&next) {
                Some((_, val)) => self.array.push(val),
                None => break,
            }
        }
    }
}

/// The index in an array part of `len` elements of key `key`, if it has one.
fn array_slot(key: i64, len: usize) -> Option<usize> {
    (key >= 1 && (key as u64) <= len as u64).then(|| key as usize - 1)
}

/// The table a value is, or the VM's error for something else: reading
/// (`write` false) or setting a field of it.
///
/// # Safety
///
/// A table value must point to a table of this runtime, as every one the
/// generated code holds does, and the result must not outlive the next
/// call into the runtime that touches the same table.
unsafe fn table_mut<'a>(val: NykVal, write: bool) -> Result<&'a mut Table, String> {
    if val.tag == TAG_TABLE {
        // SAFETY: guaranteed by the caller.
        Ok(unsafe { &mut *(val.raw as usize as *mut Table) })
    } else if write {
        Err(format!(
            "attempted to set a field on a {} value",
            val.type_name()
        ))
    } else {
        Err(format!("attempted to index a {} value", val.type_name()))
    }
}

/// A new, empty table with room for `array_cap` array elements and
/// `fields_cap` other keys.
///
/// # Safety
///
/// `out` must be valid for writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_table_new(out: *mut NykVal, array_cap: u32, fields_cap: u32) {
    let table = Box::new(Table {
        array: Vec::with_capacity(array_cap as usize),
        hash: HashMap::with_capacity(fields_cap as usize),
    });
    let ptr = Box::into_raw(table);
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::new(TAG_TABLE, ptr as usize as i64)) }
}

/// `t[k]`. A missing key, a nil key and a NaN key read as nil; indexing
/// anything but a table is an error, except a string, which reads nil
/// (the VM looks it up in the string library, which this runtime lacks).
///
/// # Safety
///
/// `out` must be valid for writing, and a table operand a table of this
/// runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_table_get(
    out: *mut NykVal,
    t_tag: u8,
    t_raw: i64,
    k_tag: u8,
    k_raw: i64,
) {
    let base = NykVal::new(t_tag, t_raw);
    let val = if base.tag == TAG_STRING {
        NykVal::NIL
    } else {
        // SAFETY: guaranteed by the caller.
        match unsafe { table_mut(base, false) } {
            Ok(table) => table.get(NykVal::new(k_tag, k_raw)),
            Err(msg) => fail(&msg),
        }
    };
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, val) }
}

/// `t[k] = v`; nil removes the entry. A nil or NaN key is an error.
///
/// # Safety
///
/// A table operand must be a table of this runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_table_set(
    t_tag: u8,
    t_raw: i64,
    k_tag: u8,
    k_raw: i64,
    v_tag: u8,
    v_raw: i64,
) {
    // SAFETY: guaranteed by the caller.
    let result = unsafe { table_mut(NykVal::new(t_tag, t_raw), true) }.and_then(|table| {
        table
            .set(NykVal::new(k_tag, k_raw), NykVal::new(v_tag, v_raw))
            .map_err(String::from)
    });
    if let Err(msg) = result {
        fail(&msg);
    }
}

/// Pushes `v` on the array part of `t`, as table constructors do for their
/// positional items (a nil is kept, so `#{1, nil}` is 2, as in the VM).
///
/// # Safety
///
/// A table operand must be a table of this runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_table_append(t_tag: u8, t_raw: i64, v_tag: u8, v_raw: i64) {
    if t_tag != TAG_TABLE {
        fail("cannot append to non-table");
    }
    // SAFETY: guaranteed by the caller.
    match unsafe { table_mut(NykVal::new(t_tag, t_raw), true) } {
        Ok(table) => table.array.push(NykVal::new(v_tag, v_raw)),
        Err(msg) => fail(&msg),
    }
}

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

/// `#v`: the byte length of a string, or the length of the array part of a
/// table.
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
        // SAFETY: guaranteed by the caller.
        TAG_TABLE => unsafe { table_mut(val, false) }.map_or(0, |t| t.array.len() as i64),
        _ => fail("len expects string, table or buffer"),
    };
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::int(len)) }
}

// ---- functions -----------------------------------------------------------------

/// A function value: a new closure object for `fn_ptr` sharing the `ncaps`
/// cells at `caps`.
///
/// # Safety
///
/// `out` must be valid for writing and `caps` must point to `ncaps` cells.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __nyk_closure(
    out: *mut NykVal,
    fn_ptr: *mut c_void,
    ncaps: u32,
    caps: *const *mut c_void,
) {
    let mut object = Vec::with_capacity(ncaps as usize + 1);
    object.push(fn_ptr);
    if ncaps > 0 {
        // SAFETY: guaranteed by the caller.
        object.extend_from_slice(unsafe { std::slice::from_raw_parts(caps, ncaps as usize) });
    }
    let ptr = Box::leak(object.into_boxed_slice()).as_ptr();
    // SAFETY: guaranteed by the caller.
    unsafe { put(out, NykVal::new(TAG_FN, ptr as usize as i64)) }
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

#[cfg(test)]
mod closure_tests {
    use super::*;

    #[test]
    fn closures_share_their_cells() {
        let cell = __nyk_cell_new(TAG_INT, 1);
        let caps = [cell.cast::<c_void>()];
        let mut a = NykVal::NIL;
        let mut b = NykVal::NIL;
        let mut got = NykVal::NIL;
        // SAFETY: valid out-pointers, one cell at `caps`, and the closure
        // objects just made have one cell each.
        unsafe {
            __nyk_closure(&mut a, std::ptr::null_mut(), 1, caps.as_ptr());
            __nyk_closure(&mut b, std::ptr::null_mut(), 1, caps.as_ptr());
            let env_a = a.raw as usize as *const *mut NykVal;
            let env_b = b.raw as usize as *const *mut NykVal;
            __nyk_upval_set(env_a, 0, TAG_INT, 7);
            __nyk_upval_get(&mut got, env_b, 0);
            assert_eq!(__nyk_upval_cell(env_b, 0), cell);
        }
        assert_ne!(a, b);
        assert_eq!(got, NykVal::int(7));
    }
}

#[cfg(test)]
mod table_tests {
    use super::*;

    fn new_table() -> NykVal {
        let mut t = NykVal::NIL;
        // SAFETY: a valid out-pointer.
        unsafe { __nyk_table_new(&mut t, 0, 0) };
        t
    }

    fn get(t: NykVal, k: NykVal) -> NykVal {
        let mut out = NykVal::NIL;
        // SAFETY: a valid out-pointer and a table of this runtime.
        unsafe { __nyk_table_get(&mut out, t.tag, t.raw, k.tag, k.raw) };
        out
    }

    fn set(t: NykVal, k: NykVal, v: NykVal) {
        // SAFETY: a table of this runtime.
        unsafe { __nyk_table_set(t.tag, t.raw, k.tag, k.raw, v.tag, v.raw) }
    }

    fn append(t: NykVal, v: NykVal) {
        // SAFETY: a table of this runtime.
        unsafe { __nyk_table_append(t.tag, t.raw, v.tag, v.raw) }
    }

    fn len(t: NykVal) -> i64 {
        let mut out = NykVal::NIL;
        // SAFETY: a valid out-pointer and a table of this runtime.
        unsafe { __nyk_len(&mut out, t.tag, t.raw) };
        out.raw
    }

    fn float(f: f64) -> NykVal {
        NykVal::new(TAG_FLOAT, f.to_bits() as i64)
    }

    #[test]
    fn tables_have_their_own_identity() {
        let (a, b) = (new_table(), new_table());
        assert_eq!(a.tag, TAG_TABLE);
        assert_ne!(a, b);
        assert_eq!(a, a);
    }

    #[test]
    fn array_and_hash_parts_round_trip() {
        let t = new_table();
        append(t, NykVal::int(10));
        append(t, NykVal::int(20));
        set(t, NykVal::string(b"name"), NykVal::int(7));
        set(t, NykVal::int(100), NykVal::int(8));
        assert_eq!(len(t), 2);
        assert_eq!(get(t, NykVal::int(2)), NykVal::int(20));
        assert_eq!(get(t, NykVal::string(b"name")), NykVal::int(7));
        assert_eq!(get(t, NykVal::int(100)), NykVal::int(8));
        assert_eq!(get(t, NykVal::int(3)), NykVal::NIL);
        assert_eq!(get(t, NykVal::string(b"missing")), NykVal::NIL);
        // Overwrite, then remove.
        set(t, NykVal::string(b"name"), NykVal::int(9));
        assert_eq!(get(t, NykVal::string(b"name")), NykVal::int(9));
        set(t, NykVal::string(b"name"), NykVal::NIL);
        assert_eq!(get(t, NykVal::string(b"name")), NykVal::NIL);
    }

    #[test]
    fn length_follows_the_array_part_like_the_vm() {
        let t = new_table();
        set(t, NykVal::int(1), NykVal::int(1));
        set(t, NykVal::int(3), NykVal::int(3));
        assert_eq!(len(t), 1);
        // Key 2 closes the gap: 3 moves from the hash into the array.
        set(t, NykVal::int(2), NykVal::int(2));
        assert_eq!(len(t), 3);
        set(t, NykVal::int(3), NykVal::NIL);
        assert_eq!(len(t), 2);
        // A hole in the middle keeps the length.
        set(t, NykVal::int(1), NykVal::NIL);
        assert_eq!(len(t), 2);
        set(t, NykVal::int(2), NykVal::NIL);
        assert_eq!(len(t), 0);
        // Constructors keep nils.
        let u = new_table();
        append(u, NykVal::int(1));
        append(u, NykVal::NIL);
        assert_eq!(len(u), 2);
    }

    #[test]
    fn floats_with_integer_values_are_integer_keys() {
        let t = new_table();
        set(t, float(1.0), NykVal::int(5));
        assert_eq!(get(t, NykVal::int(1)), NykVal::int(5));
        assert_eq!(len(t), 1);
        set(t, float(2.5), NykVal::int(6));
        assert_eq!(get(t, float(2.5)), NykVal::int(6));
        set(t, float(-0.0), NykVal::int(7));
        assert_eq!(get(t, NykVal::int(0)), NykVal::int(7));
        assert_eq!(get(t, float(f64::NAN)), NykVal::NIL);
        assert_eq!(get(t, NykVal::NIL), NykVal::NIL);
    }

    #[test]
    fn keys_and_values_can_be_tables_and_strings() {
        let (t, k, v) = (new_table(), new_table(), new_table());
        set(t, k, v);
        assert_eq!(get(t, k), v);
        assert_eq!(get(t, new_table()), NykVal::NIL);
        // Strings compare by content, not address.
        set(t, NykVal::string(b"a"), k);
        assert_eq!(get(t, NykVal::string(b"a")), k);
        set(t, NykVal::new(TAG_BOOL, 1), NykVal::int(1));
        assert_eq!(get(t, NykVal::new(TAG_BOOL, 1)), NykVal::int(1));
        assert_eq!(get(t, NykVal::new(TAG_BOOL, 0)), NykVal::NIL);
    }

    #[test]
    fn invalid_keys_are_rejected() {
        let mut t = Table::default();
        assert_eq!(
            t.set(NykVal::NIL, NykVal::int(1)),
            Err("table index is nil")
        );
        assert_eq!(
            t.set(float(f64::NAN), NykVal::int(1)),
            Err("table index is NaN")
        );
    }
}
