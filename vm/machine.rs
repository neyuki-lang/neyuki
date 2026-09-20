use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use num_integer::Integer as _;

use crate::bytecode::instruction::{Instruction, MULTRET};
use crate::bytecode::proto::Proto;
use crate::vm::frame::CallFrame;
use crate::vm::gc::GcTracker;
use crate::vm::hash::{FxHashMap, new_map};
use crate::vm::libs::{
    create_bit_lib, create_buffer_lib, create_coroutine_lib, create_debug_lib, create_json_lib,
    create_math_lib, create_os_lib, create_string_lib, create_table_lib, create_utf8_lib,
    register_bridged_natives,
};
use crate::vm::ops;
use crate::vm::value::{NativeDef, StrRef, Upvalue, Value, VmClosure, VmTable};

const MAX_CALL_DEPTH: usize = 512;

/// A register of the running frame. The interpreter loop only calls this
/// with indices below `base + max_registers` of a verified prototype, and
/// sizes the stack to cover that window before dispatching, so the bounds
/// check is skipped.
#[inline(always)]
fn slot(stack: &[Value], idx: usize) -> &Value {
    debug_assert!(idx < stack.len());
    // SAFETY: see above; the caller upholds `idx < stack.len()`.
    unsafe { stack.get_unchecked(idx) }
}

#[inline(always)]
fn slot_mut(stack: &mut [Value], idx: usize) -> &mut Value {
    debug_assert!(idx < stack.len());
    // SAFETY: as for `slot`.
    unsafe { stack.get_unchecked_mut(idx) }
}

/// An open upvalue parked with a suspended stack: the register index it will
/// reopen at, and the upvalue itself (holding the value while parked).
pub type ParkedUpvalue = (usize, Rc<Upvalue>);

pub struct CoroutineState {
    pub stack: Vec<Value>,
    pub frames: Vec<CallFrame>,
    pub status: String,
    pub func: Value,
    pub yield_callee: u8,
    pub yield_retc: u8,
    pub yield_values: Vec<Value>,
    pub open_upvalues: Vec<ParkedUpvalue>,
}

/// A `debug.sethook` hook (v1: call/return events only). `None` means no
/// hook is set, in which case event checks cost a single predictable
/// branch at call/return boundaries and nothing in the dispatch loop.
#[derive(Clone)]
pub struct DebugHook {
    pub func: Value,
    pub on_call: bool,
    pub on_return: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    Call,
    TailCall,
    Return,
}

impl HookEvent {
    fn name(self) -> &'static str {
        match self {
            HookEvent::Call => "call",
            HookEvent::TailCall => "tail call",
            HookEvent::Return => "return",
        }
    }

    fn wants(self, hook: &DebugHook) -> bool {
        match self {
            HookEvent::Call | HookEvent::TailCall => hook.on_call,
            HookEvent::Return => hook.on_return,
        }
    }
}

pub struct VM {
    pub stack: Vec<Value>,
    pub frames: Vec<CallFrame>,
    pub globals: FxHashMap<StrRef, Value>,
    pub gc: GcTracker,
    pub coroutines: HashMap<usize, Rc<RefCell<CoroutineState>>>,
    pub next_co_id: usize,
    pub current_co: Option<usize>,
    pub is_yielding: bool,
    /// Active `debug.sethook` hook, if any.
    pub hook: Option<DebugHook>,
    /// True while a hook function itself runs: hooks never trigger hooks.
    pub in_hook: bool,
    /// Upvalues that still point at a live register of the current stack,
    /// sorted by register index. Nearly always empty or very short, which is
    /// why a plain vector beats a map here.
    pub open_upvalues: Vec<ParkedUpvalue>,
    /// One past the last value produced by the most recent MULTRET `Call`,
    /// `Vararg` or returning frame. Only meaningful for the instruction that
    /// immediately consumes it.
    pub top: usize,
    /// Modules already loaded by `require`, so each one runs once.
    pub modules: HashMap<String, Value>,
    /// Every value the frame that just unwound to a `run_to_depth` boundary
    /// returned, so `call_function` can hand back more than the first.
    returned: Vec<Value>,
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

impl VM {
    pub fn new() -> Self {
        let mut vm = Self {
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
            globals: new_map(),
            gc: GcTracker::new(),
            coroutines: HashMap::new(),
            next_co_id: 1,
            current_co: None,
            is_yielding: false,
            hook: None,
            in_hook: false,
            open_upvalues: Vec::new(),
            top: 0,
            modules: HashMap::new(),
            returned: Vec::new(),
        };
        vm.register_builtins();
        vm
    }

    fn register_builtins(&mut self) {
        crate::vm::builtins::register_all(self);

        self.set_global("bit", create_bit_lib());
        self.set_global("bit32", create_bit_lib());
        self.set_global("buffer", create_buffer_lib());
        self.set_global("math", create_math_lib());
        self.set_global("string", create_string_lib());
        self.set_global("table", create_table_lib());
        self.set_global("os", create_os_lib());
        self.set_global("coroutine", create_coroutine_lib());
        self.set_global("debug", create_debug_lib());
        self.set_global("json", create_json_lib());
        self.set_global("utf8", create_utf8_lib());

        register_bridged_natives(self);
    }

    pub fn set_global(&mut self, name: &str, value: Value) {
        self.globals.insert(StrRef::from(name), value);
    }

    pub fn register_native(&mut self, def: &'static NativeDef) {
        self.set_global(def.name, Value::Native(def));
    }

