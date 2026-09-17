// Execution frame

use std::rc::Rc;
use crate::vm::value::{Value, VmClosure};

pub struct CallFrame {
    pub closure: Rc<VmClosure>,
    pub ip: usize,
    // Base index into the VM register stack for this frame
    pub base: usize,
    pub varargs: Vec<Value>,
}

impl CallFrame {
    pub fn new(closure: Rc<VmClosure>, base: usize) -> Self {
        Self {
            closure,
            ip: 0,
            base,
            varargs: Vec::new(),
        }
    }

    pub fn with_varargs(closure: Rc<VmClosure>, base: usize, varargs: Vec<Value>) -> Self {
        Self {
            closure,
            ip: 0,
            base,
            varargs,
        }
    }
}
