//! The `string` library. Strings also index into it, so `s:upper()` is
//! `string.upper(s)`.

use crate::vm::libs::strlib::*;
use crate::vm::value::{Value, VmTable};

pub fn create_string_lib() -> Value {
    let mut table = VmTable::new();
    macro_rules! reg {
        ($($name:literal => $func:ident),* $(,)?) => {
            $(table.set_str($name, crate::native!(concat!("string.", $name), $func));)*
        };
    }
    reg! {
        "byte" => builtin_byte,
        "char" => builtin_char,
        "count" => builtin_count,
        "endswith" => builtin_endswith,
        "find" => builtin_find,
        "format" => builtin_format,
        "gmatch" => builtin_gmatch,
        "gsub" => builtin_gsub,
        "len" => builtin_len,
        "lower" => builtin_lower,
        "ltrim" => builtin_ltrim,
        "match" => builtin_match,
        "pack" => builtin_pack,
        "packsize" => builtin_packsize,
        "rep" => builtin_rep,
        "reverse" => builtin_reverse,
        "rtrim" => builtin_rtrim,
        "split" => builtin_split,
        "startswith" => builtin_startswith,
        "sub" => builtin_sub,
        "trim" => builtin_trim,
        "unpack" => builtin_unpack,
        "upper" => builtin_upper,
    }
    crate::vm::libs::args::frozen(table)
}
