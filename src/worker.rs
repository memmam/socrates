//! Worker parallelism (v0.7): OS-thread isolates with string channels.
//!
//! `worker.spawn(file, args)` compiles `file` into a brand-new VM — its own
//! heap, globals, and GC — running on its own OS thread. The only things
//! that ever cross the thread boundary are `String` messages (structured
//! data goes over as JSON by convention) and the worker's final outcome,
//! so no GC'd value is ever shared and neither side needs locks.
//!
//! Lifecycle contract (documented in SPEC § 7):
//! - `spawn` blocks until the worker has *compiled*; compile errors come
//!   back synchronously as `Err(msg)` without a half-started thread.
//! - Parent `send`/child `worker.send` return `false` once the other side
//!   is gone (worker finished / handle collected) — never an error.
//! - `recv` blocks; `None` means "the other side is gone and the queue is
//!   drained", which is the natural loop-exit condition on both sides.
//! - `join()` first drops the parent's send side (a worker blocked in
//!   `worker.recv()` wakes up with `None` and can finish), then waits, and
//!   returns `Ok(())` or `Err(panic message)`. Joining twice returns the
//!   cached result. A worker whose handle is garbage-collected is detached,
//!   not cancelled; the process exits without waiting for it.

use std::io::Write;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::{check, compiler, diag, modules, source::Source, vm};

/// Where worker output (`println` etc.) goes. Defaults to process stdout;
/// the golden-test harness routes it into the same capture buffer as the
/// parent so worker programs are testable.
pub type Sink = Arc<Mutex<dyn Write + Send>>;

pub fn stdout_sink() -> Sink {
    Arc::new(Mutex::new(std::io::stdout()))
}

/// A `Write` adapter over a shared sink (each child VM owns one).
pub struct SinkWriter(pub Sink);

impl Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

/// The parent's end of a worker (inside `Obj::Worker`, behind `Rc<RefCell>`).
pub struct WorkerHandle {
    /// Parent → child. `None` after `join()` released it.
    tx: Option<Sender<String>>,
    /// Child → parent.
    rx: Receiver<String>,
    thread: Option<JoinHandle<Result<(), String>>>,
    /// Cached outcome once joined (join is idempotent).
    result: Option<Result<(), String>>,
}

impl std::fmt::Debug for WorkerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<worker>")
    }
}

impl WorkerHandle {
    /// `true` if the message was delivered to a still-listening worker.
    pub fn send(&self, msg: String) -> bool {
        match &self.tx {
            Some(tx) => tx.send(msg).is_ok(),
            None => false,
        }
    }

    /// Blocks. `None` = the worker finished and everything it sent has
    /// already been received.
    pub fn recv(&self) -> Option<String> {
        self.rx.recv().ok()
    }

    /// Non-blocking (v0.8): `None` = no message ready right now (a blocking
    /// `recv` would have waited); `Some(None)` = the worker finished and
    /// the queue is drained (the same terminal state `recv` reports as a
    /// bare `None`); `Some(Some(s))` = a message. Lets a parent poll several
    /// workers without picking one to block on — `swarm`'s dynamic
    /// scheduler wanted exactly this instead of its documented workaround.
    pub fn try_recv(&self) -> Option<Option<String>> {
        use std::sync::mpsc::TryRecvError;
        match self.rx.try_recv() {
            Ok(s) => Some(Some(s)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(None),
        }
    }

    pub fn join(&mut self) -> Result<(), String> {
        if let Some(r) = &self.result {
            return r.clone();
        }
        // Release our send side first so a worker blocked in `worker.recv()`
        // sees `None` instead of deadlocking against our join.
        self.tx = None;
        let outcome = match self.thread.take() {
            Some(t) => match t.join() {
                Ok(r) => r,
                Err(_) => Err("worker thread crashed (interpreter bug)".to_string()),
            },
            None => Err("internal: worker joined with no thread (VM bug)".to_string()),
        };
        self.result = Some(outcome.clone());
        outcome
    }
}

/// The child's end, installed on the worker's own `Vm`.
pub struct WorkerCtx {
    /// Child → parent.
    pub tx: Sender<String>,
    /// Parent → child.
    pub rx: Receiver<String>,
}

fn first_error(diags: &[diag::Diagnostic]) -> String {
    diags
        .iter()
        .find(|d| matches!(d.severity, diag::Severity::Error))
        .or(diags.first())
        .map(|d| d.message.clone())
        .unwrap_or_else(|| "unknown compile error".to_string())
}

/// Spawn a worker running `file` (resolved against `base_dir` unless
/// absolute — the same file-relative rule imports use). Blocks until the
/// worker compiles; compile problems surface here, not at `join`.
pub fn spawn(
    file: &str,
    args: Vec<String>,
    base_dir: &std::path::Path,
    sink: Sink,
) -> Result<WorkerHandle, String> {
    let path = {
        let p = std::path::Path::new(file);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            base_dir.join(p)
        }
    };
    if !path.is_file() {
        return Err(format!("worker file not found: {}", path.display()));
    }

