// IR Optimization module coordinating all mid-level passes.

pub mod const_fold;
pub mod dce;

pub use const_fold::constant_propagation;
pub use dce::dead_code_elimination;
