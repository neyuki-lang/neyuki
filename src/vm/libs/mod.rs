// VM standard libraries module.

pub mod bit;
pub mod bridge;
pub mod buffer;
pub mod coroutine;
pub mod debug;
pub mod json;
pub mod math;
pub mod os;
pub mod string;
pub mod table;
pub mod utf8;

pub use bit::create_bit_lib;
pub use bridge::register_bridged_natives;
pub use buffer::create_buffer_lib;
pub use coroutine::create_coroutine_lib;
pub use debug::create_debug_lib;
pub use json::create_json_lib;
pub use math::create_math_lib;
pub(crate) use math::primitive_sqrt;
pub use os::create_os_lib;
pub use string::create_string_lib;
pub use table::create_table_lib;
pub use utf8::create_utf8_lib;
