//! The `task` library: a cooperative scheduler on top of coroutines.
//!
//! Every task is a coroutine. `task.spawn` runs one right away,
//! `task.defer` queues it for the next scheduler pass and `task.delay` and
//! `task.wait` park it until a deadline. The scheduler runs whenever the
//! main thread blocks in `task.wait`, and after the main chunk returns the
//! program keeps running until no task is left waiting.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::vm::libs::coroutine;
use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

/// What a parked task is resumed with when it wakes up.
enum Wake {
    /// A `task.wait` call: the seconds actually waited since `since`.
    Elapsed(Instant),
    /// A `task.delay` call: the arguments it was given.
    Args(Vec<Value>),
}

/// Tasks waiting for the scheduler. The values held here are coroutine
/// handles, which keeps them alive while nothing else references them.
#[derive(Default)]
pub struct Scheduler {
    /// Tasks for the next pass, in the order they were queued.
    deferred: VecDeque<(Value, Vec<Value>)>,
    /// Tasks waiting for a deadline. The sequence number keeps tasks with
    /// the same deadline in the order they started waiting.
    sleeping: BTreeMap<(Instant, u64), (Value, Wake)>,
    next_seq: u64,
    /// How many tasks ended in an error since the last `run_until_idle`.
    failed: usize,
}

impl Scheduler {
    fn sleep_until(&mut self, at: Instant, thread: Value, wake: Wake) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.sleeping.insert((at, seq), (thread, wake));
    }

    fn is_idle(&self) -> bool {
        self.deferred.is_empty() && self.sleeping.is_empty()
    }

    /// Drops every queued resumption of `thread`.
    fn forget(&mut self, thread: &Rc<RefCell<VmTable>>) {
        let same = |v: &Value| matches!(v, Value::Table(t) if Rc::ptr_eq(t, thread));
        self.deferred.retain(|(t, _)| !same(t));
        self.sleeping.retain(|_, (t, _)| !same(t));
    }
}

pub fn create_task_lib() -> Value {
    let t = Rc::new(RefCell::new(VmTable::new()));
    let mut b = t.borrow_mut();
    b.set_str("spawn", crate::native!("task.spawn", task_spawn));
    b.set_str("defer", crate::native!("task.defer", task_defer));
    b.set_str("delay", crate::native!("task.delay", task_delay));
    b.set_str("wait", crate::native!("task.wait", task_wait));
    b.set_str("cancel", crate::native!("task.cancel", task_cancel));
    b.frozen = true;
    drop(b);
    Value::Table(t)
}

/// The thread for a `spawn`/`defer`/`delay` argument: a function becomes a
/// new coroutine, a thread is used as it is.
fn task_thread(
    vm: &mut VM,
    arg: Option<&Value>,
    func: &str,
    index: usize,
) -> Result<Value, String> {
    match arg {
        Some(f @ (Value::Closure(_) | Value::Native(_))) => {
            Ok(coroutine::new_coroutine(vm, f.clone()))
        }
        Some(t @ Value::Table(_)) if t.type_name() == "thread" => Ok(t.clone()),
        _ => Err(format!(
            "bad argument #{} to '{}' (function or thread expected)",
            index, func
        )),
    }
}

/// A delay in seconds; nil means "as soon as possible".
fn seconds_arg(args: &[Value], index: usize, func: &str) -> Result<f64, String> {
    let seconds = match args.get(index) {
        None | Some(Value::Nil) => 0.0,
        Some(v) => crate::vm::libs::args::number(v).map_err(|_| {
            format!(
                "bad argument #{} to '{}' (number expected)",
                index + 1,
                func
            )
        })?,
    };
    if seconds.is_nan() || seconds < 0.0 {
        return Err(format!(
            "bad argument #{} to '{}' (seconds must be a non-negative number)",
            index + 1,
            func
        ));
    }
    Ok(seconds)
}

/// `now + seconds`, saturating far in the future for huge or infinite
/// delays instead of overflowing.
fn deadline(now: Instant, seconds: f64) -> Instant {
    Duration::try_from_secs_f64(seconds)
        .ok()
        .and_then(|d| now.checked_add(d))
        .unwrap_or_else(|| now + Duration::from_secs(60 * 60 * 24 * 365 * 100))
}

