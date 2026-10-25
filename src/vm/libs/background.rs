//! Blocking work that runs off the interpreter thread while a task waits.
//!
//! The interpreter is single-threaded, so a native that blocks (a network
//! request, a child process, a big file read) stalls every task and, under
//! `ui.run`, the window too. [`run`] is the one helper for that:
//!
//! ```ignore
//! fn builtin_fetch(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
//!     let url = string_arg(args, 0, "url")?;          // read arguments here,
//!     background::run(
//!         vm,
//!         move || download(&url),                     // block over there,
//!         |_vm, bytes| Ok(vec![to_value(bytes)]),     // build values back here.
//!     )
//! }
//! ```
//!
//! Called inside a task (a coroutine started by `task.spawn`, `task.defer`
//! or `task.delay` that can yield right now), `work` goes to a small shared
//! pool of worker threads and the task yields. When `work` returns, its
//! result is pushed onto the scheduler's completion queue and the scheduler
//! resumes the task on its next pass: `finish` runs on the interpreter
//! thread and its values become what the native returned, or its error is
//! raised in the task. Anywhere else (the main chunk, a plain coroutine, or
//! under a native that called back into Neyuki such as `try`) both closures
//! simply run in place, so the native blocks exactly as it always did.
//!
//! The rules that follow from that:
//!
//! - `work` must not touch the VM or any `Value` (they are not `Send`):
//!   copy what it needs out of the arguments first.
//! - `finish` may not run at all: when the task is cancelled with
//!   `task.cancel` while it waits, the result is dropped instead. Anything
//!   `work` hands back must therefore clean up after itself on drop.
//! - `work` cannot be interrupted, so it should bound its own blocking
//!   (timeouts) where it can; a cancelled task frees its pool slot only once
//!   `work` returns.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use crate::vm::machine::VM;
use crate::vm::value::Value;

/// At most this many worker threads run at once; jobs beyond that queue up.
/// It bounds how many blocking calls run in parallel, not how many tasks can
/// wait.
pub(crate) const MAX_WORKERS: usize = 16;

/// A worker with nothing to do for this long exits; the pool grows back on
/// demand.
const IDLE_EXIT: Duration = Duration::from_secs(30);

/// What a finished job hands back to the interpreter thread.
pub(crate) type Finish = Box<dyn FnOnce(&mut VM) -> Result<Vec<Value>, String> + Send>;

/// A finished job, identified by the id the scheduler filed its task under.
pub(crate) struct Completion {
    pub id: u64,
    pub finish: Finish,
}

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    workers: usize,
    idle: usize,
}

struct Pool {
    queue: Mutex<Queue>,
    wake: Condvar,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        queue: Mutex::new(Queue::default()),
        wake: Condvar::new(),
    })
}

/// Queues `job` on the pool, starting another worker if every running one is
/// busy and the pool is not full yet.
fn submit(job: Job) {
    let pool = pool();
    let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
    queue.jobs.push_back(job);
    if queue.jobs.len() > queue.idle && queue.workers < MAX_WORKERS {
        let spawned = std::thread::Builder::new()
            .name("neyuki-worker".to_string())
            .spawn(worker);
        match spawned {
            Ok(_) => queue.workers += 1,
            // No thread to be had: if no worker exists to pick the job up
            // later, run it here rather than leave the task waiting forever.
            Err(_) if queue.workers == 0 => {
                let job = queue.jobs.pop_back().expect("the job was just queued");
                drop(queue);
                job();
                return;
            }
            Err(_) => {}
        }
    }
    pool.wake.notify_one();
}

fn worker() {
    let pool = pool();
    loop {
        let job = {
            let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = queue.jobs.pop_front() {
                    break job;
                }
                queue.idle += 1;
                let (guard, timeout) = pool
                    .wake
                    .wait_timeout(queue, IDLE_EXIT)
                    .unwrap_or_else(|e| e.into_inner());
                queue = guard;
                queue.idle -= 1;
                if timeout.timed_out() && queue.jobs.is_empty() {
                    queue.workers -= 1;
                    return;
                }
            }
        };
        // A panicking job must not take the worker (and its pool slot) with
        // it; the job's own completion is lost, which `run` turns into an
        // error below.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
    }
}

/// Runs `work` off the interpreter thread while the calling task waits, or
/// in place where the caller cannot yield; see the module documentation.
pub(crate) fn run<T, W, F>(vm: &mut VM, work: W, finish: F) -> Result<Vec<Value>, String>
where
    T: Send + 'static,
    W: FnOnce() -> Result<T, String> + Send + 'static,
    F: FnOnce(&mut VM, T) -> Result<Vec<Value>, String> + Send + 'static,
{
    let Some(thread) = crate::vm::libs::task::current_task(vm) else {
        let value = work()?;
        return finish(vm, value);
    };
    let (id, sender) = crate::vm::libs::task::expect_completion(vm, thread);
    submit(Box::new(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err("background work panicked".to_string()));
        let finish: Finish = Box::new(move |vm| finish(vm, outcome?));
        sender.send(Completion { id, finish });
    }));
    crate::vm::libs::coroutine::coroutine_yield(vm, &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn test_pool_runs_jobs_in_parallel() {
        let (tx, rx) = mpsc::channel();
        let start = std::time::Instant::now();
        for i in 0..8 {
            let tx = tx.clone();
            submit(Box::new(move || {
                std::thread::sleep(Duration::from_millis(200));
                let _ = tx.send(i);
            }));
        }
        let mut seen: Vec<i32> = (0..8).map(|_| rx.recv().expect("job ran")).collect();
        seen.sort();
        assert_eq!(seen, (0..8).collect::<Vec<_>>());
        // Serially this would take 1.6 s.
        assert!(start.elapsed() < Duration::from_millis(1200));
    }

    #[test]
    fn test_pool_survives_a_panicking_job() {
        submit(Box::new(|| panic!("job failed")));
        let (tx, rx) = mpsc::channel();
        submit(Box::new(move || {
            let _ = tx.send(1);
        }));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(1));
    }
}
