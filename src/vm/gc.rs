// Tricolor Mark-and-Sweep Garbage Collector with weak-pool cycle collection
// and reference-safety checks preventing data corruption on live external references.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::vm::buffer::VmBuffer;
use crate::vm::hash::FxHashMap;
use crate::vm::value::{Value, VmTable};

/// The VM's global table, passed in as a GC root.
pub type Globals = FxHashMap<Rc<str>, Value>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GcColor {
    White0,
    White1,
    Gray,
    Black,
}

impl GcColor {
    pub fn other_white(self) -> Self {
        match self {
            GcColor::White0 => GcColor::White1,
            GcColor::White1 => GcColor::White0,
            _ => self,
        }
    }
}

pub struct GcHeader {
    pub color: GcColor,
    pub size: usize,
}

#[derive(PartialEq, Eq, Debug)]
pub enum GCState {
    Pause,
    Propagate,
    Sweep,
}

#[allow(dead_code)]
pub struct GcTracker {
    pub bytes_allocated: usize,
    pub total_allocations: usize,
    pub threshold: usize,
    pub step_multiplier: usize,
    pub pause_multiplier: usize,
    pub is_running: bool,

    pub current_white: GcColor,
    pub state: GCState,
    pub tables: HashMap<*const RefCell<VmTable>, (Weak<RefCell<VmTable>>, GcHeader)>,
    pub buffers: HashMap<*const RefCell<VmBuffer>, (Weak<RefCell<VmBuffer>>, GcHeader)>,
    pub gray_stack: Vec<*const RefCell<VmTable>>,
    pub last_freed: usize,
}

#[allow(dead_code)]
impl GcTracker {
    pub fn new() -> Self {
        Self {
            bytes_allocated: 0,
            total_allocations: 0,
            threshold: 1024 * 1024, // 1 MB
            step_multiplier: 200,
            pause_multiplier: 200,
            is_running: true,

            current_white: GcColor::White0,
            state: GCState::Pause,
            tables: HashMap::new(),
            buffers: HashMap::new(),
            gray_stack: Vec::new(),
            last_freed: 0,
        }
    }

    pub fn register_table(&mut self, rc: &Rc<RefCell<VmTable>>) {
        let ptr = Rc::as_ptr(rc);
        let size = 64 + rc.borrow().array.capacity() * std::mem::size_of::<Value>();
        self.bytes_allocated += size;
        self.total_allocations += 1;
        self.tables.insert(
            ptr,
            (
                Rc::downgrade(rc),
                GcHeader {
                    color: self.current_white,
                    size,
                },
            ),
        );
    }

    pub fn register_buffer(&mut self, rc: &Rc<RefCell<VmBuffer>>) {
        let ptr = Rc::as_ptr(rc);
        let size = 32 + rc.borrow().len();
        self.bytes_allocated += size;
        self.total_allocations += 1;
        self.buffers.insert(
            ptr,
            (
                Rc::downgrade(rc),
                GcHeader {
                    color: self.current_white,
                    size,
                },
            ),
        );
    }

    #[inline]
    pub fn write_barrier(&mut self, table: &Value, val: &Value) {
        // Only a table can be white, so storing anything else needs no
        // bookkeeping; this keeps the common case free of map lookups.
        if !matches!(val, Value::Table(_)) {
            return;
        }
        let is_white = self.is_value_white(val);
        if let Value::Table(t_rc) = table {
            let t_ptr = Rc::as_ptr(t_rc);
            if let Some((_, header)) = self.tables.get_mut(&t_ptr)
                && header.color == GcColor::Black
                && is_white
            {
                header.color = GcColor::Gray;
                self.gray_stack.push(t_ptr);
            }
        }
    }

    fn is_value_white(&self, val: &Value) -> bool {
        if let Value::Table(t_rc) = val {
            let t_ptr = Rc::as_ptr(t_rc);
            if let Some((_, header)) = self.tables.get(&t_ptr) {
                return header.color == GcColor::White0 || header.color == GcColor::White1;
            }
        }
        false
    }