/// Resumes `thread` with `args` if it is still suspended. A task that ends
/// in an error is reported on stderr and counted; the other tasks go on.
fn resume_task(vm: &mut VM, thread: Value, args: Vec<Value>) {
    if !coroutine::is_suspended(vm, &thread) {
        return;
    }
    let mut resume_args = Vec::with_capacity(args.len() + 1);
    resume_args.push(thread);
    resume_args.extend(args);
    let failure = match coroutine::coroutine_resume(vm, &resume_args) {
        Ok(res) => match res.first() {
            Some(Value::Bool(false)) => Some(res.get(1).map(|v| v.to_string()).unwrap_or_default()),
            _ => None,
        },
        Err(err) => Some(err),
    };
    if let Some(message) = failure {
        eprintln!("task error: {}", message);
        vm.tasks.failed += 1;
    }
}

/// `task.spawn(f, ...)`: runs `f` (a function or thread) as a task right
/// away, until it first waits or yields, and returns its thread.
fn task_spawn(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let thread = task_thread(vm, args.first(), "task.spawn", 1)?;
    resume_task(vm, thread.clone(), args[1..].to_vec());
    Ok(vec![thread])
}

/// `task.defer(f, ...)`: runs `f` on the next scheduler pass.
fn task_defer(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let thread = task_thread(vm, args.first(), "task.defer", 1)?;
    vm.tasks
        .deferred
        .push_back((thread.clone(), args[1..].to_vec()));
    Ok(vec![thread])
}

/// `task.delay(seconds, f, ...)`: runs `f` once `seconds` have passed.
fn task_delay(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let seconds = seconds_arg(args, 0, "task.delay")?;
    let thread = task_thread(vm, args.get(1), "task.delay", 2)?;
    let rest = args.get(2..).unwrap_or_default().to_vec();
    let at = deadline(Instant::now(), seconds);
    vm.tasks.sleep_until(at, thread.clone(), Wake::Args(rest));
    Ok(vec![thread])
}

/// `task.wait(seconds?)`: suspends the calling task for `seconds` (by
/// default until the next scheduler pass) and returns the seconds actually
/// waited. Outside a yieldable coroutine, such as in the main chunk, it
/// blocks and runs the other tasks while it waits.
fn task_wait(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let seconds = seconds_arg(args, 0, "task.wait")?;
    let start = Instant::now();
    let at = deadline(start, seconds);
    if let Some(handle) = coroutine::yieldable_handle(vm) {
        vm.tasks
            .sleep_until(at, Value::Table(handle), Wake::Elapsed(start));
        return coroutine::coroutine_yield(vm, &[]);
    }
    run_until(vm, Some(at));
    Ok(vec![Value::Float(start.elapsed().as_secs_f64())])
}

/// `task.cancel(thread)`: unschedules a task and closes its coroutine.
fn task_cancel(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let Some(Value::Table(thread)) = args.first().filter(|v| v.type_name() == "thread") else {
        return Err("bad argument #1 to 'task.cancel' (thread expected)".to_string());
    };
    vm.tasks.forget(thread);
    match coroutine::coroutine_close(vm, &args[..1])?.as_slice() {
        [Value::Bool(false), message, ..] => Err(message.to_string()),
        _ => Ok(vec![]),
    }
}

/// Runs every task that is due now: the deferred queue and the sleepers
/// whose deadline has passed. Tasks queued while the pass runs wait for
/// the next one, so `task.wait()` in a loop cannot starve the scheduler.
/// Returns whether anything was resumed.
fn run_pass(vm: &mut VM) -> bool {
    let now = Instant::now();
    let deferred = std::mem::take(&mut vm.tasks.deferred);
    let mut due = Vec::new();
    while let Some(entry) = vm.tasks.sleeping.first_entry() {
        if entry.key().0 > now {
            break;
        }
        due.push(entry.remove());
    }
    let ran = !deferred.is_empty() || !due.is_empty();
    for (thread, args) in deferred {
        resume_task(vm, thread, args);
    }
    for (thread, wake) in due {
        let args = match wake {
            Wake::Elapsed(since) => vec![Value::Float(since.elapsed().as_secs_f64())],
            Wake::Args(args) => args,
        };
        resume_task(vm, thread, args);
    }
    ran
}

