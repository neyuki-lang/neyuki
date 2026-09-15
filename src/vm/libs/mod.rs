// VM standard libraries module.

pub mod bit;
pub mod buffer;
pub mod math;
pub mod os;
pub mod string;
pub mod table;

pub use bit::create_bit_lib;
pub use buffer::create_buffer_lib;
pub use math::create_math_lib;
pub use os::create_os_lib;
pub use string::create_string_lib;
pub use table::create_table_lib;
