//! Bridges the tree-walking runtime's native primitives into the register VM.
//!
//! `@neyuki/http`, `@neyuki/fs` and `@neyuki/io` are written in Neyuki on top
//! of `__http_*`, `__fs_*` and `__io_*` natives that speak `runtime::Value`.
//! Rather than keep a second copy of each primitive for the VM, every one is
//! wrapped in a VM native that converts the arguments over and the results
//! back. The primitives are plain `fn(Vec<Value>) -> Result<Vec<Value>, _>`
//! with no interpreter state behind them, so nothing else has to cross.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::runtime::{Int, Table};
use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

/// Tables are converted by copying, so a self-referential one would otherwise
/// recurse forever.
const MAX_CONVERT_DEPTH: usize = 64;

fn to_runtime(value: &Value, depth: usize) -> Result<crate::runtime::Value, String> {
    if depth > MAX_CONVERT_DEPTH {
        return Err("value nests too deeply to pass to a native function".to_string());
    }
    Ok(match value {
        Value::Nil => crate::runtime::Value::Nil,
        Value::Bool(b) => crate::runtime::Value::Bool(*b),
        Value::Int(i) => crate::runtime::Value::Integer(Int::Small(*i)),
        Value::BigInt(i) => crate::runtime::Value::Integer(Int::from_bigint((**i).clone())),
        Value::Float(f) => crate::runtime::Value::Number(*f),
        Value::String(s) => crate::runtime::Value::String(s.to_string()),
        Value::Table(t) => {
            let source = t.borrow();
            let mut table = Table {
                array: Vec::with_capacity(source.array.len()),
                fields: HashMap::with_capacity(source.fields.len()),
                const_fields: HashSet::new(),
                frozen: false,
                metatable: None,
            };
            for item in &source.array {
                table.array.push(to_runtime(item, depth + 1)?);
            }
            for (key, item) in &source.fields {
                table
                    .fields
                    .insert(key.to_string(), to_runtime(item, depth + 1)?);
            }
            crate::runtime::Value::Table(Rc::new(RefCell::new(table)))
        }
        Value::Closure(_) | Value::Native(..) => {
            return Err("cannot pass a function to a native primitive".to_string());
        }
        Value::Buffer(_) => {
            return Err("cannot pass a buffer to a native primitive".to_string());
        }
    })
}

fn from_runtime(value: &crate::runtime::Value, depth: usize) -> Result<Value, String> {
    if depth > MAX_CONVERT_DEPTH {
        return Err("value returned by a native function nests too deeply".to_string());
    }
    Ok(match value {
        crate::runtime::Value::Nil => Value::Nil,
        crate::runtime::Value::Bool(b) => Value::Bool(*b),
        crate::runtime::Value::Number(n) => Value::Float(*n),
        crate::runtime::Value::Integer(i) => match i {
            Int::Small(v) => Value::Int(*v),
            Int::Big(b) => Value::from_bigint((**b).clone()),
        },
        crate::runtime::Value::String(s) => Value::str(s),
        crate::runtime::Value::Table(t) => {
            let source = t.borrow();
            let mut table = VmTable::with_capacity(source.array.len(), source.fields.len());
            for item in &source.array {
                table.array.push(from_runtime(item, depth + 1)?);
            }
            for (key, item) in &source.fields {
                table
                    .fields
                    .insert(Rc::from(key.as_str()), from_runtime(item, depth + 1)?);
            }
            Value::Table(Rc::new(RefCell::new(table)))
        }
        crate::runtime::Value::Varargs(values) => {
            // Nested varargs collapse to their first value; a primitive that
            // returns several results wraps them at the top level instead,
            // where `call_primitive` spreads them.
            match values.first() {
                Some(first) => from_runtime(first, depth + 1)?,
                None => Value::Nil,
            }
        }
        crate::runtime::Value::Function(_) => {
            return Err("native primitive returned a function, which cannot cross".to_string());
        }
    })
}

/// Calls one of the runtime's native primitives with VM values.
pub(crate) fn call_primitive(
    primitive: crate::runtime::Native,
    args: &[Value],
) -> Result<Vec<Value>, String> {
    let converted = args
        .iter()
        .map(|arg| to_runtime(arg, 0))
        .collect::<Result<Vec<_>, _>>()?;
    let results = primitive(converted)?;
    let mut out = Vec::with_capacity(results.len());
    for result in &results {
        // Primitives that hand back several values, such as `__string_byte`,
        // report them as one `Varargs`; the VM wants them as separate results.
        if let crate::runtime::Value::Varargs(values) = result {
            for value in values {
                out.push(from_runtime(value, 0)?);
            }
        } else {
            out.push(from_runtime(result, 0)?);
        }
    }
    Ok(out)
}

macro_rules! bridged {
    ($wrapper:ident, $slice:path) => {
        fn $wrapper<const I: usize>(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
            call_primitive($slice[I].1, args)
        }
    };
}

bridged!(core_primitive, crate::runtime::PRIMITIVES);
bridged!(string_primitive, crate::string_lib::NATIVES);
bridged!(crypto_primitive, crate::crypto_lib::NATIVES);
bridged!(http_primitive, crate::http_lib::NATIVES);
bridged!(fs_primitive, crate::fs_lib::NATIVES);
bridged!(io_primitive, crate::io_lib::NATIVES);

/// Registers one primitive per listed index. The indices are spelled out
/// because each one instantiates its own wrapper function, and the `assert!`
/// fails the build if a primitive is added without being listed here.
macro_rules! register {
    ($vm:expr, $wrapper:ident, $slice:path, [$($index:literal),* $(,)?]) => {{
        const _: () = assert!(
            $slice.len() == [$($index),*].len(),
            concat!(stringify!($slice), " changed; update the index list"),
        );
        $({
            let (name, _) = $slice[$index];
            $vm.globals
                .insert(
                    Rc::from(name),
                    Value::Native($crate::native_def!($slice[$index].0, $wrapper::<$index>)),
                );
        })*
    }};
}

/// Makes the runtime's primitives callable from VM bytecode, so the bundled
/// `lib/*.nyk` modules run unchanged under the VM.
pub fn register_bridged_natives(vm: &mut VM) {
    register!(
        vm,
        core_primitive,
        crate::runtime::PRIMITIVES,
        [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31, 32, 33
        ]
    );
    register!(
        vm,
        string_primitive,
        crate::string_lib::NATIVES,
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
    );
    register!(
        vm,
        crypto_primitive,
        crate::crypto_lib::NATIVES,
        [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21
        ]
    );
    register!(
        vm,
        http_primitive,
        crate::http_lib::NATIVES,
        [0, 1, 2, 3, 4, 5, 6]
    );
    register!(
        vm,
        fs_primitive,
        crate::fs_lib::NATIVES,
        [0, 1, 2, 3, 4, 5, 6, 7, 8]
    );
    register!(
        vm,
        io_primitive,
        crate::io_lib::NATIVES,
        [0, 1, 2, 3, 4, 5, 6]
    );
}