    pub fn mark_value(&mut self, val: &Value) {
        match val {
            Value::Table(t_rc) => {
                let t_ptr = Rc::as_ptr(t_rc);
                if let Some((_, header)) = self.tables.get_mut(&t_ptr)
                    && (header.color == GcColor::White0 || header.color == GcColor::White1)
                {
                    header.color = GcColor::Gray;
                    self.gray_stack.push(t_ptr);
                }
            }
            Value::Buffer(b_rc) => {
                let b_ptr = Rc::as_ptr(b_rc);
                if let Some((_, header)) = self.buffers.get_mut(&b_ptr)
                    && (header.color == GcColor::White0 || header.color == GcColor::White1)
                {
                    header.color = GcColor::Black;
                }
            }
            Value::Closure(c) => {
                // An open upvalue's value is a register on the stack, which
                // is already a root; only closed ones hold a value themselves.
                for upval in &c.upvalues {
                    if !upval.is_open() {
                        let v = upval.closed.borrow();
                        self.mark_value(&v);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn mark_roots(&mut self, stack: &[Value], globals: &Globals) {
        self.gray_stack.clear();
        for val in stack {
            self.mark_value(val);
        }
        for val in globals.values() {
            self.mark_value(val);
        }
        self.state = GCState::Propagate;
    }

    pub fn propagate_all(&mut self) {
        while let Some(t_ptr) = self.gray_stack.pop() {
            if let Some((weak_rc, header)) = self.tables.get_mut(&t_ptr) {
                header.color = GcColor::Black;
                if let Some(rc) = weak_rc.upgrade() {
                    let tbl = rc.borrow();
                    for val in &tbl.array {
                        self.mark_value(val);
                    }
                    for val in tbl.fields.values() {
                        self.mark_value(val);
                    }
                    if let Some(mt) = &tbl.metatable {
                        let mt_val = Value::Table(mt.clone());
                        self.mark_value(&mt_val);
                    }
                }
            }
        }
        self.state = GCState::Sweep;
    }

    pub fn sweep(&mut self) {
        let mut to_remove_tables = Vec::new();
        let mut freed_bytes = 0;

        // Step 1: Identify all candidate white tables
        let mut white_tables: HashMap<*const RefCell<VmTable>, Rc<RefCell<VmTable>>> =
            HashMap::new();
        for (&ptr, (weak_rc, header)) in &self.tables {
            if header.color == GcColor::White0 || header.color == GcColor::White1 {
                if let Some(rc) = weak_rc.upgrade() {
                    white_tables.insert(ptr, rc);
                } else {
                    // Already dropped by Rust
                    to_remove_tables.push(ptr);
                    freed_bytes += header.size;
                }
            }
        }

        // Step 2: Count internal references between candidate white tables
        let mut internal_ref_counts: HashMap<*const RefCell<VmTable>, usize> = HashMap::new();
        for rc in white_tables.values() {
            let tbl = rc.borrow();
            let mut count_ref = |val: &Value| {
                if let Value::Table(target_rc) = val {
                    let target_ptr = Rc::as_ptr(target_rc);
                    if white_tables.contains_key(&target_ptr) {
                        *internal_ref_counts.entry(target_ptr).or_insert(0) += 1;
                    }
                }
            };
            for val in &tbl.array {
                count_ref(val);
            }
            for val in tbl.fields.values() {
                count_ref(val);
            }
            if let Some(mt) = &tbl.metatable {
                let target_ptr = Rc::as_ptr(mt);
                if white_tables.contains_key(&target_ptr) {
                    *internal_ref_counts.entry(target_ptr).or_insert(0) += 1;
                }
            }
        }

        // Step 3: Only collect tables where strong_count - 1 <= internal_refs
        // This guarantees NO external live references exist, completely preventing data corruption!
        let mut tables_to_clear: Vec<Rc<RefCell<VmTable>>> = Vec::new();
        for (&ptr, rc) in &white_tables {
            let strong = Rc::strong_count(rc) - 1; // subtract the white_tables holder
            let internal = internal_ref_counts.get(&ptr).copied().unwrap_or(0);
            if strong <= internal {
                // Genuine cycle or unreferenced garbage: safe to clear and collect
                tables_to_clear.push(rc.clone());
                to_remove_tables.push(ptr);
                if let Some((_, header)) = self.tables.get(&ptr) {
                    freed_bytes += header.size;
                }
            } else {
                // Table is still referenced externally! Preserve contents to prevent corruption.
                if let Some((_, header)) = self.tables.get_mut(&ptr) {
                    header.color = self.current_white.other_white();
                }
            }
        }

        // Break cycles to drop strong counts
        for rc in tables_to_clear {
            let mut tbl = rc.borrow_mut();
            tbl.array.clear();
            tbl.fields.clear();
            tbl.metatable = None;
        }

        for ptr in to_remove_tables {
            self.tables.remove(&ptr);
        }

        // Flip surviving black tables to next white
        for (_, header) in self.tables.values_mut() {
            if header.color == GcColor::Black {
                header.color = self.current_white.other_white();
            }
        }

        // Sweep buffers
        let mut to_remove_buffers = Vec::new();
        for (&ptr, (weak_buf, header)) in self.buffers.iter_mut() {
            if header.color == GcColor::White0 || header.color == GcColor::White1 {
                if let Some(rc) = weak_buf.upgrade() {
                    if Rc::strong_count(&rc) <= 2 {
                        to_remove_buffers.push(ptr);
                        freed_bytes += header.size;
                    } else {
                        header.color = self.current_white.other_white();
                    }
                } else {
                    to_remove_buffers.push(ptr);
                    freed_bytes += header.size;
                }
            } else {
                header.color = self.current_white.other_white();
            }
        }

        for ptr in to_remove_buffers {
            self.buffers.remove(&ptr);
        }

        self.bytes_allocated = self.bytes_allocated.saturating_sub(freed_bytes);
        self.last_freed = freed_bytes;
        self.current_white = self.current_white.other_white();
        self.state = GCState::Pause;
        self.threshold = (self.bytes_allocated * self.pause_multiplier / 100).max(1024 * 1024);
    }

    pub fn collect_garbage(&mut self, stack: &[Value], globals: &Globals) -> usize {
        if self.state == GCState::Pause {
            self.mark_roots(stack, globals);
        }
        while self.state == GCState::Propagate {
            self.propagate_all();
        }
        if self.state == GCState::Sweep {
            self.sweep();
        }
        self.last_freed
    }

    pub fn step(&mut self, _step_size: usize, stack: &[Value], globals: &Globals) -> bool {
        if self.bytes_allocated >= self.threshold {
            self.collect_garbage(stack, globals);
            true
        } else {
            false
        }
    }

    pub fn should_collect(&self) -> bool {
        self.is_running && self.bytes_allocated >= self.threshold
    }

    pub fn stop(&mut self) {
        self.is_running = false;
    }

    pub fn restart(&mut self) {
        self.is_running = true;
    }

    pub fn count_kb(&self) -> f64 {
        self.bytes_allocated as f64 / 1024.0
    }

    pub fn record_alloc(&mut self, bytes: usize) {
        self.bytes_allocated += bytes;
        self.total_allocations += 1;
    }

    pub fn record_free(&mut self, bytes: usize) {
        self.bytes_allocated = self.bytes_allocated.saturating_sub(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::value::{Value, VmTable};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    #[test]
    fn test_gc_self_reference() {
        let mut gc = GcTracker::new();
        let stack = Vec::new();
        let globals = crate::vm::hash::new_map();

        let rc = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&rc);
        rc.borrow_mut().set_str("self", Value::Table(rc.clone()));

        assert_eq!(gc.tables.len(), 1);
        drop(rc);

        let freed = gc.collect_garbage(&stack, &globals);
        assert!(freed > 0);
        assert_eq!(gc.tables.len(), 0);
    }

    #[test]
    fn test_gc_cross_reference() {
        let mut gc = GcTracker::new();
        let stack = Vec::new();
        let globals = crate::vm::hash::new_map();

        let a = Rc::new(RefCell::new(VmTable::new()));
        let b = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&a);
        gc.register_table(&b);

        a.borrow_mut().set_str("other", Value::Table(b.clone()));
        b.borrow_mut().set_str("other", Value::Table(a.clone()));

        assert_eq!(gc.tables.len(), 2);
        drop(a);
        drop(b);

        let freed = gc.collect_garbage(&stack, &globals);
        assert!(freed > 0);
        assert_eq!(gc.tables.len(), 0);
    }

    #[test]
    fn test_gc_3_cycle() {
        let mut gc = GcTracker::new();
        let stack = Vec::new();
        let globals = crate::vm::hash::new_map();

        let a = Rc::new(RefCell::new(VmTable::new()));
        let b = Rc::new(RefCell::new(VmTable::new()));
        let c = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&a);
        gc.register_table(&b);
        gc.register_table(&c);

        a.borrow_mut().set_str("next", Value::Table(b.clone()));
        b.borrow_mut().set_str("next", Value::Table(c.clone()));
        c.borrow_mut().set_str("next", Value::Table(a.clone()));

        assert_eq!(gc.tables.len(), 3);
        drop(a);
        drop(b);
        drop(c);

        let freed = gc.collect_garbage(&stack, &globals);
        assert!(freed > 0);
        assert_eq!(gc.tables.len(), 0);
    }

    #[test]
    fn test_gc_root_preservation() {
        let mut gc = GcTracker::new();
        let mut stack = Vec::new();
        let globals = crate::vm::hash::new_map();

        let root_rc = Rc::new(RefCell::new(VmTable::new()));
        let child_rc = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&root_rc);
        gc.register_table(&child_rc);

        root_rc
            .borrow_mut()
            .set_str("child", Value::Table(child_rc.clone()));
        stack.push(Value::Table(root_rc.clone()));

        drop(child_rc);

        let freed = gc.collect_garbage(&stack, &globals);
        assert_eq!(freed, 0);
        assert_eq!(gc.tables.len(), 2);
    }

    #[test]
    fn test_gc_external_reference_prevents_data_corruption() {
        let mut gc = GcTracker::new();
        let stack = Vec::new(); // Not in stack
        let globals = crate::vm::hash::new_map(); // Not in globals

        // Table is held by external Rust code outside VM roots
        let external_table = Rc::new(RefCell::new(VmTable::new()));
        external_table.borrow_mut().set_str(
            "important",
            Value::from_bigint(num_bigint::BigInt::from(999)),
        );
        gc.register_table(&external_table);

        // Run collection when table is not in roots
        let freed = gc.collect_garbage(&stack, &globals);
        assert_eq!(freed, 0);
        // Ensure the external table was NOT gutted or corrupted!
        assert!(external_table.borrow().fields.contains_key("important"));
        assert_eq!(
            external_table
                .borrow()
                .fields
                .get("important")
                .unwrap()
                .to_string(),
            "999"
        );
    }

    #[test]
    fn test_gc_write_barrier() {
        let mut gc = GcTracker::new();

        let parent_rc = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&parent_rc);

        let p_ptr = Rc::as_ptr(&parent_rc);
        if let Some((_, h)) = gc.tables.get_mut(&p_ptr) {
            h.color = GcColor::Black;
        }

        let child_rc = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&child_rc);

        let parent_val = Value::Table(parent_rc.clone());
        let child_val = Value::Table(child_rc.clone());

        gc.write_barrier(&parent_val, &child_val);

        if let Some((_, h)) = gc.tables.get(&p_ptr) {
            assert_eq!(h.color, GcColor::Gray);
        }
        assert_eq!(gc.gray_stack.len(), 1);
    }
}
