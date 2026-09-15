// Execution frame for register-based VM.

use std::rc::Rc;
use crate::vm::value::VmClosure;

pub struct CallFrame {
    pub closure: Rc<VmClosure>,
    pub ip: usize,
    // Base index into the VM register stack for this frame
    pub base: usize,
}

impl CallFrame {
    pub fn new(closure: Rc<VmClosure>, base: usize) -> Self {
        Self {
            closure,
            ip: 0,
            base,
        }
    }
}
