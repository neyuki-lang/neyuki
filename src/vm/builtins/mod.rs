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
    vm.register_native("print", builtin_print);
    vm.register_native("assert", builtin_assert);
    vm.register_native("type", builtin_type);
    vm.register_native("typeof", builtin_typeof);
    vm.register_native("tostring", builtin_tostring);
    vm.register_native("tonumber", builtin_tonumber);
    vm.register_native("int", builtin_int);
    vm.register_native("float", builtin_float);
    vm.register_native("error", builtin_error);
    vm.register_native("pcall", builtin_pcall);
    vm.register_native("xpcall", builtin_xpcall);
    vm.register_native("try", builtin_try);
    vm.register_native("require", builtin_require);

    // Garbage collector
    vm.register_native("collectgarbage", builtin_collectgarbage);

    // Metatable and raw functions
    vm.register_native("setmetatable", builtin_setmetatable);
    vm.register_native("getmetatable", builtin_getmetatable);
    vm.register_native("rawget", builtin_rawget);
    vm.register_native("rawset", builtin_rawset);
    vm.register_native("rawequal", builtin_rawequal);
    vm.register_native("rawlen", builtin_rawlen);
}
