// Execution frame

use crate::vm::value::{Value, VmClosure};
use std::rc::Rc;

pub struct CallFrame {
    pub closure: Rc<VmClosure>,
    pub ip: usize,
    // Base index into the VM register stack for this frame
    pub base: usize,
    pub varargs: Vec<Value>,
    // How many values the caller's `Call` asked for; MULTRET means "all of them"
    pub want_ret: u8,
    /// The line and instruction a line hook last reported in this frame.
    pub hook_line: u32,
    pub hook_ip: usize,
}

impl CallFrame {
    pub fn new(closure: Rc<VmClosure>, base: usize) -> Self {
        Self {
            closure,
            ip: 0,
            base,
            varargs: Vec::new(),
            want_ret: 1,
            hook_line: 0,
            hook_ip: 0,
        }
    }

    pub fn with_varargs(closure: Rc<VmClosure>, base: usize, varargs: Vec<Value>) -> Self {
        Self {
            closure,
            ip: 0,
            base,
            varargs,
            want_ret: 1,
            hook_line: 0,
            hook_ip: 0,
        }
    }

    pub fn wanting(mut self, want_ret: u8) -> Self {
        self.want_ret = want_ret;
        self
    }
}
