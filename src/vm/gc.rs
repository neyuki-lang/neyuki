// Tricolor Mark-and-Sweep Garbage Collector with weak-pool cycle collection
// and reference-safety checks preventing data corruption on live external references.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use crate::vm::buffer::VmBuffer;
use crate::vm::hash::FxHashMap;
use crate::vm::value::{StrRef, Value, VmTable};

/// The VM's global table, passed in as a GC root.
pub type Globals = FxHashMap<StrRef, Value>;

/// Values the collector manages: marking anything else is a no-op, so
/// weak tables only need to skip these.
fn is_collectible(val: &Value) -> bool {
    matches!(val, Value::Table(_) | Value::Buffer(_))
}

/// True when the table's metatable declares weak values (`__mode`
/// containing 'v'). Neyuki keys are strings/integers and can never be GC
/// objects, so only value weakness is meaningful; "k" alone is a no-op.
pub fn table_weak_values(t: &Rc<RefCell<VmTable>>) -> bool {
    t.borrow()
        .metatable
        .as_ref()
        .and_then(|mt| mt.borrow().fields.get("__mode").cloned())
        .and_then(|v| match v {
            Value::String(s) => Some(s.contains('v')),
            _ => None,
        })
        .unwrap_or(false)
}

/// The `__gc` handler on a table's metatable, if it is callable.
fn gc_handler(t: &Rc<RefCell<VmTable>>) -> Option<Value> {
    let mt = t.borrow().metatable.clone()?;
    let handler = mt.borrow().fields.get("__gc").cloned()?;
    match handler {
        Value::Closure(_) | Value::Native(_) => Some(handler),
        _ => None,
    }
}

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
    /// Creation order; finalizers run newest-first (Lua: reverse chronological).
    pub alloc_id: u64,
    /// True once the `__gc` finalizer has run; never runs twice.
    pub finalized: bool,
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
    /// Next table creation id (see `GcHeader::alloc_id`).
    pub next_alloc_id: u64,
    /// Strong refs to tables carrying a callable `__gc`, held so plain
    /// refcounting cannot destroy them before a sweep runs their
    /// finalizer. Entries leave here exactly once: at settle time, or
    /// when the metatable stops providing `__gc`. Never scanned as roots.
    pub pinned: HashMap<*const RefCell<VmTable>, Rc<RefCell<VmTable>>>,
    /// Incremental sweep cursor: remaining work when `sweeping` is true.
    /// Plain tables still to clear, dropped ptrs still to remove, buffers
    /// still to remove, weak survivors still to prune, and the doomed
    /// sets plus freed accumulator shared across slices.
    pub sweep_plain: Vec<Rc<RefCell<VmTable>>>,
    pub sweep_dropped: Vec<*const RefCell<VmTable>>,
    pub sweep_buf_remove: Vec<*const RefCell<VmBuffer>>,
    pub sweep_weak: Vec<Rc<RefCell<VmTable>>>,
    pub sweep_doomed_tables: HashSet<*const RefCell<VmTable>>,
    pub sweep_doomed_buffers: HashSet<*const RefCell<VmBuffer>>,
    pub sweep_freed: usize,
    pub sweeping: bool,
}