    let (parent_tx, child_rx) = channel::<String>();
    let (child_tx, parent_rx) = channel::<String>();
    // Compile handshake: the child reports readiness (or a compile error)
    // exactly once before its program runs.
    let (status_tx, status_rx) = channel::<Result<(), String>>();

    let thread = std::thread::Builder::new()
        // Same headroom the CLI gives the main interpreter thread: deep
        // Socrates recursion needs Rust stack behind it (reserved, not
        // committed, so idle workers cost almost nothing).
        .stack_size(512 * 1024 * 1024)
        .name(format!("socrates-worker:{file}"))
        .spawn(move || worker_main(path, args, child_tx, child_rx, status_tx, sink))
        .map_err(|e| format!("could not spawn worker thread: {e}"))?;

    match status_rx.recv() {
        Ok(Ok(())) => Ok(WorkerHandle {
            tx: Some(parent_tx),
            rx: parent_rx,
            thread: Some(thread),
            result: None,
        }),
        Ok(Err(msg)) => {
            let _ = thread.join();
            Err(msg)
        }
        // The thread died before reporting (should be impossible).
        Err(_) => {
            let _ = thread.join();
            Err("worker thread died during startup (interpreter bug)".to_string())
        }
    }
}

fn worker_main(
    path: std::path::PathBuf,
    args: Vec<String>,
    tx: Sender<String>,
    rx: Receiver<String>,
    status: Sender<Result<(), String>>,
    sink: Sink,
) -> Result<(), String> {
    // Full pipeline, mirroring the CLI: modules → check → compile.
    let units = match modules::load_modules(&path) {
        Ok(u) => u,
        Err((_source, diags)) => {
            let msg = format!("worker compile error: {}", first_error(&diags));
            let _ = status.send(Err(msg.clone()));
            return Err(msg);
        }
    };
    let mut checker = check::Checker::new();
    let mut builder = compiler::ProgramBuilder::new();
    let mut entries = Vec::new();
    let mut final_program = None;
    for (i, unit) in units.iter().enumerate() {
        checker.check_module(&unit.program, &unit.prefix, unit.imports.clone());
        let diags = checker.take_diags();
        if diag::has_errors(&diags) {
            let msg = format!("worker compile error: {}", first_error(&diags));
            let _ = status.send(Err(msg.clone()));
            return Err(msg);
        }
        let compiled = builder.compile_chunk(&unit.program, &checker, i as u32);
        entries.push(compiled.entry);
        final_program = Some(compiled);
    }
    let program = final_program.expect("loader returns at least the root module");
    let _ = status.send(Ok(()));

    let mut units = units;
    let first_source: Source = units.remove(0).source;
    let mut machine = vm::Vm::new(program, first_source, Box::new(SinkWriter(sink.clone())));
    for unit in units {
        machine.sources.push(unit.source);
    }
    machine.script_args = args;
    machine.entry_dir = path.parent().map(|p| p.to_path_buf());
    machine.worker_ctx = Some(WorkerCtx { tx, rx });
    // Grandchildren spawned by this worker inherit the same output sink.
    machine.worker_sink = Some(sink);
    for entry in entries {
        if let Err(e) = machine.run_entry_at(entry) {
            return Err(e.msg);
        }
    }
    Ok(())
}