/// Runs tasks until `until` passes, or with `None` until no task is left
/// waiting, sleeping the thread while nothing is due.
fn run_until(vm: &mut VM, until: Option<Instant>) {
    loop {
        let ran = run_pass(vm);
        if until.is_some_and(|u| u <= Instant::now()) {
            return;
        }
        if ran {
            continue;
        }
        let next = vm.tasks.sleeping.first_key_value().map(|(k, _)| k.0);
        let wake = match (next, until) {
            (Some(n), Some(u)) => n.min(u),
            (Some(n), None) => n,
            (None, Some(u)) => u,
            (None, None) => return,
        };
        let now = Instant::now();
        if wake > now {
            std::thread::sleep(wake - now);
        }
    }
}

/// Runs the scheduler until every task has finished or is suspended
/// without a wake-up. Called once the main chunk has returned. Fails if
/// any task raised an error, after the others have run.
pub fn run_until_idle(vm: &mut VM) -> Result<(), String> {
    run_until(vm, None);
    debug_assert!(vm.tasks.is_idle());
    match std::mem::take(&mut vm.tasks.failed) {
        0 => Ok(()),
        1 => Err("a task raised an error".to_string()),
        n => Err(format!("{} tasks raised an error", n)),
    }
}

/// Runs the tasks that are due right now without blocking (the REPL calls
/// this after each chunk and whenever a task wakes up between lines).
pub fn run_due(vm: &mut VM) {
    run_pass(vm);
    vm.tasks.failed = 0;
}

/// When the next task is due: now if one is deferred, otherwise the
/// earliest deadline, or `None` when no task is waiting.
pub fn next_due(vm: &VM) -> Option<Instant> {
    if !vm.tasks.deferred.is_empty() {
        return Some(Instant::now());
    }
    vm.tasks.sleeping.first_key_value().map(|(k, _)| k.0)
}

#[cfg(test)]
mod tests {
    use crate::vm::machine::VM;
    use crate::vm::value::Value;

    fn run_program(vm: &mut VM, src: &str) -> Result<Value, String> {
        let proto = crate::compiler::compile_program(src, "test", crate::compiler::Options::RUN)?;
        vm.execute_program(proto)
    }

    fn global(vm: &VM, name: &str) -> Value {
        vm.globals
            .get(&crate::vm::value::StrRef::from(name))
            .cloned()
            .unwrap_or(Value::Nil)
    }

    #[test]
    fn test_tasks_run_after_the_main_chunk() {
        let mut vm = VM::new();
        run_program(
            &mut vm,
            "_G.ticks = 0\n\
             task.spawn(function()\n\
             \u{20} while task.wait(0.001) do\n\
             \u{20}   _G.ticks += 1\n\
             \u{20}   if _G.ticks == 3 then break end\n\
             \u{20} end\n\
             end)\n\
             task.delay(0.002, function(v: string) _G.delayed = v end, \"done\")\n\
             _G.main_saw = _G.ticks",
        )
        .expect("program failed");
        assert_eq!(global(&vm, "main_saw").to_string(), "0");
        assert_eq!(global(&vm, "ticks").to_string(), "3");
        assert_eq!(global(&vm, "delayed").to_string(), "done");
        assert!(vm.tasks.is_idle());
    }

    #[test]
    fn test_task_errors_fail_the_program_after_the_rest_ran() {
        let mut vm = VM::new();
        let err = run_program(
            &mut vm,
            "task.spawn(function() error(\"boom\") end)\n\
             task.delay(0.001, function() error(\"again\") end)\n\
             task.delay(0.002, function() _G.ran = true end)\n\
             _G.main_done = true",
        )
        .expect_err("a failing task must fail the program");
        assert_eq!(err, "2 tasks raised an error");
        assert!(matches!(global(&vm, "main_done"), Value::Bool(true)));
        assert!(matches!(global(&vm, "ran"), Value::Bool(true)));
    }
}