    pub fn execute(&mut self, proto: Proto) -> Result<Value, String> {
        let closure = Rc::new(VmClosure {
            proto: Rc::new(proto),
            upvalues: Vec::new(),
        });
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();

        // Ensure stack has enough capacity for main frame
        let max_reg = closure.proto.max_registers as usize;
        self.stack.resize(max_reg + 1, Value::Nil);

        self.frames.push(CallFrame::new(closure, 0));
        let run_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run()));
        match run_res {
            Ok(res) => res,
            Err(panic_payload) => {
                let msg = if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unexpected panic during VM execution".to_string()
                };
                Err(format!("VM runtime panic caught: {}", msg))
            }
        }
    }

    /// Writes a register, growing the stack if the frame's declared register
    /// window was exceeded (variadic results can spill past it).
    #[inline]
    fn set_reg_grow(&mut self, idx: usize, val: Value) {
        if idx >= self.stack.len() {
            self.stack.resize(idx + 1, Value::Nil);
        }
        self.stack[idx] = val;
    }

    // ----- upvalues -------------------------------------------------------

    /// Moves every open upvalue at or above `from` off the stack and into
    /// its own cell. Called when the frame owning those registers ends.
    pub fn close_upvalues(&mut self, from: usize) {
        while let Some((idx, _)) = self.open_upvalues.last() {
            if *idx < from {
                break;
            }
            let (idx, uv) = self.open_upvalues.pop().unwrap();
            let val = self.stack.get(idx).cloned().unwrap_or(Value::Nil);
            uv.close(val);
        }
    }

    /// Finds or creates the open upvalue for stack slot `idx`.
    fn capture_upvalue(&mut self, idx: usize) -> Rc<Upvalue> {
        if idx >= self.stack.len() {
            self.stack.resize(idx + 1, Value::Nil);
        }
        let pos = self.open_upvalues.partition_point(|(i, _)| *i < idx);
        if let Some((i, uv)) = self.open_upvalues.get(pos)
            && *i == idx
        {
            return uv.clone();
        }
        let uv = Upvalue::open(idx);
        self.open_upvalues.insert(pos, (idx, uv.clone()));
        uv
    }

    /// Detaches every open upvalue from the current stack so the stack can be
    /// put aside (a coroutine switch). Each keeps its value in its cell until
    /// `unpark_upvalues` reattaches it.
    pub fn park_upvalues(&mut self) -> Vec<ParkedUpvalue> {
        let parked = std::mem::take(&mut self.open_upvalues);
        for (idx, uv) in &parked {
            let val = self.stack.get(*idx).cloned().unwrap_or(Value::Nil);
            uv.close(val);
        }
        parked
    }

    /// Reattaches upvalues parked by `park_upvalues` to the (now current)
    /// stack they belong to, copying any writes made meanwhile back into
    /// their registers.
    pub fn unpark_upvalues(&mut self, parked: Vec<ParkedUpvalue>) {
        for (idx, uv) in &parked {
            let val = std::mem::replace(&mut *uv.closed.borrow_mut(), Value::Nil);
            self.set_reg_grow(*idx, val);
            uv.location.set(Some(*idx));
        }
        self.open_upvalues = parked;
    }

    #[inline]
    pub fn upvalue_get(&self, uv: &Upvalue) -> Value {
        match uv.location.get() {
            Some(idx) => self.stack.get(idx).cloned().unwrap_or(Value::Nil),
            None => uv.closed.borrow().clone(),
        }
    }

    #[inline]
    pub fn upvalue_set(&mut self, uv: &Upvalue, val: Value) {
        match uv.location.get() {
            Some(idx) => self.set_reg_grow(idx, val),
            None => *uv.closed.borrow_mut() = val,
        }
    }

    // ----- calls ----------------------------------------------------------

    /// Runs a module's top-level code and returns its value, leaving the
    /// frames and registers of whatever called `require` untouched.
    pub fn execute_module(&mut self, proto: Proto) -> Result<Value, String> {
        let closure = Rc::new(VmClosure {
            proto: Rc::new(proto),
            upvalues: Vec::new(),
        });
        let depth = self.frames.len();
        let prev_stack_len = self.stack.len();
        let base = prev_stack_len + 1;
        let needed = base + closure.proto.max_registers as usize + 1;
        if needed >= self.stack.len() {
            self.stack.resize(needed + 1, Value::Nil);
        }
        self.frames.push(CallFrame::new(closure, base));
        let result = self.run_to_depth(depth);
        self.frames.truncate(depth);
        // Upvalues captured by the module's closures live on in the closures
        // themselves; closing them here just moves the values across.
        self.close_upvalues(prev_stack_len);
        self.stack.truncate(prev_stack_len);
        result
    }

    /// Writes the results of a native (or `__call`) invocation back into the
    /// caller's registers, starting at the register that held the callee.
    fn store_call_results(&mut self, callee: u8, retc: u8, results: &[Value]) {
        let base = self.frames.last().map(|f| f.base).unwrap_or(0);
        let dest = base + callee as usize;
        let count = match retc {
            MULTRET => results.len(),
            0 => 1,
            n => n as usize,
        };
        if dest + count > self.stack.len() {
            self.stack.resize(dest + count, Value::Nil);
        }
        for i in 0..count {
            self.stack[dest + i] = results.get(i).cloned().unwrap_or(Value::Nil);
        }
        if retc == MULTRET {
            self.top = dest + count;
        }
    }

    pub fn call_function(&mut self, func: Value, args: &[Value]) -> Result<Vec<Value>, String> {
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(format!(
                "call stack overflow: exceeded maximum call depth of {}",
                MAX_CALL_DEPTH
            ));
        }
        match func {
            Value::Native(def) => {
                if let Err(err) = self.fire_hook(HookEvent::Call) {
                    return Err(err);
                }
                match (def.func)(self, args) {
                    Ok(results) => {
                        if let Err(err) = self.fire_hook(HookEvent::Return) {
                            return Err(err);
                        }
                        Ok(results)
                    }
                    Err(err) => Err(err),
                }
            }
            Value::Closure(c) => {
                let depth = self.frames.len();
                let prev_stack_len = self.stack.len();
                let base = prev_stack_len + 1;
                let needed = base + c.proto.max_registers as usize + args.len() + 1;
                if needed >= self.stack.len() {
                    self.stack.resize(needed + 1, Value::Nil);
                }
                for (i, arg) in args.iter().enumerate() {
                    self.stack[base + i] = arg.clone();
                }
                let num_params = c.proto.num_params as usize;
                for i in args.len()..num_params {
                    self.stack[base + i] = Value::Nil;
                }
                let varargs = if args.len() > num_params {
                    args[num_params..].to_vec()
                } else {
                    Vec::new()
                };
                self.frames
                    .push(CallFrame::with_varargs(c.clone(), base, varargs));
                if let Err(err) = self.fire_hook(HookEvent::Call) {
                    if !self.is_yielding {
                        self.frames.truncate(depth);
                        self.close_upvalues(prev_stack_len);
                        self.stack.truncate(prev_stack_len);
                    }
                    return Err(err);
                }
                self.returned.clear();
                match self.run_to_depth(depth) {
                    Ok(res) => {
                        // No return hook here: the callee reports its own
                        // return through its Return instruction (or
                        // fall-off-end), so firing again would double-report.
                        if !self.is_yielding {
                            self.close_upvalues(prev_stack_len);
                            self.stack.truncate(prev_stack_len);
                        }
                        // A closure can return several values, which callers
                        // such as a generic `for` over an iterator rely on.
                        let returned = std::mem::take(&mut self.returned);
                        if returned.is_empty() {
                            Ok(vec![res])
                        } else {
                            Ok(returned)
                        }
                    }
                    Err(err) => {
                        if !self.is_yielding {
                            self.frames.truncate(depth);
                            self.close_upvalues(prev_stack_len);
                            self.stack.truncate(prev_stack_len);
                        }
                        Err(err)
                    }
                }
            }
            Value::Table(ref t) => {
                let handler = t.borrow().metamethod("__call");
                if let Some(handler) = handler {
                    let mut call_args = Vec::with_capacity(args.len() + 1);
                    call_args.push(func.clone());
                    call_args.extend_from_slice(args);
                    return self.call_function(handler, &call_args);
                }
                Err("attempt to call a table value without __call metamethod".to_string())
            }
            _ => Err("attempt to call a non-function value".to_string()),
        }
    }

    fn get_binop_metamethod(&self, a: &Value, b: &Value, event: &str) -> Option<Value> {
        if let Value::Table(t) = a
            && let Some(h) = t.borrow().metamethod(event)
        {
            return Some(h);
        }
        if let Value::Table(t) = b
            && let Some(h) = t.borrow().metamethod(event)
        {
            return Some(h);
        }
        None
    }

    fn get_unop_metamethod(&self, val: &Value, event: &str) -> Option<Value> {
        if let Value::Table(t) = val {
            return t.borrow().metamethod(event);
        }
        None
    }

    /// Runs the active debug hook for `event`, if one is set and listening.
    /// Hooks never trigger hooks (`in_hook` guard); a hook error propagates
    /// to the caller, which is responsible for unwinding its own frame.
    fn fire_hook(&mut self, event: HookEvent) -> Result<(), String> {
        let (func, wanted) = match &self.hook {
            Some(hook) if !self.in_hook => (hook.func.clone(), event.wants(hook)),
            _ => return Ok(()),
        };
        if !wanted {
            return Ok(());
        }
        self.in_hook = true;
        let arg = Value::str(event.name());
        let res = self.call_function(func, &[arg]);
        self.in_hook = false;
        res.map(|_| ())
    }

    /// Full collection with finalizers: mark, extract the finalizer queue,
    /// run each handler newest-first, settle resurrections, then finish the
    /// sweep. A handler error is recorded and propagated after the whole
    /// queue ran; its table is still collected (run-once guarantee).
    pub(crate) fn gc_collect(&mut self) -> Result<(), String> {
        let crate::vm::gc::SweepPlan {
            queue,
            internal,
            plain,
            dropped,
        } = self.gc.collect_plan(&self.stack, &self.globals);
        let mut settled_cleared = Vec::new();
        let mut settled_freed = 0;
        let mut first_err: Option<String> = None;
        for (rc, handler) in &queue {
            let arg = Value::Table(rc.clone());
            let res = self.call_function(handler.clone(), std::slice::from_ref(&arg));
            let (cleared, freed) = self.gc.settle_table(rc, res.is_ok(), &internal);
            if let Some(ptr) = cleared {
                settled_cleared.push(ptr);
            }
            settled_freed += freed;
            if let Err(e) = res {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        self.gc
            .sweep_finish(plain, dropped, settled_cleared, settled_freed);
        if let Some(e) = first_err {
            return Err(e);
        }
        Ok(())
    }

    pub fn run(&mut self) -> Result<Value, String> {
        self.run_to_depth(0)
    }

    /// Calls a binary metamethod and stores its first result in `dst`.
    fn call_binop_mm(
        &mut self,
        mm: Value,
        va: Value,
        vb: Value,
        base: usize,
        dst: u8,
    ) -> Result<(), String> {
        let res = self.call_function(mm, &[va, vb])?;
        self.set_reg_grow(
            base + dst as usize,
            res.into_iter().next().unwrap_or(Value::Nil),
        );
        Ok(())
    }

    /// Calls a comparison metamethod and reports whether its result is truthy.
    fn call_cmp_mm(&mut self, mm: Value, va: Value, vb: Value) -> Result<bool, String> {
        let res = self.call_function(mm, &[va, vb])?;
        Ok(res.first().is_some_and(|v| v.is_truthy()))
    }

    /// The interpreter loop.
    ///
    /// The outer loop (re)loads the current frame's code and register base
    /// into locals; the inner loop dispatches instructions against them.
    /// Anything that changes the current frame breaks out to the outer loop,
    /// and anything that can run other code (natives, metamethods) writes
    /// the instruction pointer back first so that code sees a consistent
    /// frame.
    pub fn run_to_depth(&mut self, target_depth: usize) -> Result<Value, String> {
        'frames: loop {
            if self.frames.len() <= target_depth {
                return Ok(Value::Nil);
            }
            let (closure, base, mut ip) = {
                let f = self.frames.last().unwrap();
                (f.closure.clone(), f.base, f.ip)
            };
            // Verified once per prototype: afterwards every register operand
            // is known to be below `max_registers`, and with the stack sized
            // to cover the frame's window the register accessors below can
            // skip bounds checks. Nothing shrinks the stack while a frame is
            // live: callers restore it to at least its earlier length.
            if !closure.proto.is_verified() {
                crate::bytecode::verify_proto(&closure.proto).map_err(|e| e.to_string())?;
                closure.proto.mark_verified();
            }
            let window_end = base + closure.proto.max_registers as usize;
            if self.stack.len() < window_end {
                self.stack.resize(window_end, Value::Nil);
            }
            let code: &[Instruction] = &closure.proto.instructions;
            let consts: &[Value] = closure.proto.values();

            // Register access relative to this frame. `reg!` reads, `set!` writes.
            macro_rules! reg {
                ($r:expr) => {
                    *slot(&self.stack, base + $r as usize)
                };
            }
            macro_rules! set {
                ($r:expr, $v:expr) => {{
                    let v = $v;
                    *slot_mut(&mut self.stack, base + $r as usize) = v;
                }};
            }
            macro_rules! sync_ip {
                () => {
                    self.frames.last_mut().unwrap().ip = ip;
                };
            }
            macro_rules! jump {
                ($off:expr) => {{
                    ip = (ip as isize + $off as isize) as usize;
                }};
            }
            macro_rules! throw {
                ($($arg:tt)*) => {{
                    sync_ip!();
                    return Err(format!($($arg)*));
                }};
            }
            // Runs a fallible expression, syncing the ip before propagating
            // an error so tracebacks point at the failing instruction.
            macro_rules! attempt {
                ($e:expr) => {
                    match $e {
                        Ok(v) => v,
                        Err(e) => {
                            sync_ip!();
                            return Err(e);
                        }
                    }
                };
            }
            // Binary arithmetic: `i64` fast path, then floats, then the
            // generic evaluator (bignums, mixed types, errors); a table
            // operand goes through its metamethod.
            macro_rules! arith {
                ($dst:expr, $a:expr, $b:expr, $checked:ident, $fop:tt, $slow:path, $event:literal) => {{
                    let ra = &reg!($a);
                    let rb = &reg!($b);
                    let res = match (ra, rb) {
                        (Value::Int(x), Value::Int(y)) => match x.$checked(*y) {
                            Some(r) => Value::Int(r),
                            None => attempt!($slow(ra, rb)),
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x $fop y),
                        (Value::Table(_), _) | (_, Value::Table(_)) => {
                            if let Some(mm) = self.get_binop_metamethod(ra, rb, $event) {
                                let (va, vb) = (ra.clone(), rb.clone());
                                sync_ip!();
                                self.call_binop_mm(mm, va, vb, base, $dst)?;
                                continue;
                            }
                            attempt!($slow(ra, rb))
                        }
                        _ => attempt!($slow(ra, rb)),
                    };
                    set!($dst, res);
                }};
            }
            // Arithmetic without an inline fast path (division, modulo,
            // power): straight to the evaluator, metamethods first.
            macro_rules! arith_slow {
                ($dst:expr, $a:expr, $b:expr, $slow:path, $event:literal) => {{
                    let ra = &reg!($a);
                    let rb = &reg!($b);
                    if matches!(ra, Value::Table(_)) || matches!(rb, Value::Table(_)) {
                        if let Some(mm) = self.get_binop_metamethod(ra, rb, $event) {
                            let (va, vb) = (ra.clone(), rb.clone());
                            sync_ip!();
                            self.call_binop_mm(mm, va, vb, base, $dst)?;
                            continue;
                        }
                    }
                    let res = attempt!($slow(ra, rb));
                    set!($dst, res);
                }};
            }
            macro_rules! bitop {
                ($dst:expr, $a:expr, $b:expr, $slow:path) => {{
                    let res = attempt!($slow(&reg!($a), &reg!($b)));
                    set!($dst, res);
                }};
            }
            // Ordered comparison with a conditional jump. `$mm_swap` is true
            // for `>`/`>=`, which are `<`/`<=` with the operands swapped.
            macro_rules! compare {
                ($a:expr, $b:expr, $jump:expr, $op:tt, $slow:path, $event:literal, $mm_swap:expr) => {{
                    let ra = &reg!($a);
                    let rb = &reg!($b);
                    let holds = match (ra, rb) {
                        (Value::Int(x), Value::Int(y)) => x $op y,
                        (Value::Float(x), Value::Float(y)) => x $op y,
                        (Value::Table(_), _) | (_, Value::Table(_)) => {
                            let (l, r) = if $mm_swap { (rb, ra) } else { (ra, rb) };
                            if let Some(mm) = self.get_binop_metamethod(l, r, $event) {
                                let (vl, vr) = (l.clone(), r.clone());
                                sync_ip!();
                                self.call_cmp_mm(mm, vl, vr)?
                            } else {
                                attempt!($slow(ra, rb))
                            }
                        }
                        _ => attempt!($slow(ra, rb)),
                    };
                    if !holds {
                        jump!($jump);
                    }
                }};
            }

            loop {
                let Some(&inst) = code.get(ip) else {
                    // Fell off the end without a `Return`: treat as returning nothing.
                    sync_ip!();
                    if let Err(err) = self.fire_hook(HookEvent::Return) {
                        return Err(err);
                    }
                    self.frames.pop();
                    continue 'frames;
                };
                ip += 1;

                match inst {
                    Instruction::LoadNil { dst } => set!(dst, Value::Nil),
                    Instruction::LoadBool { dst, val } => set!(dst, Value::Bool(val)),
                    Instruction::LoadInt { dst, val } => set!(dst, Value::Int(val as i64)),
                    Instruction::LoadK { dst, k } => set!(dst, consts[k as usize].clone()),
                    Instruction::Move { dst, src } => {
                        let v = reg!(src).clone();
                        set!(dst, v);
                    }
                    Instruction::GetGlobal { dst, name_k } => {
                        let Value::String(name) = &consts[name_k as usize] else {
                            throw!("global name must be string");
                        };
                        let val = self.globals.get(&**name).cloned().unwrap_or(Value::Nil);
                        set!(dst, val);
                    }
                    Instruction::SetGlobal { src, name_k } => {
                        let Value::String(name) = &consts[name_k as usize] else {
                            throw!("global name must be string");
                        };
                        let val = reg!(src).clone();
                        // Re-inserting an existing key would drop the old Rc for
                        // an identical one; a lookup first avoids that churn.
                        if let Some(slot) = self.globals.get_mut(&**name) {
                            *slot = val;
                        } else {
                            self.globals.insert(name.clone(), val);
                        }
                    }
                    Instruction::GetUpval { dst, upval_idx } => {
                        let val = self.upvalue_get(&closure.upvalues[upval_idx as usize]);
                        set!(dst, val);
                    }
                    Instruction::SetUpval { src, upval_idx } => {
                        let val = reg!(src).clone();
                        self.upvalue_set(&closure.upvalues[upval_idx as usize], val);
                    }
                    Instruction::NewTable { dst } => {
                        if self.gc.should_collect() {
                            if let Err(err) = self.gc_collect() {
                                sync_ip!();
                                return Err(err);
                            }
                        }
                        let rc = Rc::new(RefCell::new(VmTable::new()));
                        self.gc.register_table(&rc);
                        set!(dst, Value::Table(rc));
                    }
                    Instruction::GetTable { dst, table, key } => {
                        let tbl = &reg!(table);
                        let k = &reg!(key);
                        let val = match Self::table_get_fast(tbl, k) {
                            Some(v) => v,
                            None => {
                                let (tbl, k) = (tbl.clone(), k.clone());
                                sync_ip!();
                                self.table_get(&tbl, &k)?
                            }
                        };
                        set!(dst, val);
                    }
                    Instruction::GetTableK { dst, table, key_k } => {
                        let tbl = &reg!(table);
                        let k = &consts[key_k as usize];
                        let val = match Self::table_get_fast(tbl, k) {
                            Some(v) => v,
                            None => {
                                let tbl = tbl.clone();
                                sync_ip!();
                                self.table_get(&tbl, k)?
                            }
                        };
                        set!(dst, val);
                    }
                    Instruction::SetTable { table, key, val } => {
                        let v = reg!(val).clone();
                        if !Self::table_set_fast(&reg!(table), &reg!(key), &v) {
                            let (tbl, k) = (reg!(table).clone(), reg!(key).clone());
                            sync_ip!();
                            self.table_set(&tbl, k, v)?;
                        }
                    }
                    Instruction::SetTableK { table, key_k, val } => {
                        let v = reg!(val).clone();
                        let k = &consts[key_k as usize];
                        if !Self::table_set_fast(&reg!(table), k, &v) {
                            let tbl = reg!(table).clone();
                            sync_ip!();
                            self.table_set(&tbl, k.clone(), v)?;
                        }
                    }
                    Instruction::AppendArray { table, src } => {
                        let val = reg!(src).clone();
                        if let Value::Table(t) = &reg!(table) {
                            t.borrow_mut().array.push(val);
                        } else {
                            throw!("cannot append to non-table");
                        }
                    }
                    Instruction::SetList {
                        table,
                        base: list,
                        count,
                    } => {
                        let start = base + list as usize;
                        let end = if count == MULTRET {
                            self.top.max(start)
                        } else {
                            start + count as usize
                        };
                        let end = end.min(self.stack.len());
                        if let Value::Table(t) = &reg!(table) {
                            let t = t.clone();
                            let mut tbl = t.borrow_mut();
                            tbl.array.reserve(end.saturating_sub(start));
                            tbl.array.extend(self.stack[start..end].iter().cloned());
                        } else {
                            throw!("cannot SetList to non-table");
                        }
                    }
                    Instruction::TForCall { base: b, retc } => {
                        sync_ip!();
                        self.tfor_call(base, b, retc)?;
                    }
                    Instruction::TForLoop { base: b, jump } => {
                        let first_var = reg!(b + 3).clone();
                        if matches!(first_var, Value::Nil) {
                            // Exit loop: jump forward past the back-jump
                            jump!(jump);
                        } else {
                            // Update ctrl to first result, continue
                            set!(b + 2, first_var);
                        }
                    }
                    Instruction::Add { dst, a, b } => {
                        arith!(dst, a, b, checked_add, +, ops::eval_add, "__add")
                    }
                    Instruction::Sub { dst, a, b } => {
                        arith!(dst, a, b, checked_sub, -, ops::eval_sub, "__sub")
                    }
                    Instruction::Mul { dst, a, b } => {
                        arith!(dst, a, b, checked_mul, *, ops::eval_mul, "__mul")
                    }
                    Instruction::Div { dst, a, b } => {
                        arith_slow!(dst, a, b, ops::eval_div, "__div")
                    }
                    Instruction::IDiv { dst, a, b } => {
                        // Fast path for small ints; the zero divisor,
                        // MIN/-1 overflow, floats, BigInts and metamethods
                        // keep going through the generic evaluator so
                        // semantics cannot drift.
                        let ra = &reg!(a);
                        let rb = &reg!(b);
                        let res = match (ra, rb) {
                            (Value::Int(x), Value::Int(y)) => {
                                if *y == 0 || (*x == i64::MIN && *y == -1) {
                                    attempt!(ops::eval_idiv(ra, rb))
                                } else {
                                    Value::Int(x.div_floor(y))
                                }
                            }
                            _ => attempt!(ops::eval_idiv(ra, rb)),
                        };
                        set!(dst, res);
                    }
                    Instruction::Mod { dst, a, b } => {
                        // Fast path for small ints; the zero divisor,
                        // MIN/-1 overflow, floats, BigInts and metamethods
                        // keep going through the generic evaluator so
                        // semantics cannot drift.
                        let ra = &reg!(a);
                        let rb = &reg!(b);
                        let res = match (ra, rb) {
                            (Value::Int(x), Value::Int(y)) => {
                                if *y == 0 {
                                    attempt!(ops::eval_mod(ra, rb))
                                } else if *y == -1 {
                                    Value::Int(0)
                                } else {
                                    Value::Int(x.mod_floor(y))
                                }
                            }
                            _ => attempt!(ops::eval_mod(ra, rb)),
                        };
                        set!(dst, res);
                    }
                    Instruction::Pow { dst, a, b } => {
                        arith_slow!(dst, a, b, ops::eval_pow, "__pow")
                    }
                    Instruction::BitAnd { dst, a, b } => {
                        let ra = &reg!(a);
                        let rb = &reg!(b);
                        let res = match (ra, rb) {
                            (Value::Int(x), Value::Int(y)) => Value::Int(x & y),
                            _ => attempt!(ops::eval_bitand(ra, rb)),
                        };
                        set!(dst, res);
                    }
                    Instruction::BitOr { dst, a, b } => {
                        let ra = &reg!(a);
                        let rb = &reg!(b);
                        let res = match (ra, rb) {
                            (Value::Int(x), Value::Int(y)) => Value::Int(x | y),
                            _ => attempt!(ops::eval_bitor(ra, rb)),
                        };
                        set!(dst, res);
                    }
                    Instruction::BitXor { dst, a, b } => {
                        let ra = &reg!(a);
                        let rb = &reg!(b);
                        let res = match (ra, rb) {
                            (Value::Int(x), Value::Int(y)) => Value::Int(x ^ y),
                            _ => attempt!(ops::eval_bitxor(ra, rb)),
                        };
                        set!(dst, res);
                    }
                    Instruction::Shl { dst, a, b } => {
                        // Only shifts that provably keep every bit stay
                        // inline; the headroom rule mirrors `shift_left`
                        // exactly and everything else (negatives, widening,
                        // floats) delegates to it.
                        let ra = &reg!(a);
                        let rb = &reg!(b);
                        let res = match (ra, rb) {
                            (Value::Int(x), Value::Int(y))
                                if *x >= 0
                                    && *y >= 0
                                    && (*y as u64)
                                        <= x.leading_zeros().saturating_sub(1) as u64 =>
                            {
                                Value::Int(x << *y)
                            }
                            _ => attempt!(ops::eval_shl(ra, rb)),
                        };
                        set!(dst, res);
                    }
                    Instruction::Shr { dst, a, b } => {
                        // Fast path for the overwhelmingly common
                        // small-int shift; every other operand shape keeps
                        // going through the generic evaluator so its
                        // semantics (widening, floats, errors) cannot drift.
                        let ra = &reg!(a);
                        let rb = &reg!(b);
                        let res = match (ra, rb) {
                            (Value::Int(x), Value::Int(y)) => {
                                if *y < 0 {
                                    attempt!(ops::eval_shl(ra, rb))
                                } else if *y >= 64 {
                                    Value::Int(if *x < 0 { -1 } else { 0 })
                                } else {
                                    Value::Int(x >> *y)
                                }
                            }
                            _ => attempt!(ops::eval_shr(ra, rb)),
                        };
                        set!(dst, res);
                    }
                    Instruction::LShl { dst, a, b } => bitop!(dst, a, b, ops::eval_lshl),
                    Instruction::LShr { dst, a, b } => bitop!(dst, a, b, ops::eval_lshr),
                    Instruction::Concat { dst, a, b } => {
                        let ra = &reg!(a);
                        let rb = &reg!(b);
                        let res = match (ra, rb) {
                            (Value::String(x), Value::String(y)) => {
                                let mut s = String::with_capacity(x.len() + y.len());
                                s.push_str(x);
                                s.push_str(y);
                                Value::string(s)
                            }
                            (Value::Table(_), _) | (_, Value::Table(_))
                                if self.get_binop_metamethod(ra, rb, "__concat").is_some() =>
                            {
                                let mm = self.get_binop_metamethod(ra, rb, "__concat").unwrap();
                                let (va, vb) = (ra.clone(), rb.clone());
                                sync_ip!();
                                self.call_binop_mm(mm, va, vb, base, dst)?;
                                continue;
                            }
                            _ => Value::string(format!("{}{}", ra, rb)),
                        };
                        set!(dst, res);
                    }
                    Instruction::Unm { dst, src } => {
                        let v = &reg!(src);
                        let res = match v {
                            Value::Int(i) if *i != i64::MIN => Value::Int(-i),
                            Value::Float(f) => Value::Float(-f),
                            Value::Table(_) => {
                                if let Some(mm) = self.get_unop_metamethod(v, "__unm") {
                                    let v = v.clone();
                                    sync_ip!();
                                    let res = self.call_function(mm, &[v])?;
                                    set!(dst, res.into_iter().next().unwrap_or(Value::Nil));
                                    continue;
                                }
                                throw!("unary minus expects a number");
                            }
                            _ => attempt!(ops::eval_unm(v)),
                        };
                        set!(dst, res);
                    }
                    Instruction::Not { dst, src } => {
                        let truthy = reg!(src).is_truthy();
                        set!(dst, Value::Bool(!truthy));
                    }
                    Instruction::Len { dst, src } => {
                        let v = &reg!(src);
                        let len = match v {
                            Value::String(s) => s.len(),
                            Value::Table(t) => {
                                if let Some(mm) = self.get_unop_metamethod(v, "__len") {
                                    let v = v.clone();
                                    sync_ip!();
                                    let res = self.call_function(mm, &[v])?;
                                    set!(dst, res.into_iter().next().unwrap_or(Value::Nil));
                                    continue;
                                }
                                t.borrow().array.len()
                            }
                            Value::Buffer(b) => b.borrow().len(),
                            _ => throw!("len expects string, table or buffer"),
                        };
                        set!(dst, Value::from_usize(len));
                    }
                    Instruction::BitNot { dst, src } => {
                        let res = attempt!(ops::eval_bitnot(&reg!(src)));
                        set!(dst, res);
                    }
                    Instruction::Coalesce { dst, a, b } => {
                        let va = &reg!(a);
                        let res = if matches!(va, Value::Nil) {
                            reg!(b).clone()
                        } else {
                            va.clone()
                        };
                        set!(dst, res);
                    }
                    Instruction::Eq {
                        a,
                        b,
                        jump_if_false,
                    } => {
                        let is_eq = self.values_equal(base, a, b, ip)?;
                        if !is_eq {
                            jump!(jump_if_false);
                        }
                    }
                    Instruction::Ne {
                        a,
                        b,
                        jump_if_false,
                    } => {
                        let is_eq = self.values_equal(base, a, b, ip)?;
                        if is_eq {
                            jump!(jump_if_false);
                        }
                    }
                    Instruction::Lt {
                        a,
                        b,
                        jump_if_false,
                    } => {
                        compare!(a, b, jump_if_false, <, ops::eval_lt, "__lt", false)
                    }
                    Instruction::Le {
                        a,
                        b,
                        jump_if_false,
                    } => {
                        compare!(a, b, jump_if_false, <=, ops::eval_le, "__le", false)
                    }
                    Instruction::Gt {
                        a,
                        b,
                        jump_if_false,
                    } => {
                        compare!(a, b, jump_if_false, >, ops::eval_gt, "__lt", true)
                    }
                    Instruction::Ge {
                        a,
                        b,
                        jump_if_false,
                    } => {
                        compare!(a, b, jump_if_false, >=, ops::eval_ge, "__le", true)
                    }
                    Instruction::Test { reg, jump_if_false } => {
                        if !reg!(reg).is_truthy() {
                            jump!(jump_if_false);
                        }
                    }
                    Instruction::Jump { offset } => jump!(offset),
                    Instruction::ForPrep { base: b, jump } => {
                        let step = &reg!(b + 2);
                        match step {
                            Value::Int(0) => throw!("numeric for step cannot be zero"),
                            Value::Float(f) if *f == 0.0 => {
                                throw!("numeric for step cannot be zero")
                            }
                            _ => {}
                        }
                        let init_minus_step = attempt!(ops::eval_sub(&reg!(b), step));
                        set!(b, init_minus_step);
                        jump!(jump);
                    }
                    Instruction::ForLoop { base: b, jump } => {
                        let idx = &reg!(b);
                        let limit = &reg!(b + 1);
                        let step = &reg!(b + 2);
                        // All-integer loops are the common case and need no
                        // allocation or generic dispatch at all.
                        if let (Value::Int(i), Value::Int(lim), Value::Int(st)) = (idx, limit, step)
                            && let Some(next) = i.checked_add(*st)
                        {
                            let again = if *st >= 0 { next <= *lim } else { next >= *lim };
                            set!(b, Value::Int(next));
                            if again {
                                set!(b + 3, Value::Int(next));
                                jump!(jump);
                            }
                            continue;
                        }
                        match step {
                            Value::Int(0) => throw!("numeric for step cannot be zero"),
                            Value::Float(f) if *f == 0.0 => {
                                throw!("numeric for step cannot be zero")
                            }
                            _ => {}
                        }
                        let is_positive = match step {
                            Value::Int(i) => *i >= 0,
                            Value::BigInt(i) => num_traits::Signed::is_positive(&**i),
                            Value::Float(f) => *f >= 0.0,
                            _ => true,
                        };
                        let next_idx = attempt!(ops::eval_add(idx, step));
                        let loop_again = if is_positive {
                            attempt!(ops::eval_le(&next_idx, limit))
                        } else {
                            attempt!(ops::eval_ge(&next_idx, limit))
                        };
                        set!(b, next_idx.clone());
                        if loop_again {
                            set!(b + 3, next_idx);
                            jump!(jump);
                        }
                    }
                    Instruction::Closure { dst, proto_idx } => {
                        let child_proto = closure.proto.protos[proto_idx as usize].clone();
                        let mut upvalues = Vec::with_capacity(child_proto.upvalues.len());
                        for updesc in &child_proto.upvalues {
                            if updesc.in_stack {
                                upvalues.push(self.capture_upvalue(base + updesc.index as usize));
                            } else {
                                let up = closure
                                    .upvalues
                                    .get(updesc.index as usize)
                                    .cloned()
                                    .unwrap_or_else(|| Upvalue::closed(Value::Nil));
                                upvalues.push(up);
                            }
                        }
                        let new_closure = Value::Closure(Rc::new(VmClosure {
                            proto: child_proto,
                            upvalues,
                        }));
                        set!(dst, new_closure);
                    }
                    Instruction::Call { callee, argc, retc } => {
                        let args_start = base + callee as usize + 1;
                        // MULTRET args run from the first argument register up to
                        // whatever the previous instruction left on the stack.
                        let args_end = if argc == MULTRET {
                            self.top.max(args_start)
                        } else {
                            args_start + argc as usize
                        };
                        let argc = args_end - args_start;
                        if args_end > self.stack.len() {
                            self.stack.resize(args_end, Value::Nil);
                        }
                        sync_ip!();

                        match &reg!(callee) {
                            Value::Closure(callee_closure) => {
                                // Tail call: this `Call` is immediately
                                // followed by `Return` of its own results, so
                                // the current frame is replaced instead of
                                // pushing a new one, giving proper tail calls
                                // constant stack space. Only the exact shape
                                // the compilers emit for `return f(...)`
                                // qualifies (MULTRET on both sides with the
                                // results starting at the callee slot): any
                                // extra value would need an instruction
                                // between the two, which fails the adjacency
                                // check. A frame with no caller (base 0)
                                // cannot be replaced.
                                let tail = retc == MULTRET
                                    && base >= 1
                                    && matches!(
                                        code.get(ip),
                                        Some(Instruction::Return {
                                            base: ret,
                                            count: MULTRET,
                                        }) if *ret == callee
                                    );
                                if !tail && self.frames.len() >= MAX_CALL_DEPTH {
                                    throw!(
                                        "call stack overflow: exceeded maximum call depth of {}",
                                        MAX_CALL_DEPTH
                                    );
                                }
                                let callee_closure = callee_closure.clone();
                                let proto = &callee_closure.proto;
                                // For a tail call the callee window slides
                                // down one slot so it sits exactly where a
                                // call from our caller would put it, and the
                                // callee inherits our return obligation.
                                let (new_base, want_ret) = if tail {
                                    let cur = self.frames.pop().unwrap();
                                    if !self.open_upvalues.is_empty() {
                                        self.close_upvalues(cur.base);
                                    }
                                    // Relocate the whole callee window so the
                                    // callee sits where our caller put us
                                    // (`cur.base - 1`) and the arguments
                                    // follow; the new frame keeps our base so
                                    // its results land in our caller's slot.
                                    let caller_dest = cur.base - 1;
                                    let callee_abs = cur.base + callee as usize;
                                    let shift = callee_abs - caller_dest;
                                    let len = args_end - callee_abs;
                                    for k in 0..len {
                                        let v = std::mem::replace(
                                            &mut self.stack[callee_abs + k],
                                            Value::Nil,
                                        );
                                        self.stack[caller_dest + k] = v;
                                    }
                                    if self.top > callee_abs {
                                        self.top -= shift;
                                    }
                                    (cur.base, cur.want_ret)
                                } else {
                                    (args_start, retc)
                                };
                                let needed = new_base + proto.max_registers as usize;
                                if needed >= self.stack.len() {
                                    self.stack.resize(needed + 1, Value::Nil);
                                }
                                let num_params = proto.num_params as usize;
                                for i in argc..num_params {
                                    self.stack[new_base + i] = Value::Nil;
                                }
                                let varargs = if proto.is_vararg && argc > num_params {
                                    self.stack[new_base + num_params..new_base + argc].to_vec()
                                } else {
                                    Vec::new()
                                };
                                let new_frame =
                                    CallFrame::with_varargs(callee_closure, new_base, varargs)
                                        .wanting(want_ret);
                                self.frames.push(new_frame);
                                // A replaced frame never returns, so a tail
                                // call reports "tail call" instead of "call".
                                let event = if tail {
                                    HookEvent::TailCall
                                } else {
                                    HookEvent::Call
                                };
                                if let Err(err) = self.fire_hook(event) {
                                    self.frames.pop();
                                    sync_ip!();
                                    return Err(err);
                                }
                                continue 'frames;
                            }
                            Value::Native(def) => {
                                let func = def.func;
                                // Natives take a slice of owned values; copying
                                // the argument window is cheaper than exposing
                                // the stack to code that may resize it.
                                let args: Vec<Value> = self.stack[args_start..args_end].to_vec();
                                if let Err(err) = self.fire_hook(HookEvent::Call) {
                                    sync_ip!();
                                    return Err(err);
                                }
                                match func(self, &args) {
                                    Ok(results) => {
                                        if let Err(err) = self.fire_hook(HookEvent::Return) {
                                            sync_ip!();
                                            return Err(err);
                                        }
                                        self.store_call_results(callee, retc, &results);
                                    }
                                    Err(err) if self.is_yielding => {
                                        if let Some(co_id) = self.current_co
                                            && let Some(co_rc) = self.coroutines.get(&co_id)
                                        {
                                            let mut cs = co_rc.borrow_mut();
                                            cs.yield_callee = callee;
                                            cs.yield_retc = retc;
                                        }
                                        return Err(err);
                                    }
                                    Err(err) => return Err(err),
                                }
                            }
                            Value::Table(t) => {
                                let handler = t.borrow().metamethod("__call");
                                let callee_val = reg!(callee).clone();
                                if let Some(handler) = handler {
                                    let mut call_args = Vec::with_capacity(argc + 1);
                                    call_args.push(callee_val);
                                    call_args.extend_from_slice(&self.stack[args_start..args_end]);
                                    let results = self.call_function(handler, &call_args)?;
                                    self.store_call_results(callee, retc, &results);
                                } else {
                                    throw!("attempted to call non-function ({:?})", callee_val);
                                }
                            }
                            other => {
                                throw!("attempted to call non-function ({:?})", other);
                            }
                        }
                    }
                    Instruction::Return { base: ret, count } => {
                        sync_ip!();
                        if let Err(err) = self.fire_hook(HookEvent::Return) {
                            return Err(err);
                        }
                        let frame = self.frames.pop().unwrap();
                        if !self.open_upvalues.is_empty() {
                            self.close_upvalues(frame.base);
                        }
                        let ret_start = frame.base + ret as usize;
                        let ret_end = if count == MULTRET {
                            self.top.max(ret_start)
                        } else {
                            ret_start + count as usize
                        };
                        let ret_end = ret_end.min(self.stack.len());
                        let nret = ret_end.saturating_sub(ret_start);

                        if self.frames.len() == target_depth {
                            self.returned.clear();
                            self.returned
                                .extend(self.stack[ret_start..ret_end].iter().cloned());
                            let top_val = self.returned.first().cloned().unwrap_or(Value::Nil);
                            return Ok(top_val);
                        }
                        // The results land where the caller's `Call` put the callee.
                        let caller_dest = frame.base - 1;
                        let wanted = match frame.want_ret {
                            MULTRET => nret,
                            0 => 1,
                            n => n as usize,
                        };
                        let needed = caller_dest + wanted;
                        if needed > self.stack.len() {
                            self.stack.resize(needed, Value::Nil);
                        }
                        // Values move down the stack in order; the source and
                        // destination windows may overlap, which is fine when
                        // copying front to back since caller_dest < ret_start.
                        for i in 0..wanted {
                            let v = if i < nret {
                                std::mem::replace(&mut self.stack[ret_start + i], Value::Nil)
                            } else {
                                // Fewer values than asked for pads with nil rather
                                // than leaving whatever the register happened to hold.
                                Value::Nil
                            };
                            self.stack[caller_dest + i] = v;
                        }
                        if frame.want_ret == MULTRET {
                            self.top = caller_dest + nret;
                        }
                        continue 'frames;
                    }
                    Instruction::Vararg { dst, count } => {
                        let all = count == 0 || count == MULTRET;
                        let frame = self.frames.last().unwrap();
                        let cnt = if all {
                            frame.varargs.len()
                        } else {
                            count as usize
                        };
                        let dest = base + dst as usize;
                        if dest + cnt > self.stack.len() {
                            self.stack.resize(dest + cnt, Value::Nil);
                        }
                        let frame = self.frames.last().unwrap();
                        for i in 0..cnt {
                            let val = frame.varargs.get(i).cloned().unwrap_or(Value::Nil);
                            self.stack[dest + i] = val;
                        }
                        if all {
                            self.top = dest + cnt;
                        }
                    }
                }
            }
        }
    }

    /// `==` with the `__eq` metamethod fallback for two tables.
    #[inline]
    fn values_equal(&mut self, base: usize, a: u8, b: u8, ip: usize) -> Result<bool, String> {
        let va = &self.stack[base + a as usize];
        let vb = &self.stack[base + b as usize];
        if va == vb {
            return Ok(true);
        }
        if let (Value::Table(_), Value::Table(_)) = (va, vb)
            && let Some(mm) = self.get_binop_metamethod(va, vb, "__eq")
        {
            let (va, vb) = (va.clone(), vb.clone());
            self.frames.last_mut().unwrap().ip = ip;
            return self.call_cmp_mm(mm, va, vb);
        }
        Ok(false)
    }

    /// The generic-for step. `b` is the loop's base register: iterator,
    /// state, control, then the loop variables.
    fn tfor_call(&mut self, base: usize, b: u8, retc: u8) -> Result<(), String> {
        let iter_fn = self.stack[base + b as usize].clone();
        let state = self.stack[base + b as usize + 1].clone();
        let ctrl = self.stack[base + b as usize + 2].clone();
        let results = match &iter_fn {
            Value::Table(t) => {
                let has_call = t.borrow().metamethod("__call");
                if has_call.is_some() {
                    self.call_function(iter_fn, &[state, ctrl])?
                } else {
                    let tbl = t.borrow();
                    if retc > 1 && !tbl.array.is_empty() {
                        let next_idx = match ctrl {
                            Value::Nil => 0,
                            Value::Int(i) => usize::try_from(i).unwrap_or(tbl.array.len()),
                            _ => tbl.array.len(),
                        };
                        if next_idx < tbl.array.len() {
                            vec![Value::from_usize(next_idx + 1), tbl.array[next_idx].clone()]
                        } else {
                            vec![Value::Nil, Value::Nil]
                        }
                    } else if retc > 1 {
                        let mut keys: Vec<&StrRef> = tbl.fields.keys().collect();
                        keys.sort();
                        let next_key = match &ctrl {
                            Value::Nil => keys.first().copied(),
                            Value::String(prev_k) => keys
                                .iter()
                                .position(|k| ***k == **prev_k)
                                .and_then(|pos| keys.get(pos + 1).copied()),
                            _ => None,
                        };
                        if let Some(k) = next_key {
                            let v = tbl.fields.get(k).cloned().unwrap_or(Value::Nil);
                            vec![Value::String(k.clone()), v]
                        } else {
                            vec![Value::Nil, Value::Nil]
                        }
                    } else {
                        let next_idx = match state {
                            Value::Nil => 0,
                            Value::Int(i) => usize::try_from(i).unwrap_or(tbl.array.len()),
                            _ => tbl.array.len(),
                        };
                        if next_idx < tbl.array.len() {
                            let val = tbl.array[next_idx].clone();
                            drop(tbl);
                            self.stack[base + b as usize + 1] = Value::from_usize(next_idx + 1);
                            vec![val]
                        } else {
                            vec![Value::Nil]
                        }
                    }
                }
            }
            _ => self.call_function(iter_fn, &[state, ctrl])?,
        };
        let var_base = base + b as usize + 3;
        if var_base + retc as usize > self.stack.len() {
            self.stack.resize(var_base + retc as usize, Value::Nil);
        }
        for i in 0..(retc as usize) {
            self.stack[var_base + i] = results.get(i).cloned().unwrap_or(Value::Nil);
        }
        Ok(())
    }

    // ----- tables ---------------------------------------------------------

    /// Raw lookup of an existing, non-nil entry. `None` means the slow path
    /// (metamethods, buffers, errors) has to decide.
    #[inline]
    fn table_get_fast(table: &Value, key: &Value) -> Option<Value> {
        let Value::Table(t) = table else {
            return None;
        };
        let tbl = t.borrow();
        let found = match key {
            Value::Int(i) if *i >= 1 => tbl.array.get(*i as usize - 1),
            Value::String(k) => tbl.fields.get(&**k),
            _ => None,
        };
        match found {
            Some(Value::Nil) | None => {
                if tbl.metatable.is_none() {
                    // Nothing can intercept a miss: the answer is nil, unless
                    // the key is one a table can never hold.
                    match key {
                        Value::String(_) => Some(Value::Nil),
                        Value::Int(i) if *i >= 1 => Some(Value::Nil),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            Some(v) => Some(v.clone()),
        }
    }

    /// Raw store into an existing array slot or field, or an append, on a
    /// table that nothing can intercept. Returns false to defer to the slow
    /// path, which also reports errors.
    #[inline]
    fn table_set_fast(table: &Value, key: &Value, val: &Value) -> bool {
        let Value::Table(t) = table else {
            return false;
        };
        let mut tbl = t.borrow_mut();
        if tbl.frozen {
            return false;
        }
        match key {
            Value::Int(i) if *i >= 1 => {
                let idx = *i as usize - 1;
                if idx < tbl.array.len() {
                    tbl.array[idx] = val.clone();
                    true
                } else if idx == tbl.array.len() && tbl.metatable.is_none() {
                    tbl.array.push(val.clone());
                    true
                } else {
                    false
                }
            }
            Value::String(k) => {
                if let Some(slot) = tbl.fields.get_mut(&**k) {
                    *slot = val.clone();
                    true
                } else if tbl.metatable.is_none() {
                    tbl.fields.insert(k.clone(), val.clone());
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn table_get(&mut self, table: &Value, key: &Value) -> Result<Value, String> {
        self.table_get_depth(table, key, 0)
    }

    fn table_get_depth(
        &mut self,
        table: &Value,
        key: &Value,
        depth: usize,
    ) -> Result<Value, String> {
        if depth > 100 {
            return Err("loop in gettable / __index chain".to_string());
        }
        match table {
            Value::Table(t) => {
                let direct_val = {
                    let tbl = t.borrow();
                    match key {
                        Value::String(k) => tbl.fields.get(&**k).cloned(),
                        Value::Int(idx) if *idx > 0 => tbl.array.get(*idx as usize - 1).cloned(),
                        Value::BigInt(b) if num_traits::Signed::is_positive(&**b) => {
                            return Err("table index too large".to_string());
                        }
                        _ => None,
                    }
                };

                if let Some(val) = direct_val
                    && !matches!(val, Value::Nil)
                {
                    return Ok(val);
                }

                // Check __index metamethod
                let index_handler = t.borrow().metamethod("__index");
                match index_handler {
                    Some(handler @ Value::Table(_)) => {
                        return self.table_get_depth(&handler, key, depth + 1);
                    }
                    Some(handler @ (Value::Native(_) | Value::Closure(_))) => {
                        let res = self.call_function(handler, &[table.clone(), key.clone()])?;
                        return Ok(res.into_iter().next().unwrap_or(Value::Nil));
                    }
                    _ => {}
                }
                match key {
                    Value::String(_) => Ok(Value::Nil),
                    Value::Int(idx) if *idx > 0 => Ok(Value::Nil),
                    _ => Err("invalid table key".to_string()),
                }
            }
            Value::Buffer(b) => {
                let buf = b.borrow();
                match key {
                    Value::Int(idx) => {
                        let i = usize::try_from(*idx)
                            .map_err(|_| "buffer index too large".to_string())?;
                        let val = buf.read_u8(i)?;
                        Ok(Value::Int(val as i64))
                    }
                    Value::BigInt(_) => Err("buffer index too large".to_string()),
                    _ => Ok(Value::Nil),
                }
            }
            _ => Err("attempted to index a non-table value".to_string()),
        }
    }

    pub fn table_set(&mut self, table: &Value, key: Value, val: Value) -> Result<(), String> {
        self.table_set_depth(table, key, val, 0)
    }

    fn table_set_depth(
        &mut self,
        table: &Value,
        key: Value,
        val: Value,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 100 {
            return Err("loop in settable / __newindex chain".to_string());
        }
        self.gc.write_barrier(table, &val);
        match table {
            Value::Table(t) => {
                if t.borrow().frozen {
                    return Err("attempted to mutate a frozen table".to_string());
                }

                let key_exists = match &key {
                    Value::String(k) => t.borrow().fields.contains_key(&**k),
                    Value::Int(idx) if *idx > 0 => (*idx as usize - 1) < t.borrow().array.len(),
                    Value::BigInt(b) if num_traits::Signed::is_positive(&**b) => {
                        return Err("table index too large".to_string());
                    }
                    _ => false,
                };

                if !key_exists {
                    let newindex_handler = t.borrow().metamethod("__newindex");
                    match newindex_handler {
                        Some(handler @ Value::Table(_)) => {
                            return self.table_set_depth(&handler, key, val, depth + 1);
                        }
                        Some(handler @ (Value::Native(_) | Value::Closure(_))) => {
                            self.call_function(handler, &[table.clone(), key, val])?;
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                let mut tbl = t.borrow_mut();
                match key {
                    Value::String(k) => {
                        tbl.fields.insert(k, val);
                    }
                    Value::Int(idx) if idx > 0 => {
                        let i = idx as usize;
                        if i - 1 < tbl.array.len() {
                            tbl.array[i - 1] = val;
                        } else if i - 1 == tbl.array.len() {
                            tbl.array.push(val);
                        } else {
                            tbl.array.resize(i - 1, Value::Nil);
                            tbl.array.push(val);
                        }
                    }
                    _ => return Err("invalid table key".to_string()),
                }
                Ok(())
            }
            Value::Buffer(b) => {
                let mut buf = b.borrow_mut();
                match key {
                    Value::Int(idx) => {
                        let i = usize::try_from(idx)
                            .map_err(|_| "buffer index too large".to_string())?;
                        let byte_val = match val {
                            Value::Int(v) => u8::try_from(v)
                                .map_err(|_| "value out of byte range".to_string())?,
                            Value::Float(f) => f as u8,
                            _ => return Err("buffer value expects byte".to_string()),
                        };
                        buf.write_u8(i, byte_val)?;
                        Ok(())
                    }
                    Value::BigInt(_) => Err("buffer index too large".to_string()),
                    _ => Err("invalid buffer index".to_string()),
                }
            }
            _ => Err("attempted to set field on a non-table value".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile_to_proto;
    use crate::parser::Parser;

    fn run_code(src: &str) -> Value {
        let mut parser = Parser::new(src);
        let (stmts, _pool) = parser.parse_program().expect("syntax error");
        let proto = compile_to_proto(&stmts);
        let mut vm = VM::new();
        vm.execute(proto).expect("execution error")
    }

    #[test]
    fn test_vm_arithmetic() {
        let res = run_code("local a = 10\nlocal b = 20\nreturn a + b * 2");
        assert_eq!(res.to_string(), "50");
    }

    #[test]
    fn test_vm_bitwise_operations() {
        let res =
            run_code("local a = 12 & 10\nlocal b = 12 | 10\nlocal c = 12 ~ 10\nreturn a + b + c");
        // 8 + 14 + 6 = 28
        assert_eq!(res.to_string(), "28");
    }

    #[test]
    fn test_vm_bitwise_not_and_shifts() {
        let res = run_code("local a = ~0\nlocal b = 1 << 4\nlocal c = 16 >> 2\nreturn a + b + c");
        // -1 + 16 + 4 = 19
        assert_eq!(res.to_string(), "19");
    }

    #[test]
    fn test_vm_logical_shifts() {
        let res = run_code("local a = -1 >>> 60\nlocal b = 1 <<< 4\nreturn a + b");
        // 15 + 16 = 31
        assert_eq!(res.to_string(), "31");
    }

    #[test]
    fn test_vm_shr_edge_cases() {
        // Negative value, oversized shift, and negative shift amount.
        let res = run_code("return (16 >> 2) + (1 >> 100) + (-8 >> 2) + (4 >> -1)");
        // 4 + 0 + -2 + 8 = 10
        assert_eq!(res.to_string(), "10");
    }

    #[test]
    fn test_vm_mod_edge_cases() {
        let res = run_code("return (17 % 5) + (-17 % 5) + (7.5 % 2)");
        // Floored modulo: 2 + 3 + 1.5 = 6.5 (verified against Lua 5.4).
        assert_eq!(res.to_string(), "6.5");
    }

    #[test]
    fn test_vm_mod_min_zero() {
        // i64::MIN % -1 is 0 mathematically, not an overflow panic.
        let res = run_code("return -9223372036854775808 % -1");
        assert_eq!(res.to_string(), "0");
    }

    #[test]
    fn test_vm_mod_zero_message() {
        let err = crate::vm::ops::eval_mod(
            &crate::vm::value::Value::Int(1),
            &crate::vm::value::Value::Int(0),
        )
        .unwrap_err();
        assert_eq!(err, "modulo by zero");
    }

    #[test]
    fn test_vm_idiv_edge_cases() {
        let res = run_code("return (17 // 5) + (-17 // 5) + (7.5 // 2)");
        // Floored division: 3 + -4 + 3.0 = 2.0, displayed as "2".
        assert_eq!(res.to_string(), "2");
    }

    #[test]
    fn test_vm_idiv_min_neg1_widens() {
        // i64::MIN // -1 overflows i64: must widen to BigInt, not wrap.
        let res = run_code("return -9223372036854775808 // -1");
        assert_eq!(res.to_string(), "9223372036854775808");
    }

    #[test]
    fn test_vm_idiv_zero_message() {
        let err = crate::vm::ops::eval_idiv(
            &crate::vm::value::Value::Int(1),
            &crate::vm::value::Value::Int(0),
        )
        .unwrap_err();
        assert_eq!(err, "division by zero");
    }

    #[test]
    fn test_vm_bitops_int_paths() {
        let res = run_code("return (12 & 10) + (12 | 10) + (12 ~ 10) + (1 << 4)");
        // 8 + 14 + 6 + 16 = 44
        assert_eq!(res.to_string(), "44");
    }

    #[test]
    fn test_vm_shl_widening_fallback() {
        // 1 << 100 widens past i64: must equal 2^100, not wrap.
        // 3 << 62 also widens (headroom rule); both go through fallback.
        let res = run_code(
            "return ((1 << 100) == 1267650600228229401496703205376) and ((3 << 62) == 13835058055282163712)",
        );
        assert_eq!(res.to_string(), "true");
    }

    #[test]
    fn test_vm_functions_and_calls() {
        let code = "function add(x, y) return x + y end\nreturn add(15, 27)";
        let res = run_code(code);
        assert_eq!(res.to_string(), "42");
    }

    #[test]
    fn test_vm_tail_call_deep_recursion() {
        let code = "local function loop(n, acc)\nif n == 0 then return acc end\nreturn loop(n - 1, acc + n)\nend\nreturn loop(100000, 0)";
        let res = run_code(code);
        assert_eq!(res.to_string(), "5000050000");
    }

    #[test]
    fn test_vm_if_and_loops() {
        let code =
            "local sum = 0\nlocal i = 1\nwhile i <= 5 do\n  sum = sum + i\n  i++\nend\nreturn sum";
        let res = run_code(code);
        assert_eq!(res.to_string(), "15");
    }

    #[test]
    fn test_vm_tables() {
        let code = "local t = {10, 20, 30}\nt[2] = 99\nreturn t[2]";
        let res = run_code(code);
        assert_eq!(res.to_string(), "99");
    }

    #[test]
    fn test_vm_bit_library() {
        let code = "local band = bit.band(15, 7)\nlocal bor = bit.bor(8, 2)\nlocal bxor = bit.bxor(10, 6)\nreturn band + bor + bxor";
        // 7 + 10 + 12 = 29
        let res = run_code(code);
        assert_eq!(res.to_string(), "29");
    }

    #[test]
    fn test_vm_buffer_library() {
        let code = "local b = buffer.create(16)\nbuffer.writeu8(b, 0, 42)\nbuffer.writei32(b, 4, 123456)\nlocal u = buffer.readu8(b, 0)\nlocal i = buffer.readi32(b, 4)\nreturn u + i";
        // 42 + 123456 = 123498
        let res = run_code(code);
        assert_eq!(res.to_string(), "123498");
    }

    #[test]
    fn test_vm_gc_library() {
        let code = "local kb = collectgarbage(\"count\")\nassert(kb >= 0)\nlocal running = collectgarbage(\"isrunning\")\nassert(running == true)\nreturn 1";
        let res = run_code(code);
        assert_eq!(res.to_string(), "1");
    }

    #[test]
    fn test_vm_math_library() {
        let code = "local a = math.abs(-15)\nlocal b = math.floor(3.9)\nlocal c = math.max(10, 25, 5)\nreturn a + b + c";
        // 15 + 3 + 25 = 43
        let res = run_code(code);
        assert_eq!(res.to_string(), "43");
    }

    #[test]
    fn test_vm_string_library() {
        let code = "local s = \"hello\"\nlocal upper = string.upper(s)\nlocal sub = string.sub(s, 1, 4)\nreturn upper .. \"_\" .. sub";
        let res = run_code(code);
        assert_eq!(res.to_string(), "HELLO_hell");
    }

    #[test]
    fn test_vm_table_library() {
        let code =
            "local t = {10, 20}\ntable.insert(t, 30)\nlocal s = table.concat(t, \",\")\nreturn s";
        let res = run_code(code);
        assert_eq!(res.to_string(), "10,20,30");
    }

    #[test]
    fn test_vm_pcall_and_error() {
        let code = "local ok, err = pcall(error, \"boom\")\nassert(ok == false)\nassert(err == \"boom\")\nreturn 1";
        let res = run_code(code);
        assert_eq!(res.to_string(), "1");
    }

    #[test]
    fn test_vm_and_or_short_circuit() {
        let code = "local a = false and 10\nlocal b = 5 or 20\nlocal c = false or 30\nlocal d = true and 40\nreturn b + c + d";
        // 5 + 30 + 40 = 75
        let res = run_code(code);
        assert_eq!(res.to_string(), "75");
    }

    #[test]
    fn test_vm_for_prep_and_for_loop() {
        let code = "local sum = 0\nfor i = 1, 5 do\n  sum = sum + i\nend\nreturn sum";
        let res = run_code(code);
        assert_eq!(res.to_string(), "15");

        let code_step = "local sum = 0\nfor i = 10, 2, -2 do\n  sum = sum + i\nend\nreturn sum";
        // 10 + 8 + 6 + 4 + 2 = 30
        let res_step = run_code(code_step);
        assert_eq!(res_step.to_string(), "30");
    }

    #[test]
    fn test_vm_setlist_table_construct() {
        let code = "local t = {10, 20, 30, 40, 50}\nreturn t[1] + t[3] + t[5]";
        // 10 + 30 + 50 = 90
        let res = run_code(code);
        assert_eq!(res.to_string(), "90");
    }

    #[test]
    fn test_vm_metatables_index_newindex() {
        let code = "local proto = { answer = 42 }\nlocal t = {}\nsetmetatable(t, { __index = proto })\nreturn t.answer";
        let res = run_code(code);
        assert_eq!(res.to_string(), "42");

        let code_fn = "local t = {}\nsetmetatable(t, { __index = function(tbl, key) return key .. \"_handled\" end })\nreturn t.foo";
        let res_fn = run_code(code_fn);
        assert_eq!(res_fn.to_string(), "foo_handled");

        let code_newindex = "local sink = {}\nlocal t = {}\nsetmetatable(t, { __newindex = sink })\nt.val = 123\nreturn sink.val";
        let res_newindex = run_code(code_newindex);
        assert_eq!(res_newindex.to_string(), "123");
    }

    #[test]
    fn test_vm_metatables_operators_and_call() {
        let code_add = "local mt = { __add = function(a, b) return a.val + b.val end }\nlocal a = { val = 15 }\nlocal b = { val = 27 }\nsetmetatable(a, mt)\nreturn a + b";
        let res_add = run_code(code_add);
        assert_eq!(res_add.to_string(), "42");

        let code_call = "local t = {}\nsetmetatable(t, { __call = function(tbl, x, y) return x * y end })\nreturn t(6, 7)";
        let res_call = run_code(code_call);
        assert_eq!(res_call.to_string(), "42");

        let code_tostring = "local t = { name = \"Neyuki\" }\nsetmetatable(t, { __tostring = function(x) return \"Hello \" .. x.name end })\nreturn tostring(t)";
        let res_tostring = run_code(code_tostring);
        assert_eq!(res_tostring.to_string(), "Hello Neyuki");
    }

    #[test]
    fn test_vm_raw_builtins() {
        let code = "local t = {}\nrawset(t, \"k\", 99)\nlocal a = rawget(t, \"k\")\nlocal eq = rawequal(t, t)\nassert(eq == true)\nreturn a";
        let res = run_code(code);
        assert_eq!(res.to_string(), "99");
    }

    #[test]
    fn test_vm_debug_hook_call_return() {
        // Verified against Lua 5.4: the sethook call itself reports its
        // return (hook is active by then), then add's pair, then the
        // clearing sethook reports its call (but no return: hook is off).
        let code = "local events = {}\nlocal function tracer(ev)\n  events[#events + 1] = ev\nend\ndebug.sethook(tracer, \"cr\")\nlocal function add(x, y)\n  return x + y\nend\nlocal s = add(1, 2)\ndebug.sethook(nil)\nreturn table.concat(events, \",\")";
        let res = run_code(code);
        assert_eq!(res.to_string(), "return,call,return,call");
    }

    #[test]
    fn test_vm_debug_gethook_roundtrip() {
        let code = "local function tracer(ev) end\ndebug.sethook(tracer, \"cr\")\nlocal h, m = debug.gethook()\nassert(h == tracer)\nassert(m == \"cr\")\ndebug.sethook(nil)\nlocal h2 = debug.gethook()\nassert(h2 == nil)\nreturn 1";
        let res = run_code(code);
        assert_eq!(res.to_string(), "1");
    }

    // Deferred to v3: per-instruction line numbers are not plumbed yet
    // (both compilers emit line 1; the parser's SpanPool is dropped in
    // `compile_source`). Needs spans threaded AST -> IR/bytecode emit.
    #[test]
    #[ignore]
    fn test_vm_debug_hook_line_events() {
        let code = "local lines = {}\nlocal function tracer(ev)\n  if ev == \"line\" then\n    local info = debug.getinfo(2)\n    lines[#lines + 1] = info.currentline\n  end\nend\ndebug.sethook(tracer, \"l\")\nlocal a = 1\nlocal b = 2\nlocal c = a + b\ndebug.sethook(nil)\nreturn table.concat(lines, \",\")";
        let res = run_code(code);
        assert_eq!(res.to_string(), "9,10,11,12");
    }

    #[test]
    fn test_vm_debug_hook_native_call() {
        // `tostring(i)` over a loop variable cannot be constant-folded, so
        // the native call really executes. Lua-consistent sequence:
        // sethook-return, two tostring pairs, clearing call.
        let code = "local events = {}\nlocal function tracer(ev)\n  events[#events + 1] = ev\nend\ndebug.sethook(tracer, \"cr\")\nlocal x = \"\"\nfor i = 1, 2 do x = tostring(i) end\ndebug.sethook(nil)\nassert(x == \"2\")\nreturn table.concat(events, \",\")";
        let res = run_code(code);
        assert_eq!(res.to_string(), "return,call,return,call,return,call");
    }

    #[test]
    fn test_vm_debug_hook_pcall_path() {
        // Lua 5.4 ground truth: sethook-return, pcall-call, add-call,
        // add-return, pcall-return, clearing call.
        let code = "local events = {}\nlocal function tracer(ev)\n  events[#events + 1] = ev\nend\nlocal function add(x, y)\n  return x + y\nend\ndebug.sethook(tracer, \"cr\")\nlocal ok, v = pcall(add, 1, 2)\ndebug.sethook(nil)\nassert(ok == true)\nassert(v == 3)\nreturn table.concat(events, \",\")";
        let res = run_code(code);
        assert_eq!(res.to_string(), "return,call,call,return,return,call");
    }

    #[test]
    fn test_vm_debug_hook_tail_call_event() {
        // Lua 5.4 ground truth: sethook-return, initial call, one "tail
        // call" per replaced frame, final return, clearing call.
        let code = "local events = {}\nlocal function tracer(ev)\n  events[#events + 1] = ev\nend\nlocal function loop(n, acc)\n  if n == 0 then return acc end\n  return loop(n - 1, acc + n)\nend\ndebug.sethook(tracer, \"cr\")\nlocal r = loop(3, 0)\ndebug.sethook(nil)\nassert(r == 6)\nreturn table.concat(events, \",\")";
        let res = run_code(code);
        assert_eq!(
            res.to_string(),
            "return,call,tail call,tail call,tail call,return,call"
        );
    }

    #[test]
    fn test_vm_require_bundled_libs() {
        let code_bit = "local bit = require(\"@neyuki/bit\")\nreturn bit.band(7, 3)";
        let res_bit = run_code(code_bit);
        assert_eq!(res_bit.to_string(), "3");

        let code_buf = "local buffer = require(\"@neyuki/buffer\")\nlocal b = buffer.create(8)\nbuffer.writeu8(b, 0, 99)\nreturn buffer.readu8(b, 0)";
        let res_buf = run_code(code_buf);
        assert_eq!(res_buf.to_string(), "99");

        let code_os =
            "local os = require(\"@neyuki/os\")\nlocal t = os.clock()\nassert(t >= 0)\nreturn 1";
        let res_os = run_code(code_os);
        assert_eq!(res_os.to_string(), "1");
    }

    #[test]
    fn test_vm_utf8_library() {
        let code = "local u = require(\"@neyuki/utf8\")\nlocal s = u.char(65, 66, 67)\nassert(u.len(s) == 3)\nassert(u.codepoint(s, 1, 1) == 65)\nreturn s";
        let res = run_code(code);
        assert_eq!(res.to_string(), "ABC");
    }

    #[test]
    fn test_vm_json_library() {
        let code = "local j = require(\"@neyuki/json\")\nlocal s = j.encode({ name = \"Neyuki\", version = 1 })\nlocal obj = j.decode(s)\nreturn obj.name";
        let res = run_code(code);
        assert_eq!(res.to_string(), "Neyuki");
    }

    #[test]
    fn test_vm_debug_library() {
        let code = "local d = require(\"@neyuki/debug\")\nlocal trace = d.traceback(\"test trace\")\nassert(string.find(trace, \"stack traceback\") != nil)\nreturn 1";
        let res = run_code(code);
        assert_eq!(res.to_string(), "1");
    }

    #[test]
    fn test_vm_coroutine_library() {
        let code = "local co = require(\"@neyuki/coroutine\")\nlocal f = function(x) return x * 10 end\nlocal t = co.create(f)\nlocal ok, val = co.resume(t, 5)\nassert(ok == true)\nassert(val == 50)\nassert(co.status(t) == \"dead\")\nreturn val";
        let res = run_code(code);
        assert_eq!(res.to_string(), "50");
    }

    #[test]
    fn test_vm_call_stack_overflow_caught() {
        // Infinite NON-tail recursion via Instruction::Call must be caught by
        // MAX_CALL_DEPTH = 512. (A tail call like `return f()` reuses the
        // frame instead and legitimately runs forever.)
        let code = "local function f()\n  return f() + 1\nend\nreturn f()";
        let stmts = crate::compiler::compile_source(code).expect("syntax error");
        let proto = crate::compiler::compile_to_proto(&stmts);
        let mut vm = VM::new();
        let res = vm.execute(proto);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("call stack overflow"));
    }

    #[test]
    fn test_vm_string_rep_limits() {
        // Empty string with giant n should be caught without allocating GBs of memory
        let code = "return string.rep(\"\", 2000000000)";
        let stmts = crate::compiler::compile_source(code).expect("syntax error");
        let proto = crate::compiler::compile_to_proto(&stmts);
        let mut vm = VM::new();
        let res = vm.execute(proto);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("exceeds maximum limit"));
    }

    #[test]
    fn test_vm_crypto_library() {
        let code = "local c = require(\"@neyuki/crypto\")\nlocal h = c.hash(\"hello world\", \"sha256\")\nreturn h";
        let res = run_code(code);
        // sha256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        assert_eq!(
            res.to_string(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_vm_coroutine_real_yield_resume() {
        let code = "local co = require(\"@neyuki/coroutine\")\n\
local f = function(x)\n\
  local y = co.yield(x + 10)\n\
  return y * 2\n\
end\n\
local t = co.create(f)\n\
assert(co.status(t) == \"suspended\")\n\
local ok1, val1 = co.resume(t, 5)\n\
assert(ok1 == true)\n\
assert(val1 == 15)\n\
assert(co.status(t) == \"suspended\")\n\
local ok2, val2 = co.resume(t, 20)\n\
assert(ok2 == true)\n\
assert(val2 == 40)\n\
assert(co.status(t) == \"dead\")\n\
return val2";
        let res = run_code(code);
        assert_eq!(res.to_string(), "40");
    }

    #[test]
    fn test_vm_debug_getinfo_and_traceback() {
        let code = "local d = require(\"@neyuki/debug\")\n\
local info = d.getinfo(d.traceback)\n\
assert(info.what == \"C\")\n\
assert(info.name == \"debug.traceback\")\n\
local tb = d.traceback(\"error message\")\n\
assert(string.len(tb) > 10)\n\
return 1";
        let res = run_code(code);
        assert_eq!(res.to_string(), "1");
    }

    #[test]
    fn test_vm_varargs() {
        let code = "local function sum(first, ...)\n\
  local second = ...\n\
  return first + second\n\
end\n\
return sum(10, 25, 99)";
        let res = run_code(code);
        assert_eq!(res.to_string(), "35");
    }

    #[test]
    fn test_vm_xpcall_with_args_and_custom_handler() {
        let code = "local function failing(a, b)\n\
  error(tostring(a + b))\n\
end\n\
local function handler(err)\n\
  return \"caught: \" .. tostring(err)\n\
end\n\
local ok, msg = xpcall(failing, handler, 20, 30)\n\
assert(ok == false)\n\
assert(msg == \"caught: 50\")\n\
return msg";
        let res = run_code(code);
        assert_eq!(res.to_string(), "caught: 50");
    }

    #[test]
    fn test_vm_table_sort_custom_comparator() {
        let code = "local t = { 3, 1, 4, 1, 5, 9 }\n\
table.sort(t, function(a, b) return a > b end)\n\
assert(t[1] == 9)\n\
assert(t[2] == 5)\n\
assert(t[6] == 1)\n\
return t[1]";
        let res = run_code(code);
        assert_eq!(res.to_string(), "9");
    }

    #[test]
    fn test_vm_string_format() {
        let code = "local s = string.format(\"hello %s, number %d, hex %x\", \"world\", 42, 255)\n\
return s";
        let res = run_code(code);
        assert_eq!(res.to_string(), "hello world, number 42, hex ff");
    }

    #[test]
    fn test_vm_string_sub_utf8_safe() {
        let code = "local s = \"xin chào thế giới\"\n\
local sub1 = string.sub(s, 1, 6)\n\
return sub1";
        let res = run_code(code);
        assert!(!res.to_string().is_empty());
    }

    #[test]
    fn test_vm_tonumber_bases() {
        let code = "assert(tonumber(\"1010\", 2) == 10)\n\
assert(tonumber(\"ff\", 16) == 255)\n\
assert(tonumber(\"0xFF\") == 255)\n\
assert(tonumber(\"  42  \") == 42)\n\
assert(tonumber(\"-0x10\") == -16)\n\
assert(tonumber(\"z\", 36) == 35)\n\
return tonumber(\"1010\", 2)";
        let res = run_code(code);
        assert_eq!(res.to_string(), "10");
    }

    #[test]
    fn test_vm_coroutine_no_magic_yield_collision() {
        let code = "local co = coroutine.create(function()\n\
    error(\"__NEYUKI_COROUTINE_YIELD__\")\n\
end)\n\
local ok, err = coroutine.resume(co)\n\
assert(ok == false)\n\
assert(coroutine.status(co) == \"dead\")\n\
return coroutine.status(co)";
        let res = run_code(code);
        assert_eq!(res.to_string(), "dead");
    }

    #[test]
    fn test_vm_open_upvalue_mutation() {
        let code = "local x = 1\n\
local f = function() return x end\n\
x = 2\n\
return f()";
        let res = run_code(code);
        assert_eq!(res.to_string(), "2");
    }

    #[test]
    fn test_vm_numeric_for_zero_step_rejected() {
        let code = "for i = 1, 10, 0 do end";
        let res = std::panic::catch_unwind(|| {
            run_code(code);
        });
        assert!(res.is_err());
    }

    #[test]
    fn test_vm_missing_param_nil_initialization() {
        let code = "local function f(a, b, c)\n\
  return c == nil\n\
end\n\
return f(1)";
        let res = run_code(code);
        assert_eq!(res.to_string(), "true");
    }

    #[test]
    fn test_vm_direct_table_generic_for() {
        let code = "local t = { 10, 20, 30 }\n\
local sum = 0\n\
for i, v in t do\n\
  sum = sum + v\n\
end\n\
return sum";
        let res = run_code(code);
        assert_eq!(res.to_string(), "60");
    }

    #[test]
    fn test_vm_require_path_traversal_blocked() {
        let code = "return require(\"../secret.nyk\")";
        let res = std::panic::catch_unwind(|| {
            run_code(code);
        });
        assert!(res.is_err());
    }

    #[test]
    fn test_vm_return_forwards_every_value() {
        let code = "local function pair()
  return 1, 2
end
local function forward()
  return pair()
end
local a, b = forward()
return a + b";
        assert_eq!(run_code(code).to_string(), "3");
    }

    #[test]
    fn test_vm_missing_returns_pad_with_nil() {
        // The callee returns one value where two were asked for; the second
        // must be nil rather than whatever the register held.
        let code = "local function one()
  return 7
end
local used = 99
local a, b = one()
return b == nil";
        assert_eq!(run_code(code).to_string(), "true");
    }

    #[test]
    fn test_vm_varargs_spread_into_call_and_table() {
        let code = "local table = require(\"@neyuki/table\")
local function count(...)
  local args = {...}
  return #args
end
local function relay(...)
  return count(...)
end
return relay(1, 2, 3) + #{table.unpack({4, 5})}";
        assert_eq!(run_code(code).to_string(), "5");
    }

    #[test]
    fn test_vm_block_local_captured_by_closure() {
        // The closure's upvalue must survive the register being reused after
        // the block it was declared in ends.
        let code = "local function make()
  local t = {}
  if (true) then
    local hidden = 11
    t.get = function() return hidden end
  end
  return t
end
return make().get()";
        assert_eq!(run_code(code).to_string(), "11");
    }

    #[test]
    fn test_vm_string_gmatch() {
        let code = "local string = require(\"@neyuki/string\")
local parts = {}
for piece in string.gmatch(\"a=1&b=2\", \"[^&]+\") do
  parts[#parts + 1] = piece
end
return parts[1] .. \",\" .. parts[2]";
        assert_eq!(run_code(code).to_string(), "a=1,b=2");
    }

    #[test]
    fn test_vm_requires_bundled_http_module() {
        // `@neyuki/http` has no Rust counterpart in the VM: it is the bundled
        // lib/http.nyk running on the bridged native primitives.
        let code = "local http = require(\"@neyuki/http\")
local path, params = http.parseQuery(\"/hello?name=you\")
return path .. \" \" .. params.name .. \" \" .. http.encode(\"a b\")";
        assert_eq!(run_code(code).to_string(), "/hello you a%20b");
    }

    #[test]
    fn test_vm_require_caches_modules() {
        let code = "return require(\"@neyuki/io\") == require(\"@neyuki/io\")";
        assert_eq!(run_code(code).to_string(), "true");
    }
}