/// Sweep work extracted for the driver: plain garbage to clear, the
/// finalizer queue (newest first), and white-internal reference counts
/// for resurrection checks. Nothing is cleared or recolored yet.
pub struct SweepPlan {
    pub plain: Vec<Rc<RefCell<VmTable>>>,
    pub dropped: Vec<*const RefCell<VmTable>>,
    pub queue: Vec<(Rc<RefCell<VmTable>>, Value)>,
    pub internal: HashMap<*const RefCell<VmTable>, usize>,
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
            next_alloc_id: 0,
            pinned: HashMap::new(),
            sweep_plain: Vec::new(),
            sweep_dropped: Vec::new(),
            sweep_buf_remove: Vec::new(),
            sweep_weak: Vec::new(),
            sweep_doomed_tables: HashSet::new(),
            sweep_doomed_buffers: HashSet::new(),
            sweep_freed: 0,
            sweeping: false,
        }
    }

    /// Pin a table with a finalizer: the tracker itself keeps it alive so
    /// refcounting alone can never destroy it before its `__gc` runs.
    /// Idempotent.
    pub fn pin_table(&mut self, rc: &Rc<RefCell<VmTable>>) {
        self.pinned.insert(Rc::as_ptr(rc), rc.clone());
    }

    /// Release a pin (metatable lost its callable `__gc`, or the table was
    /// settled). Back to plain refcounting + sweep rules afterwards.
    pub fn unpin_table(&mut self, rc: &Rc<RefCell<VmTable>>) {
        self.pinned.remove(&Rc::as_ptr(rc));
    }

    pub fn register_table(&mut self, rc: &Rc<RefCell<VmTable>>) {
        let ptr = Rc::as_ptr(rc);
        let size = 64 + rc.borrow().array.capacity() * std::mem::size_of::<Value>();
        self.bytes_allocated += size;
        self.total_allocations += 1;
        let alloc_id = self.next_alloc_id;
        self.next_alloc_id = self.next_alloc_id.wrapping_add(1);
        self.tables.insert(
            ptr,
            (
                Rc::downgrade(rc),
                GcHeader {
                    color: self.current_white,
                    size,
                    alloc_id,
                    finalized: false,
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
                    alloc_id: 0,
                    finalized: true,
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

    /// Propagates at most `budget` gray entries; returns true when the
    /// gray stack is empty (marking complete, state moves to Sweep).
    /// Unbounded callers pass `usize::MAX`.
    pub fn propagate_n(&mut self, budget: usize) -> bool {
        let mut remaining = budget;
        while remaining > 0 {
            let Some(t_ptr) = self.gray_stack.pop() else {
                break;
            };
            remaining -= 1;
            if let Some((weak_rc, header)) = self.tables.get_mut(&t_ptr) {
                header.color = GcColor::Black;
                if let Some(rc) = weak_rc.upgrade() {
                    // Weak tables do not mark their values: entries whose
                    // value is only reachable from here die at sweep.
                    let weak = table_weak_values(&rc);
                    let tbl = rc.borrow();
                    for val in &tbl.array {
                        if !(weak && is_collectible(val)) {
                            self.mark_value(val);
                        }
                    }
                    for val in tbl.fields.values() {
                        if !(weak && is_collectible(val)) {
                            self.mark_value(val);
                        }
                    }
                    if let Some(mt) = &tbl.metatable {
                        let mt_val = Value::Table(mt.clone());
                        self.mark_value(&mt_val);
                    }
                }
            }
        }
        if self.gray_stack.is_empty() {
            self.state = GCState::Sweep;
            true
        } else {
            false
        }
    }

    /// Drains the gray stack completely (see `propagate_n`).
    pub fn propagate_all(&mut self) {
        while !self.propagate_n(usize::MAX) {}
    }

    /// Mark + propagate + identify + extract. Leaves colors and contents
    /// untouched (except marking); clearing, pruning and flips happen in
    /// `sweep_finish` after the driver settles finalizers.
    pub fn collect_plan(&mut self, stack: &[Value], globals: &Globals) -> SweepPlan {
        if self.state == GCState::Pause {
            self.mark_roots(stack, globals);
        }
        while self.state == GCState::Propagate {
            self.propagate_all();
        }
        // From here the state stays Sweep until `sweep_finish`, which also
        // makes nested collections back out instead of re-entering.
        let mut dropped = Vec::new();
        let mut white_tables: HashMap<*const RefCell<VmTable>, Rc<RefCell<VmTable>>> =
            HashMap::new();
        for (&ptr, (weak_rc, header)) in &self.tables {
            if header.color == GcColor::White0 || header.color == GcColor::White1 {
                if let Some(rc) = weak_rc.upgrade() {
                    white_tables.insert(ptr, rc);
                } else {
                    // Already dropped by Rust
                    dropped.push(ptr);
                }
            }
        }

        // Count internal references between candidate white tables, plus
        // references held by live weak tables: a value kept only through
        // weak links is garbage, so those links count as internal.
        let mut internal_ref_counts: HashMap<*const RefCell<VmTable>, usize> = HashMap::new();
        {
            let count_ref = |val: &Value,
                             white_tables: &HashMap<
                *const RefCell<VmTable>,
                Rc<RefCell<VmTable>>,
            >,
                             internal_ref_counts: &mut HashMap<
                *const RefCell<VmTable>,
                usize,
            >| {
                if let Value::Table(target_rc) = val {
                    let target_ptr = Rc::as_ptr(target_rc);
                    if white_tables.contains_key(&target_ptr) {
                        *internal_ref_counts.entry(target_ptr).or_insert(0) += 1;
                    }
                }
            };
            for rc in white_tables.values() {
                let tbl = rc.borrow();
                for val in &tbl.array {
                    count_ref(val, &white_tables, &mut internal_ref_counts);
                }
                for val in tbl.fields.values() {
                    count_ref(val, &white_tables, &mut internal_ref_counts);
                }
                if let Some(mt) = &tbl.metatable {
                    let target_ptr = Rc::as_ptr(mt);
                    if white_tables.contains_key(&target_ptr) {
                        *internal_ref_counts.entry(target_ptr).or_insert(0) += 1;
                    }
                }
            }
            // Live-but-weak holders: their values are unmarked by design,
            // so without this their targets would look externally held.
            for (ptr, (weak_rc, header)) in &self.tables {
                if header.color != GcColor::Black {
                    continue;
                }
                let Some(rc) = weak_rc.upgrade() else {
                    continue;
                };
                if !table_weak_values(&rc) {
                    continue;
                }
                let _ = ptr;
                let tbl = rc.borrow();
                for val in tbl.array.iter().chain(tbl.fields.values()) {
                    count_ref(val, &white_tables, &mut internal_ref_counts);
                }
            }
        }

        // Only collect tables with no untracked live reference: holders
        // are the white_tables entry (1), our own pin if any, and
        // white-internal refs. Anything beyond that is external: spare it.
        // (Preserved tables keep their contents; recolored in finish.)
        let mut plain: Vec<Rc<RefCell<VmTable>>> = Vec::new();
        let mut queue: Vec<(Rc<RefCell<VmTable>>, Value)> = Vec::new();
        for (&ptr, rc) in &white_tables {
            // subtract the white_tables holder
            let strong = Rc::strong_count(rc) - 1;
            let pinned = usize::from(self.pinned.contains_key(&ptr));
            let internal = internal_ref_counts.get(&ptr).copied().unwrap_or(0);
            if strong > pinned + internal {
                // Still referenced externally: spare it (recolored in finish).
                continue;
            }
            match gc_handler(rc) {
                Some(handler) if !self.tables.get(&ptr).is_some_and(|(_, h)| h.finalized) => {
                    queue.push((rc.clone(), handler));
                }
                _ => {
                    plain.push(rc.clone());
                }
            }
        }
        // Newest finalizers first (Lua: reverse chronological order).
        queue.sort_by_key(|(rc, _)| {
            std::cmp::Reverse(
                self.tables
                    .get(&Rc::as_ptr(rc))
                    .map(|(_, h)| h.alloc_id)
                    .unwrap_or(0),
            )
        });
        // Mark queued tables black so a nested collection (e.g. triggered
        // from inside a finalizer) sees them as live instead of re-queuing.
        for (rc, _) in &queue {
            if let Some((_, header)) = self.tables.get_mut(&Rc::as_ptr(rc)) {
                header.color = GcColor::Black;
            }
        }

        SweepPlan {
            plain,
            dropped,
            queue,
            internal: internal_ref_counts,
        }
    }

    /// Settles one queued finalizable after its handler ran. Returns
    /// `(cleared_ptr, freed_bytes)`: `None, 0` when the table was
    /// resurrected (kept with contents intact, never finalized again).
    /// Erroring finalizers still collect their table (Luacollects them).
    pub fn settle_table(
        &mut self,
        rc: &Rc<RefCell<VmTable>>,
        ran_ok: bool,
        internal: &HashMap<*const RefCell<VmTable>, usize>,
    ) -> (Option<*const RefCell<VmTable>>, usize) {
        let ptr = Rc::as_ptr(rc);
        let Some((_, header)) = self.tables.get_mut(&ptr) else {
            return (None, 0);
        };
        header.finalized = true;
        if ran_ok {
            // Holders now: our `rc` (1), our pin if still held, plus
            // white-internal refs (still uncleared — nothing is cleared
            // before settling). Anything beyond that is a genuine external
            // reference: resurrected.
            let strong = Rc::strong_count(rc);
            let pinned = usize::from(self.pinned.contains_key(&ptr));
            let internal_refs = internal.get(&ptr).copied().unwrap_or(0);
            if strong > 1 + pinned + internal_refs {
                // Spared: back to plain refcounting (unpin), recolor to
                // the surviving shade with contents intact. Weak pruning
                // skips non-black tables, so a weak resurrected table
                // keeps its entries this cycle.
                self.pinned.remove(&ptr);
                if let Some((_, h)) = self.tables.get_mut(&ptr) {
                    h.color = self.current_white.other_white();
                }
                return (None, 0);
            }
        }
        let freed = self.tables.get(&ptr).map(|(_, h)| h.size).unwrap_or(0);
        {
            let mut tbl = rc.borrow_mut();
            tbl.array.clear();
            tbl.fields.clear();
            tbl.metatable = None;
        }
        self.tables.remove(&ptr);
        // Whoever leaves the map loses its pin, or the pinned Rc would
        // leak (and keep the entry) forever.
        self.pinned.remove(&ptr);
        self.bytes_allocated = self.bytes_allocated.saturating_sub(freed);
        (Some(ptr), freed)
    }

    /// Removes a dead value from a surviving weak table entry-wise.
    fn prune_weak_table(
        tbl: &mut VmTable,
        doomed_tables: &HashSet<*const RefCell<VmTable>>,
        doomed_buffers: &HashSet<*const RefCell<VmBuffer>>,
    ) {
        tbl.array.retain(|v| match v {
            Value::Table(t) => !doomed_tables.contains(&Rc::as_ptr(t)),
            Value::Buffer(b) => !doomed_buffers.contains(&Rc::as_ptr(b)),
            _ => true,
        });
        tbl.fields.retain(|_, v| match v {
            Value::Table(t) => !doomed_tables.contains(&Rc::as_ptr(t)),
            Value::Buffer(b) => !doomed_buffers.contains(&Rc::as_ptr(b)),
            _ => true,
        });
    }

    /// Finishes a collection started by `collect_plan`: clears plain
    /// garbage and settled-cleared finalizables, sweeps buffers, prunes
    /// dead entries from surviving BLACK weak tables (spared and preserved
    /// tables are never pruned — their values were deliberately unmarked),
    /// then flips colors and recomputes the threshold.
    /// Stages a sweep: snapshots buffer decisions and the weak-prune
    /// list, parks everything in cursor fields. Nothing is freed yet;
    /// the first `sweep_slice` call starts freeing.
    pub fn sweep_begin(
        &mut self,
        plain: Vec<Rc<RefCell<VmTable>>>,
        dropped: Vec<*const RefCell<VmTable>>,
        settled_cleared: Vec<*const RefCell<VmTable>>,
        settled_freed: usize,
    ) {
        let mut doomed_tables: HashSet<*const RefCell<VmTable>> = HashSet::new();
        for ptr in settled_cleared {
            doomed_tables.insert(ptr);
        }

        let mut doomed_buffers: HashSet<*const RefCell<VmBuffer>> = HashSet::new();
        let mut to_remove_buffers = Vec::new();
        for (&ptr, (weak_buf, header)) in self.buffers.iter_mut() {
            if header.color == GcColor::White0 || header.color == GcColor::White1 {
                match weak_buf.upgrade() {
                    Some(rc) if Rc::strong_count(&rc) > 2 => {
                        header.color = self.current_white.other_white();
                    }
                    Some(_) => {
                        doomed_buffers.insert(ptr);
                        to_remove_buffers.push(ptr);
                    }
                    None => {
                        doomed_buffers.insert(ptr);
                        to_remove_buffers.push(ptr);
                    }
                }
            } else {
                header.color = self.current_white.other_white();
            }
        }

        // Weak survivors to prune once all clearing is done. Upgraded now
        // so they cannot vanish mid-sweep.
        let weak_black: Vec<Rc<RefCell<VmTable>>> = self
            .tables
            .iter()
            .filter_map(|(_, (weak_rc, header))| {
                if header.color == GcColor::Black {
                    weak_rc.upgrade()
                } else {
                    None
                }
            })
            .filter(table_weak_values)
            .collect();

        self.sweep_plain = plain;
        self.sweep_dropped = dropped;
        self.sweep_buf_remove = to_remove_buffers;
        self.sweep_weak = weak_black;
        self.sweep_doomed_tables = doomed_tables;
        self.sweep_doomed_buffers = doomed_buffers;
        self.sweep_freed = settled_freed;
        self.sweeping = true;
    }

    /// Performs up to `budget` units of staged sweep work (one cleared
    /// table / removed entry / pruned table each) and returns true when
    /// the sweep is fully done (pruned, colors flipped, threshold
    /// recomputed). Pruning and flipping run in the final call only, once
    /// every doomed set is complete.
    pub fn sweep_slice(&mut self, budget: usize) -> bool {
        if !self.sweeping {
            return true;
        }
        let mut remaining = budget;
        while remaining > 0 {
            if let Some(rc) = self.sweep_plain.pop() {
                let ptr = Rc::as_ptr(&rc);
                if let Some((_, header)) = self.tables.get(&ptr) {
                    self.sweep_freed += header.size;
                    self.bytes_allocated = self.bytes_allocated.saturating_sub(header.size);
                }
                {
                    let mut tbl = rc.borrow_mut();
                    tbl.array.clear();
                    tbl.fields.clear();
                    tbl.metatable = None;
                }
                self.tables.remove(&ptr);
                self.pinned.remove(&ptr);
                self.sweep_doomed_tables.insert(ptr);
                remaining -= 1;
                continue;
            }
            if let Some(ptr) = self.sweep_dropped.pop() {
                if let Some((_, header)) = self.tables.get(&ptr) {
                    self.sweep_freed += header.size;
                    self.bytes_allocated = self.bytes_allocated.saturating_sub(header.size);
                }
                self.tables.remove(&ptr);
                self.pinned.remove(&ptr);
                self.sweep_doomed_tables.insert(ptr);
                remaining -= 1;
                continue;
            }
            if let Some(ptr) = self.sweep_buf_remove.pop() {
                if let Some((_, header)) = self.buffers.get(&ptr) {
                    self.sweep_freed += header.size;
                    self.bytes_allocated = self.bytes_allocated.saturating_sub(header.size);
                }
                self.buffers.remove(&ptr);
                remaining -= 1;
                continue;
            }
            break;
        }
        if !self.sweep_plain.is_empty()
            || !self.sweep_dropped.is_empty()
            || !self.sweep_buf_remove.is_empty()
        {
            return false;
        }
        for rc in std::mem::take(&mut self.sweep_weak) {
            Self::prune_weak_table(
                &mut rc.borrow_mut(),
                &self.sweep_doomed_tables,
                &self.sweep_doomed_buffers,
            );
        }
        for (_, header) in self.tables.values_mut() {
            if header.color == GcColor::Black {
                header.color = self.current_white.other_white();
            }
        }
        self.last_freed = self.sweep_freed;
        self.sweep_freed = 0;
        self.current_white = self.current_white.other_white();
        self.state = GCState::Pause;
        self.threshold = (self.bytes_allocated * self.pause_multiplier / 100).max(1024 * 1024);
        self.sweeping = false;
        true
    }

    pub fn sweep_finish(
        &mut self,
        plain: Vec<Rc<RefCell<VmTable>>>,
        dropped: Vec<*const RefCell<VmTable>>,
        settled_cleared: Vec<*const RefCell<VmTable>>,
        settled_freed: usize,
    ) {
        self.sweep_begin(plain, dropped, settled_cleared, settled_freed);
        while !self.sweep_slice(usize::MAX) {}
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
    use std::rc::Rc;

    /// Full collection for tests without finalizers: plan, assert nothing
    /// needs a finalizer, finish. Returns bytes freed.
    fn collect_all(gc: &mut GcTracker, stack: &[Value], globals: &Globals) -> usize {
        let plan = gc.collect_plan(stack, globals);
        assert!(plan.queue.is_empty(), "unit tests use no finalizers");
        let SweepPlan { plain, dropped, .. } = plan;
        gc.sweep_finish(plain, dropped, Vec::new(), 0);
        gc.last_freed
    }

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

        let freed = collect_all(&mut gc, &stack, &globals);
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

        let freed = collect_all(&mut gc, &stack, &globals);
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

        let freed = collect_all(&mut gc, &stack, &globals);
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

        let freed = collect_all(&mut gc, &stack, &globals);
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
        let freed = collect_all(&mut gc, &stack, &globals);
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

    /// Builds a tracker with: a rooted cycle, an unrooted garbage cycle,
    /// and a rooted weak table holding a dead value.
    fn build_mixed_heap() -> (GcTracker, Vec<Value>, Globals) {
        let mut gc = GcTracker::new();
        let mut stack = Vec::new();
        let globals = crate::vm::hash::new_map();

        let root = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&root);
        let live_child = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&live_child);
        root.borrow_mut()
            .set_str("child", Value::Table(live_child.clone()));
        drop(live_child);
        stack.push(Value::Table(root.clone()));
        drop(root);

        let a = Rc::new(RefCell::new(VmTable::new()));
        let b = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&a);
        gc.register_table(&b);
        a.borrow_mut().set_str("other", Value::Table(b.clone()));
        b.borrow_mut().set_str("other", Value::Table(a.clone()));
        drop(a);
        drop(b);

        let weak = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&weak);
        let mt = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&mt);
        mt.borrow_mut().set_str("__mode", Value::String("v".into()));
        weak.borrow_mut().metatable = Some(mt.clone());
        drop(mt);
        let dead = Rc::new(RefCell::new(VmTable::new()));
        gc.register_table(&dead);
        weak.borrow_mut()
            .fields
            .insert(StrRef::from("item"), Value::Table(dead.clone()));
        drop(dead);
        stack.push(Value::Table(weak.clone()));
        drop(weak);

        (gc, stack, globals)
    }

    #[test]
    fn test_gc_sweep_slices_match_drain() {
        // Drain path (existing behavior).
        let (mut g1, s1, gl1) = build_mixed_heap();
        let plan1 = g1.collect_plan(&s1, &gl1);
        assert!(plan1.queue.is_empty());
        let SweepPlan { plain, dropped, .. } = plan1;
        g1.sweep_finish(plain, dropped, Vec::new(), 0);
        // Weak entry pruned on both paths.
        let weak_entry_gone = |gc: &GcTracker| {
            gc.tables.values().all(|(weak_rc, _)| {
                let tbl = weak_rc.upgrade().expect("live table");
                let t = tbl.borrow();
                !(t.metatable.is_some() && t.fields.contains_key("item"))
            })
        };
        assert!(weak_entry_gone(&g1));

        // Sliced path, one unit at a time (hardest cursor workout).
        let (mut g2, s2, gl2) = build_mixed_heap();
        let plan2 = g2.collect_plan(&s2, &gl2);
        assert!(plan2.queue.is_empty());
        let SweepPlan { plain, dropped, .. } = plan2;
        g2.sweep_begin(plain, dropped, Vec::new(), 0);
        while !g2.sweep_slice(1) {}
        assert!(weak_entry_gone(&g2));

        assert_eq!(g1.tables.len(), g2.tables.len());
        assert_eq!(g1.bytes_allocated, g2.bytes_allocated);
        assert_eq!(g1.last_freed, g2.last_freed);
        assert_eq!(g1.current_white, g2.current_white);
        assert_eq!(g1.state, g2.state);
    }

    #[test]
    fn test_gc_sweep_staging_flags() {
        // Staged sweep state is observable: begin stages work, a zero
        // budget slice leaves it staged, draining finishes it.
        let (mut gc, stack, globals) = build_mixed_heap();
        assert!(!gc.sweeping);
        let plan = gc.collect_plan(&stack, &globals);
        let SweepPlan { plain, dropped, .. } = plan;
        gc.sweep_begin(plain, dropped, Vec::new(), 0);
        assert!(gc.sweeping);
        assert!(!gc.sweep_slice(0));
        assert!(gc.sweeping);
        assert!(gc.sweep_slice(usize::MAX));
        assert!(!gc.sweeping);
        assert_eq!(gc.state, GCState::Pause);
    }
}
