// Built-in modules and global functions registry for Neyuki VM.

pub mod array;
pub mod gc;
pub mod global;
pub mod meta;

#[allow(unused_imports)]
pub use array::{builtin_array_create, builtin_table_clear};
pub use gc::builtin_collectgarbage;
pub use global::{
    builtin_assert, builtin_error, builtin_float, builtin_int, builtin_pcall, builtin_print,
    builtin_require, builtin_tonumber, builtin_tostring, builtin_try, builtin_type, builtin_typeof,
    builtin_xpcall,
};
pub use meta::{
    builtin_getmetatable, builtin_rawequal, builtin_rawget, builtin_rawlen, builtin_rawset,
    builtin_setmetatable,
};

use crate::vm::machine::VM;

pub fn register_all(vm: &mut VM) {
    // Globals
    vm.register_native(crate::native_def!("print", builtin_print));
    vm.register_native(crate::native_def!("assert", builtin_assert));
    vm.register_native(crate::native_def!("type", builtin_type));
    vm.register_native(crate::native_def!("typeof", builtin_typeof));
    vm.register_native(crate::native_def!("tostring", builtin_tostring));
    vm.register_native(crate::native_def!("tonumber", builtin_tonumber));
    vm.register_native(crate::native_def!("int", builtin_int));
    vm.register_native(crate::native_def!("float", builtin_float));
    vm.register_native(crate::native_def!("error", builtin_error));
    vm.register_native(crate::native_def!("pcall", builtin_pcall));
    vm.register_native(crate::native_def!("xpcall", builtin_xpcall));
    vm.register_native(crate::native_def!("try", builtin_try));
    vm.register_native(crate::native_def!("require", builtin_require));

    // Garbage collector
    vm.register_native(crate::native_def!("collectgarbage", builtin_collectgarbage));

    // Metatable and raw functions
    vm.register_native(crate::native_def!("setmetatable", builtin_setmetatable));
    vm.register_native(crate::native_def!("getmetatable", builtin_getmetatable));
    vm.register_native(crate::native_def!("rawget", builtin_rawget));
    vm.register_native(crate::native_def!("rawset", builtin_rawset));
    vm.register_native(crate::native_def!("rawequal", builtin_rawequal));
    vm.register_native(crate::native_def!("rawlen", builtin_rawlen));
}
