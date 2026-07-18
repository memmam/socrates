//! The virtual machine: a stack machine over `CompiledProgram` bytecode.
//!
//! GC protocol: `gc_checkpoint()` may collect, using the value stack, globals,
//! frames, open upvalues, interned constants, cached function closures, and
//! `temp_roots` as roots. Handlers call it *before* removing operands from the
//! stack, so everything live is rooted at collection time; `Heap::alloc` never
//! collects on its own.

use std::io::Write;
use std::time::Instant;

use crate::bytecode::{CompiledProgram, Const, Op, RtDef};
use crate::source::Source;
use crate::value::{FMap, Handle, Heap, Obj, Upval, UpvalStorage, Value};

const DEFAULT_MAX_FRAMES: usize = 4096;

// FNV-1a, the structural-hash primitive (`Vm::hash_value`).
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[inline]
fn mix(h: u64, x: u64) -> u64 {
    (h ^ x).wrapping_mul(FNV_PRIME)
}

/// The call-depth cap. `SOCRATES_MAX_DEPTH` overrides the default (floor 64 —
/// below that the prelude itself couldn't run); recursive tree-walking
/// workloads on deep data legitimately want more headroom than the default.
/// A malformed value warns rather than being silently ignored — the variable
/// is usually set by someone actively chasing a stack-overflow panic.
fn max_frames() -> usize {
    let Ok(raw) = std::env::var("SOCRATES_MAX_DEPTH") else {
        return DEFAULT_MAX_FRAMES;
    };
    match raw.trim().parse::<usize>() {
        Ok(n) => n.max(64),
        Err(_) => {
            eprintln!(
                "warning: ignoring SOCRATES_MAX_DEPTH={raw:?} (not a number); \
                 using the default of {DEFAULT_MAX_FRAMES}"
            );
            DEFAULT_MAX_FRAMES
        }
    }
}

pub struct TraceFrame {
    pub fn_name: String,
    pub source_name: String,
    pub line: u32,
    pub col: u32,
}

pub struct VmError {
    pub msg: String,
    pub trace: Vec<TraceFrame>,
}

impl VmError {
    pub fn render(&self, color: bool) -> String {
        let mut out = String::new();
        let (red, bold, reset) = if color {
            ("\x1b[1;31m", "\x1b[1m", "\x1b[0m")
        } else {
            ("", "", "")
        };
        out.push_str(&format!("{red}panic:{reset} {bold}{}{reset}\n", self.msg));
        for t in &self.trace {
            if t.source_name.is_empty() {
                out.push_str(&format!("  at {}\n", t.fn_name));
            } else {
                out.push_str(&format!(
                    "  at {} ({}:{}:{})\n",
                    t.fn_name, t.source_name, t.line, t.col
                ));
            }
        }
        out
    }
}

struct Frame {
    proto: u32,
    closure: Option<Handle>,
    ip: usize,
    base: usize,
    callee_slot: bool,
}

pub struct Vm {
    pub heap: Heap,
    pub stack: Vec<Value>,
    frames: Vec<Frame>,
    pub globals: Vec<Value>,
    open_upvalues: Vec<Handle>,
    pub program: CompiledProgram,
    pub sources: Vec<Source>,
    /// Pre-resolved constants (strings interned as permanent handles).
    interned: Vec<Value>,
    /// Lazily interned single-character ASCII strings (GC roots once
    /// filled): `char()`, string iteration, `chars()`/`at()` hit these
    /// constantly in char-at-a-time code. Boxed so the table doesn't
    /// bloat `Vm` itself (hot fields stay on nearby cache lines).
    ascii_strs: Box<[Value; 128]>,
    /// Cached zero-upvalue closures for `PushFn`.
    fn_closures: Vec<Option<Value>>,
    pub temp_roots: Vec<Value>,
    /// Arguments after the script path on the CLI (`os.args()`).
    pub script_args: Vec<String>,
    pub out: Box<dyn Write>,
    /// The entry script's directory, for `worker.spawn`'s file-relative
    /// resolution. Explicit because `sources[0]` is the first *loaded*
    /// module (an import), not the entry script (v0.7 demo-round fix).
    pub entry_dir: Option<std::path::PathBuf>,
    /// Set when this VM *is* a worker: its channel ends to the parent.
    pub worker_ctx: Option<crate::worker::WorkerCtx>,
    /// Where workers spawned by this VM write their output. `None` means
    /// process stdout; the test harness routes it into its capture buffer.
    pub worker_sink: Option<crate::worker::Sink>,
    /// The window `win.make_current()` (v0.8) most recently bound as
    /// "current" — mirrors `glfwMakeContextCurrent`'s single-current-context
    /// model. Every `gfx.*` native reads this (see `natives::gfx_window`);
    /// `None` means either no window has ever called `make_current()`, or
    /// the `gl` cargo feature is off (no `WindowHandle` can be constructed
    /// in that build, so this stays `None` for the process's whole life).
    pub gfx_current_window: Option<std::rc::Rc<std::cell::RefCell<crate::window::WindowHandle>>>,
    start: Instant,
    rng: u64,
    /// Call-depth cap, read once from `SOCRATES_MAX_DEPTH` at construction.
    max_frames: usize,
}

impl Vm {
    pub fn new(program: CompiledProgram, source: Source, out: Box<dyn Write>) -> Vm {
        let mut vm = Vm {
            heap: Heap::new(),
            stack: Vec::with_capacity(256),
            frames: Vec::new(),
            globals: Vec::new(),
            open_upvalues: Vec::new(),
            program: CompiledProgram {
                protos: Vec::new(),
                consts: Vec::new(),
                defs: Vec::new(),
                globals: 0,
                global_names: Vec::new(),
                entry: 0,
            },
            sources: Vec::new(),
            interned: Vec::new(),
            ascii_strs: Box::new([Value::Undefined; 128]),
            fn_closures: Vec::new(),
            temp_roots: Vec::new(),
            script_args: Vec::new(),
            out,
            entry_dir: None,
            worker_ctx: None,
            worker_sink: None,
            gfx_current_window: None,
            start: Instant::now(),
            max_frames: max_frames(),
            rng: 0x9E3779B97F4A7C15 ^ {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0x1234_5678)
            },
        };
        vm.update_program(program, source);
        vm
    }

    /// Install a new (superset) program — used at startup and per REPL chunk.
    pub fn update_program(&mut self, program: CompiledProgram, source: Source) {
        self.program = program;
        self.sources.push(source);
        self.globals.resize(self.program.globals as usize, Value::Undefined);
        self.fn_closures.resize(self.program.protos.len(), None);
        // Intern any new constants.
        for i in self.interned.len()..self.program.consts.len() {
            let v = match &self.program.consts[i] {
                Const::Int(n) => Value::Int(*n),
                Const::Float(f) => Value::Float(*f),
                Const::Str(s) => {
                    let h = self.heap.alloc(Obj::Str(s.clone()));
                    Value::Obj(h)
                }
            };
            self.interned.push(v);
        }
    }

    /// Execute the current program's entry proto to completion. On panic the
    /// frame and value stacks unwind to their pre-entry state, so a persistent
    /// session (REPL) neither leaks frames into later stack traces nor burns
    /// call-depth budget.
    pub fn run_entry(&mut self) -> Result<Value, VmError> {
        self.run_entry_at(self.program.entry)
    }

    /// Execute a specific proto as an entry point (multi-module programs run
    /// each module's script proto in dependency order).
    pub fn run_entry_at(&mut self, entry: u32) -> Result<Value, VmError> {
        let base = self.stack.len();
        let entry_frames = self.frames.len();
        self.frames.push(Frame {
            proto: entry,
            closure: None,
            ip: 0,
            base,
            callee_slot: false,
        });
        let min = self.frames.len();
        match self.run(min) {
            Ok(()) => Ok(self.stack.pop().unwrap_or(Value::Unit)),
            Err(e) => {
                self.close_upvalues(base);
                self.frames.truncate(entry_frames);
                self.stack.truncate(base);
                self.temp_roots.clear();
                Err(e)
            }
        }
    }

    // ------------------------------------------------------------------
    // Errors
    // ------------------------------------------------------------------

    pub fn error(&self, msg: impl Into<String>) -> VmError {
        let mut trace = Vec::new();
        for f in self.frames.iter().rev().take(64) {
            let proto = &self.program.protos[f.proto as usize];
            let ip = f.ip.saturating_sub(1).min(proto.spans.len().saturating_sub(1));
            let (line, col, sname) = if proto.spans.is_empty() {
                (0, 0, "<unknown>".to_string())
            } else {
                let span = proto.spans[ip];
                let src = &self.sources[proto.source as usize];
                let lc = src.line_col(span.start);
                (lc.line, lc.col, src.name.clone())
            };
            trace.push(TraceFrame {
                fn_name: proto.name.clone(),
                source_name: sname,
                line,
                col,
            });
        }
        if self.frames.len() > 64 {
            trace.push(TraceFrame {
                fn_name: format!("... and {} more frames", self.frames.len() - 64),
                source_name: String::new(),
                line: 0,
                col: 0,
            });
        }
        VmError { msg: msg.into(), trace }
    }

    // ------------------------------------------------------------------
    // GC
    // ------------------------------------------------------------------

    pub fn gc_checkpoint(&mut self) {
        if !self.heap.wants_gc() {
            return;
        }
        for v in &self.stack {
            self.heap.mark_value(*v);
        }
        for v in &self.globals {
            self.heap.mark_value(*v);
        }
        for v in &self.temp_roots {
            self.heap.mark_value(*v);
        }
        for v in &self.interned {
            self.heap.mark_value(*v);
        }
        for v in self.ascii_strs.iter() {
            self.heap.mark_value(*v);
        }
        for v in self.fn_closures.iter().flatten() {
            self.heap.mark_value(*v);
        }
        for h in &self.open_upvalues {
            self.heap.mark_handle(*h);
        }
        for f in &self.frames {
            if let Some(h) = f.closure {
                self.heap.mark_handle(h);
            }
        }
        self.heap.trace();
        self.heap.sweep();
    }

    /// Checkpoint, then allocate (operands must be rooted by the caller).
    pub fn alloc(&mut self, obj: Obj) -> Handle {
        self.gc_checkpoint();
        self.heap.alloc(obj)
    }

    // ------------------------------------------------------------------
    // Stack helpers
    // ------------------------------------------------------------------

    #[inline]
    fn pop(&mut self) -> Value {
        self.stack.pop().expect("stack underflow (VM bug)")
    }

    #[inline]
    fn peek(&self, from_top: usize) -> Value {
        self.stack[self.stack.len() - 1 - from_top]
    }

    /// Write the dispatch loop's cached instruction pointer back into the
    /// current frame. `run()` keeps `ip` in a local; anything that builds a
    /// stack trace (errors, natives) or transfers control (calls, returns)
    /// must see the real value, so those paths sync first.
    #[inline]
    fn sync_ip(&mut self, ip: usize) {
        self.frames.last_mut().expect("no frame").ip = ip;
    }

    /// Sync `ip`, then build an error — the cold exit of dispatch handlers.
    #[cold]
    fn err_at(&mut self, ip: usize, msg: impl Into<String>) -> VmError {
        self.sync_ip(ip);
        self.error(msg)
    }

    /// Native calling convention: argument `i` of `argc` (stack-top relative).
    pub fn native_arg(&self, argc: u8, i: u8) -> Value {
        self.stack[self.stack.len() - argc as usize + i as usize]
    }

    /// Pop a native call's arguments and push its result.
    pub fn finish_native(&mut self, argc: u8, result: Value) {
        if argc == 0 {
            self.stack.push(result);
        } else {
            // Overwrite the first argument's slot, drop the rest.
            let n = self.stack.len() - argc as usize;
            self.stack[n] = result;
            self.stack.truncate(n + 1);
        }
    }

    pub fn str_of(&self, v: Value) -> Result<String, VmError> {
        match v {
            Value::Obj(h) => match self.heap.get(h) {
                Obj::Str(s) => Ok(s.clone()),
                _ => Err(self.error("internal: expected String (VM bug)")),
            },
            _ => Err(self.error("internal: expected String (VM bug)")),
        }
    }

    pub fn alloc_str(&mut self, s: String) -> Value {
        Value::Obj(self.alloc(Obj::Str(s)))
    }

    /// A single-character string. ASCII characters come from the interned
    /// table (filled on first use) instead of allocating each time.
    pub fn char_str(&mut self, c: char) -> Value {
        if !c.is_ascii() {
            return self.alloc_str(c.to_string());
        }
        let i = c as usize;
        let v = self.ascii_strs[i];
        if !matches!(v, Value::Undefined) {
            return v;
        }
        let h = self.heap.alloc(Obj::Str(c.to_string()));
        let v = Value::Obj(h);
        self.ascii_strs[i] = v;
        v
    }

    // ------------------------------------------------------------------
    // Upvalues
    // ------------------------------------------------------------------

    fn capture_upvalue(&mut self, stack_idx: usize) -> Handle {
        for &h in &self.open_upvalues {
            if let Obj::Upvalue(Upval::Open(i)) = self.heap.get(h) {
                if *i == stack_idx {
                    return h;
                }
            }
        }
        let h = self.heap.alloc(Obj::Upvalue(Upval::Open(stack_idx)));
        self.open_upvalues.push(h);
        h
    }

    fn close_upvalues(&mut self, from: usize) {
        // Hot path: block exits and returns land here, and open upvalues are
        // rare — bail without touching anything when there are none, and
        // close in place (order is irrelevant; this is a search list).
        if self.open_upvalues.is_empty() {
            return;
        }
        let mut i = 0;
        while i < self.open_upvalues.len() {
            let h = self.open_upvalues[i];
            let close = match self.heap.get(h) {
                Obj::Upvalue(Upval::Open(idx)) if *idx >= from => Some(*idx),
                _ => None,
            };
            match close {
                Some(idx) => {
                    let v = self.stack[idx];
                    *self.heap.get_mut(h) = Obj::Upvalue(Upval::Closed(v));
                    self.open_upvalues.swap_remove(i);
                }
                None => i += 1,
            }
        }
    }

    // ------------------------------------------------------------------
    // Calls
    // ------------------------------------------------------------------

    fn push_frame(
        &mut self,
        proto: u32,
        closure: Option<Handle>,
        argc: u8,
        callee_slot: bool,
    ) -> Result<(), VmError> {
        if self.frames.len() >= self.max_frames {
            return Err(self.error("stack overflow"));
        }
        let p = &self.program.protos[proto as usize];
        if p.arity != argc {
            return Err(self.error(format!(
                "internal: `{}` expects {} args, got {argc} (VM bug)",
                p.name, p.arity
            )));
        }
        self.frames.push(Frame {
            proto,
            closure,
            ip: 0,
            base: self.stack.len() - argc as usize,
            callee_slot,
        });
        Ok(())
    }

    /// Replace the current frame in place for a tail call: close its
    /// upvalues, slide the callee slot (when present) and args down over the
    /// frame, and restart execution in `proto`. The caller sees the eventual
    /// result exactly where the departing frame would have left its own.
    fn reuse_frame(
        &mut self,
        proto: u32,
        closure: Option<Handle>,
        argc: u8,
    ) -> Result<(), VmError> {
        let p = &self.program.protos[proto as usize];
        if p.arity != argc {
            return Err(self.error(format!(
                "internal: `{}` expects {} args, got {argc} (VM bug)",
                p.name, p.arity
            )));
        }
        let f = self.frames.last().unwrap();
        let (old_base, old_callee) = (f.base, f.callee_slot);
        self.close_upvalues(old_base);
        let cut = old_base - usize::from(old_callee);
        let has_callee = usize::from(closure.is_some());
        let move_n = argc as usize + has_callee;
        let start = self.stack.len() - move_n;
        for i in 0..move_n {
            self.stack[cut + i] = self.stack[start + i];
        }
        self.stack.truncate(cut + move_n);
        let f = self.frames.last_mut().unwrap();
        f.proto = proto;
        f.closure = closure;
        f.ip = 0;
        f.base = cut + has_callee;
        f.callee_slot = has_callee == 1;
        Ok(())
    }

    /// Call a zero-argument callable, catching runtime panics: on failure
    /// the frame and value stacks (and open upvalues and temp roots) are
    /// restored to their pre-call state and the panic message is returned.
    /// Side effects performed before the panic are kept — `try` is a
    /// recovery boundary, not a transaction.
    pub fn call_value_caught(&mut self, callee: Value) -> Result<Value, String> {
        let frames_at = self.frames.len();
        let stack_at = self.stack.len();
        let roots_at = self.temp_roots.len();
        match self.call_value(callee, &[]) {
            Ok(v) => Ok(v),
            Err(e) => {
                self.close_upvalues(stack_at);
                self.frames.truncate(frames_at);
                self.stack.truncate(stack_at);
                self.temp_roots.truncate(roots_at);
                Err(e.msg)
            }
        }
    }

    /// Call any callable value with the given arguments (used by natives).
    pub fn call_value(&mut self, callee: Value, args: &[Value]) -> Result<Value, VmError> {
        match callee {
            Value::Native(n) => {
                for a in args {
                    self.stack.push(*a);
                }
                crate::natives::call_native(self, n, args.len() as u8)?;
                Ok(self.pop())
            }
            Value::Obj(h) => {
                let proto = match self.heap.get(h) {
                    Obj::Closure { proto, .. } => *proto,
                    _ => return Err(self.error("value is not callable")),
                };
                let entry_frames = self.frames.len();
                self.stack.push(callee);
                for a in args {
                    self.stack.push(*a);
                }
                self.push_frame(proto, Some(h), args.len() as u8, true)?;
                self.run(entry_frames + 1)?;
                Ok(self.pop())
            }
            _ => Err(self.error("value is not callable")),
        }
    }

    // ------------------------------------------------------------------
    // Main dispatch loop
    // ------------------------------------------------------------------

    fn run(&mut self, min_frames: usize) -> Result<(), VmError> {
        // Frame-hot state lives in locals: `ip` (already advanced past the
        // current op), `base`, and the current proto index. They are synced
        // back to the frame at every call/return/error (see `sync_ip`), and
        // reloaded whenever the current frame changes.
        let frame = self.frames.last().expect("no frame");
        let mut proto_idx = frame.proto as usize;
        let mut ip = frame.ip;
        let mut base = frame.base;
        loop {
            let op = self.program.protos[proto_idx].code[ip];
            ip += 1;

            match op {
                Op::Const(i) => self.stack.push(self.interned[i as usize]),
                Op::Unit => self.stack.push(Value::Unit),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),
                Op::Pop => {
                    self.pop();
                }
                Op::PopN(n) => {
                    let len = self.stack.len() - n as usize;
                    self.stack.truncate(len);
                }
                Op::Dup => self.stack.push(self.peek(0)),
                Op::Dup2 => {
                    let b = self.peek(0);
                    let a = self.peek(1);
                    self.stack.push(a);
                    self.stack.push(b);
                }
                Op::EndBlock(n) => {
                    let top = self.pop();
                    let new_len = self.stack.len() - n as usize;
                    self.close_upvalues(new_len);
                    self.stack.truncate(new_len);
                    self.stack.push(top);
                }
                Op::PopScope(n) => {
                    let new_len = self.stack.len() - n as usize;
                    self.close_upvalues(new_len);
                    self.stack.truncate(new_len);
                }

                Op::GetLocal(s) => self.stack.push(self.stack[base + s as usize]),
                // Superinstructions (H3): each arm is the exact sequential
                // composition of its two constituent ops (see bytecode.rs);
                // compact + frequent, so they stay inline like their parts.
                Op::GetLocal2(a, b) => {
                    self.stack.push(self.stack[base + a as usize]);
                    self.stack.push(self.stack[base + b as usize]);
                }
                Op::GetLocalConst(s, c) => {
                    self.stack.push(self.stack[base + s as usize]);
                    self.stack.push(self.interned[c as usize]);
                }
                Op::GetGlobalConst(g, c) => {
                    let v = self.globals[g as usize];
                    if matches!(v, Value::Undefined) {
                        return Err(self.err_uninit_global(g, ip));
                    }
                    self.stack.push(v);
                    self.stack.push(self.interned[c as usize]);
                }
                Op::GetLocalTestVariant(s, i) => {
                    let t = self.stack[base + s as usize];
                    let Value::Obj(h) = t else {
                        return Err(
                            self.err_at(ip, "internal: bad enum value (VM bug)")
                        );
                    };
                    let b = match self.heap.get(h) {
                        Obj::Variant { variant, .. } => *variant == i as u32,
                        _ => {
                            return Err(
                                self.err_at(ip, "internal: bad enum value (VM bug)")
                            )
                        }
                    };
                    self.stack.push(t);
                    self.stack.push(Value::Bool(b));
                }
                Op::SetLocal(s) => {
                    let v = self.pop();
                    self.stack[base + s as usize] = v;
                }
                Op::GetGlobal(g) => {
                    let v = self.globals[g as usize];
                    if matches!(v, Value::Undefined) {
                        return Err(self.err_uninit_global(g, ip));
                    }
                    self.stack.push(v);
                }
                Op::SetGlobal(g) => {
                    let v = self.pop();
                    self.globals[g as usize] = v;
                }
                Op::GetUpvalue(i) => {
                    let closure = self.frames.last().unwrap().closure.expect("no closure");
                    let uh = match self.heap.get(closure) {
                        Obj::Closure { upvals, .. } => upvals.as_slice()[i as usize],
                        _ => return Err(self.err_at(ip, "internal: bad closure (VM bug)")),
                    };
                    let v = match self.heap.get(uh) {
                        Obj::Upvalue(Upval::Open(idx)) => self.stack[*idx],
                        Obj::Upvalue(Upval::Closed(v)) => *v,
                        _ => return Err(self.err_at(ip, "internal: bad upvalue (VM bug)")),
                    };
                    self.stack.push(v);
                }
                Op::SetUpvalue(i) => self.op_set_upvalue(i, ip)?,

                Op::PushFn(p) => {
                    if let Some(v) = self.fn_closures[p as usize] {
                        self.stack.push(v);
                    } else {
                        let h = self.alloc(Obj::Closure { proto: p, upvals: UpvalStorage::new() });
                        let v = Value::Obj(h);
                        self.fn_closures[p as usize] = Some(v);
                        self.stack.push(v);
                    }
                }
                Op::PushNative(n) => self.stack.push(Value::Native(n)),
                Op::Closure(p) => self.op_closure(p, base, ip)?,

                Op::Jump(off) => {
                    ip = (ip as i64 + off as i64) as usize;
                }
                Op::JumpIfFalse(off) => {
                    let v = self.pop();
                    match v {
                        Value::Bool(b) => {
                            if !b {
                                ip = (ip as i64 + off as i64) as usize;
                            }
                        }
                        _ => return Err(self.err_at(ip, "internal: expected Bool (VM bug)")),
                    }
                }
                Op::JumpIfFalsePeek(off) => {
                    let v = self.peek(0);
                    match v {
                        Value::Bool(b) => {
                            if !b {
                                ip = (ip as i64 + off as i64) as usize;
                            }
                        }
                        _ => return Err(self.err_at(ip, "internal: expected Bool (VM bug)")),
                    }
                }
                Op::JumpIfTruePeek(off) => {
                    let v = self.peek(0);
                    match v {
                        Value::Bool(b) => {
                            if b {
                                ip = (ip as i64 + off as i64) as usize;
                            }
                        }
                        _ => return Err(self.err_at(ip, "internal: expected Bool (VM bug)")),
                    }
                }

                Op::Call(argc) => {
                    let callee = self.peek(argc as usize);
                    match callee {
                        Value::Obj(h) => {
                            let proto = match self.heap.get(h) {
                                Obj::Closure { proto, .. } => *proto,
                                _ => {
                                    return Err(self.err_at(ip, "value is not callable"));
                                }
                            };
                            self.sync_ip(ip);
                            self.push_frame(proto, Some(h), argc, true)?;
                            proto_idx = proto as usize;
                            ip = 0;
                            base = self.stack.len() - argc as usize;
                        }
                        Value::Native(n) => {
                            self.sync_ip(ip);
                            crate::natives::call_native(self, n, argc)?;
                            // Overwrite the callee slot beneath the result.
                            let result = self.pop();
                            *self.stack.last_mut().expect("stack underflow (VM bug)") = result;
                        }
                        _ => return Err(self.err_at(ip, "value is not callable")),
                    }
                }
                Op::CallFn(p, argc) => {
                    self.sync_ip(ip);
                    self.push_frame(p, None, argc, false)?;
                    proto_idx = p as usize;
                    ip = 0;
                    base = self.stack.len() - argc as usize;
                }
                Op::TailCallFn(p, argc) => {
                    self.sync_ip(ip);
                    self.reuse_frame(p, None, argc)?;
                    proto_idx = p as usize;
                    ip = 0;
                    base = self.frames.last().unwrap().base;
                }
                Op::TailCall(argc) => {
                    let callee = self.peek(argc as usize);
                    match callee {
                        Value::Obj(h) => {
                            let proto = match self.heap.get(h) {
                                Obj::Closure { proto, .. } => *proto,
                                _ => {
                                    return Err(self.err_at(ip, "value is not callable"));
                                }
                            };
                            self.sync_ip(ip);
                            self.reuse_frame(proto, Some(h), argc)?;
                            proto_idx = proto as usize;
                            ip = 0;
                            base = self.frames.last().unwrap().base;
                        }
                        Value::Native(n) => {
                            // A native in tail position pushes no frame; call
                            // it and return its result like `Op::Return`.
                            self.sync_ip(ip);
                            crate::natives::call_native(self, n, argc)?;
                            let result = self.pop();
                            // (The callee slot beneath is removed by the
                            // truncate below.)
                            let f = self.frames.pop().unwrap();
                            self.close_upvalues(f.base);
                            let cut = f.base - usize::from(f.callee_slot);
                            self.stack.truncate(cut);
                            self.stack.push(result);
                            if self.frames.len() < min_frames {
                                return Ok(());
                            }
                            let f = self.frames.last().unwrap();
                            proto_idx = f.proto as usize;
                            ip = f.ip;
                            base = f.base;
                        }
                        _ => return Err(self.err_at(ip, "value is not callable")),
                    }
                }
                Op::CallNative(n, argc) => {
                    self.sync_ip(ip);
                    crate::natives::call_native(self, n, argc)?;
                }
                Op::Return => {
                    let result = self.pop();
                    let f = self.frames.pop().unwrap();
                    self.close_upvalues(f.base);
                    let cut = f.base - usize::from(f.callee_slot);
                    self.stack.truncate(cut);
                    self.stack.push(result);
                    if self.frames.len() < min_frames {
                        return Ok(());
                    }
                    let f = self.frames.last().unwrap();
                    proto_idx = f.proto as usize;
                    ip = f.ip;
                    base = f.base;
                }

                Op::Add => self.op_add(ip)?,
                Op::Sub => self.op_arith(op, ip)?,
                Op::Mul => self.op_arith(op, ip)?,
                Op::Div => self.op_arith(op, ip)?,
                Op::Rem => self.op_arith(op, ip)?,
                Op::BitAnd | Op::BitOr | Op::BitXor | Op::Shl | Op::Shr => {
                    self.op_bitwise(op, ip)?
                }
                Op::Neg => {
                    let v = self.pop();
                    let r = match v {
                        Value::Int(i) => Value::Int(
                            i.checked_neg()
                                .ok_or_else(|| self.err_at(ip, "integer overflow"))?,
                        ),
                        Value::Float(f) => Value::Float(-f),
                        _ => {
                            return Err(
                                self.err_at(ip, "internal: bad negate operand (VM bug)")
                            )
                        }
                    };
                    self.stack.push(r);
                }
                Op::Not => {
                    let v = self.pop();
                    match v {
                        Value::Bool(b) => self.stack.push(Value::Bool(!b)),
                        _ => return Err(self.err_at(ip, "internal: expected Bool (VM bug)")),
                    }
                }
                Op::Eq => {
                    let b = self.peek(0);
                    let a = self.peek(1);
                    let eq = self.value_eq(a, b, 0).map_err(|m| self.err_at(ip, m))?;
                    let n = self.stack.len() - 2;
                    self.stack[n] = Value::Bool(eq);
                    self.stack.truncate(n + 1);
                }
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let b = self.peek(0);
                    let a = self.peek(1);
                    // Floats get direct IEEE-754 comparisons (every ordered
                    // comparison involving NaN is false).
                    let r = if let (Value::Float(x), Value::Float(y)) = (a, b) {
                        match op {
                            Op::Lt => x < y,
                            Op::Le => x <= y,
                            Op::Gt => x > y,
                            _ => x >= y,
                        }
                    } else {
                        let ord = self.compare(a, b).map_err(|m| self.err_at(ip, m))?;
                        match op {
                            Op::Lt => ord.is_lt(),
                            Op::Le => ord.is_le(),
                            Op::Gt => ord.is_gt(),
                            _ => ord.is_ge(),
                        }
                    };
                    let n = self.stack.len() - 2;
                    self.stack[n] = Value::Bool(r);
                    self.stack.truncate(n + 1);
                }

                Op::ToString => self.op_to_string(ip)?,
                Op::Concat(n) => self.op_concat(n, ip)?,

                Op::MakeList(n) => {
                    self.gc_checkpoint();
                    let start = self.stack.len() - n as usize;
                    let items: Vec<Value> = self.stack.split_off(start);
                    let h = self.heap.alloc(Obj::List(items));
                    self.stack.push(Value::Obj(h));
                }
                Op::MakeMap(n) => self.op_make_map(n, ip)?,
                Op::MakeTuple(n) => {
                    self.gc_checkpoint();
                    let start = self.stack.len() - n as usize;
                    let items: Vec<Value> = self.stack.split_off(start);
                    let h = self.heap.alloc(Obj::Tuple(items));
                    self.stack.push(Value::Obj(h));
                }
                Op::MakeRange { inclusive } => self.op_make_range(inclusive, ip)?,
                Op::MakeStructEmpty(def) => self.op_make_struct_empty(def),
                Op::StructSetField(i) => {
                    let v = self.pop();
                    let s = self.peek(0);
                    let Value::Obj(h) = s else {
                        return Err(self.err_at(ip, "internal: bad struct (VM bug)"));
                    };
                    match self.heap.get_mut(h) {
                        Obj::Struct { fields, .. } => fields[i as usize] = v,
                        _ => return Err(self.err_at(ip, "internal: bad struct (VM bug)")),
                    }
                }
                Op::MakeVariant { def, variant, arity } => {
                    self.gc_checkpoint();
                    let start = self.stack.len() - arity as usize;
                    let fields: Vec<Value> = self.stack.split_off(start);
                    let h = self.heap.alloc(Obj::Variant {
                        def,
                        variant: variant as u32,
                        fields,
                    });
                    self.stack.push(Value::Obj(h));
                }

                Op::GetField(i) => {
                    let s = self.pop();
                    let Value::Obj(h) = s else {
                        return Err(self.err_at(ip, "internal: bad struct (VM bug)"));
                    };
                    let v = match self.heap.get(h) {
                        Obj::Struct { fields, .. } => fields[i as usize],
                        _ => return Err(self.err_at(ip, "internal: bad struct (VM bug)")),
                    };
                    self.stack.push(v);
                }
                Op::SetField(i) => {
                    let v = self.pop();
                    let s = self.pop();
                    let Value::Obj(h) = s else {
                        return Err(self.err_at(ip, "internal: bad struct (VM bug)"));
                    };
                    match self.heap.get_mut(h) {
                        Obj::Struct { fields, .. } => fields[i as usize] = v,
                        _ => return Err(self.err_at(ip, "internal: bad struct (VM bug)")),
                    }
                }
                Op::TupleGet(i) => {
                    let t = self.pop();
                    let Value::Obj(h) = t else {
                        return Err(self.err_at(ip, "internal: bad tuple (VM bug)"));
                    };
                    let v = match self.heap.get(h) {
                        Obj::Tuple(items) => items[i as usize],
                        _ => return Err(self.err_at(ip, "internal: bad tuple (VM bug)")),
                    };
                    self.stack.push(v);
                }
                Op::GetVariantField(i) => {
                    let t = self.pop();
                    let Value::Obj(h) = t else {
                        return Err(self.err_at(ip, "internal: bad enum value (VM bug)"));
                    };
                    let v = match self.heap.get(h) {
                        Obj::Variant { fields, .. } => fields[i as usize],
                        _ => return Err(self.err_at(ip, "internal: bad enum value (VM bug)")),
                    };
                    self.stack.push(v);
                }
                Op::TestVariant(i) => {
                    let t = self.peek(0);
                    let Value::Obj(h) = t else {
                        return Err(self.err_at(ip, "internal: bad enum value (VM bug)"));
                    };
                    let b = match self.heap.get(h) {
                        Obj::Variant { variant, .. } => *variant == i as u32,
                        _ => return Err(self.err_at(ip, "internal: bad enum value (VM bug)")),
                    };
                    self.stack.push(Value::Bool(b));
                }

                Op::Index => {
                    let idx = self.pop();
                    let base_v = self.pop();
                    let v = self.index_get(base_v, idx, ip)?;
                    self.stack.push(v);
                }
                Op::IndexSet => {
                    let v = self.pop();
                    let idx = self.pop();
                    let base_v = self.pop();
                    self.index_set(base_v, idx, v, ip)?;
                }

                Op::ForPrep => self.op_for_prep(ip)?,
                Op::ForNext(off) => ip = self.op_for_next(off, ip)?,
                Op::ForNextRange { off, inclusive } => {
                    let hi_v = self.peek(0);
                    let cur_v = self.peek(1);
                    match (cur_v, hi_v) {
                        (Value::Int(cur), Value::Int(hi)) => {
                            let in_range = if inclusive { cur <= hi } else { cur < hi };
                            if in_range {
                                let next_state = cur
                                    .checked_add(1)
                                    .map(Value::Int)
                                    .unwrap_or(Value::Unit);
                                let n = self.stack.len();
                                self.stack[n - 2] = next_state;
                                self.stack.push(Value::Int(cur));
                            } else {
                                ip = (ip as i64 + off as i64) as usize;
                            }
                        }
                        (Value::Unit, _) => {
                            ip = (ip as i64 + off as i64) as usize;
                        }
                        _ => {
                            return Err(
                                self.err_at(ip, "internal: bad range loop state (VM bug)")
                            )
                        }
                    }
                }

                Op::MatchFail => {
                    return Err(self.err_at(
                        ip,
                        "match did not cover the scrutinee value (all arms failed)",
                    ));
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Op bodies with per-target binding (bench/RESULTS.md, "The dispatch
    // restructure"). The bodies live here once; the attribute pair binds
    // each target to its measured-fastest form. Default (compact loop):
    // #[inline(never)] keeps bulky or rare bodies out of run(), so the
    // hot loop's machine code stays small — with lto=true and
    // codegen-units=1, any edit anywhere shifts every function address,
    // and a large run() amplifies those layout shifts into measurable
    // dispatch swings (the codegen lottery). aarch64-linux
    // (`monolithic_dispatch`, emitted by build.rs): #[inline(always)]
    // folds the bodies back into run() — the compact loop measured a
    // reproducible enum_match cost there and the monolith measured none.
    // ------------------------------------------------------------------

    #[cold]
    #[inline(never)]
    fn err_uninit_global(&mut self, g: u16, ip: usize) -> VmError {
        let name = self
            .program
            .global_names
            .get(g as usize)
            .cloned()
            .unwrap_or_default();
        self.err_at(ip, format!("global `{name}` used before initialization"))
    }

    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_set_upvalue(&mut self, i: u16, ip: usize) -> Result<(), VmError> {
        let v = self.pop();
        let closure = self.frames.last().unwrap().closure.expect("no closure");
        let uh = match self.heap.get(closure) {
            Obj::Closure { upvals, .. } => upvals.as_slice()[i as usize],
            _ => return Err(self.err_at(ip, "internal: bad closure (VM bug)")),
        };
        match self.heap.get_mut(uh) {
            Obj::Upvalue(u @ Upval::Open(_)) => {
                if let Upval::Open(idx) = *u {
                    self.stack[idx] = v;
                }
            }
            Obj::Upvalue(u) => *u = Upval::Closed(v),
            _ => return Err(self.err_at(ip, "internal: bad upvalue (VM bug)")),
        }
        Ok(())
    }

    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_closure(&mut self, p: u32, base: usize, ip: usize) -> Result<(), VmError> {
        self.gc_checkpoint();
        let parent_closure = self.frames.last().unwrap().closure;
        let n_upvals = self.program.protos[p as usize].upvals.len();
        let mut upvals = UpvalStorage::with_capacity(n_upvals);
        for k in 0..n_upvals {
            let d = self.program.protos[p as usize].upvals[k];
            if d.from_local {
                let h = self.capture_upvalue(base + d.index as usize);
                upvals.push(h);
            } else {
                let pc = parent_closure.expect("upvalue chain without closure");
                let uh = match self.heap.get(pc) {
                    Obj::Closure { upvals, .. } => upvals.as_slice()[d.index as usize],
                    _ => return Err(self.err_at(ip, "internal: bad closure (VM bug)")),
                };
                upvals.push(uh);
            }
        }
        let h = self.heap.alloc(Obj::Closure { proto: p, upvals });
        self.stack.push(Value::Obj(h));
        Ok(())
    }

    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_to_string(&mut self, ip: usize) -> Result<(), VmError> {
        self.sync_ip(ip);
        let v = self.peek(0);
        // A string already is its own display form; strings are
        // immutable and never identity-compared, so the handle
        // can stay in place as-is.
        if !matches!(v, Value::Obj(h)
            if matches!(self.heap.get(h), Obj::Str(_)))
        {
            let s = self.display_value(v)?;
            let sv = self.alloc_str(s);
            self.pop();
            self.stack.push(sv);
        }
        Ok(())
    }

    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_concat(&mut self, n: u16, ip: usize) -> Result<(), VmError> {
        self.sync_ip(ip);
        let n = n as usize;
        // Exact-size the result, then copy each part straight
        // out of the heap (no per-part clone).
        let mut cap = 0usize;
        for i in 0..n {
            match self.peek(i) {
                Value::Obj(h) => match self.heap.get(h) {
                    Obj::Str(part) => cap += part.len(),
                    _ => return Err(self.error("internal: expected String (VM bug)")),
                },
                _ => return Err(self.error("internal: expected String (VM bug)")),
            }
        }
        let mut s = String::with_capacity(cap);
        for i in (0..n).rev() {
            let Value::Obj(h) = self.peek(i) else { unreachable!() };
            let Obj::Str(part) = self.heap.get(h) else { unreachable!() };
            s.push_str(part);
        }
        let sv = self.alloc_str(s);
        let len = self.stack.len() - n;
        self.stack.truncate(len);
        self.stack.push(sv);
        Ok(())
    }

    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_make_map(&mut self, n: u16, ip: usize) -> Result<(), VmError> {
        self.sync_ip(ip);
        self.gc_checkpoint();
        let start = self.stack.len() - 2 * n as usize;
        let kvs: Vec<Value> = self.stack.split_off(start);
        // Values already off-stack: root them while we hash/alloc.
        let tr = self.temp_roots.len();
        self.temp_roots.extend_from_slice(&kvs);
        let mut map = FMap::new();
        for pair in kvs.chunks(2) {
            let (k, v) = (pair[0], pair[1]);
            let hash = self.hash_value(k, 0).map_err(|m| self.error(m))?;
            let existing = self.map_find(&map, hash, k).map_err(|m| self.error(m))?;
            match existing {
                Some(idx) => {
                    map.set_at(idx, v);
                }
                None => map.push(hash, k, v),
            }
        }
        let h = self.heap.alloc(Obj::Map(map));
        self.temp_roots.truncate(tr);
        self.stack.push(Value::Obj(h));
        Ok(())
    }

    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_make_range(&mut self, inclusive: bool, ip: usize) -> Result<(), VmError> {
        self.gc_checkpoint();
        let hi = self.pop();
        let lo = self.pop();
        let (Value::Int(lo), Value::Int(hi)) = (lo, hi) else {
            return Err(self.err_at(ip, "internal: bad range bounds (VM bug)"));
        };
        let h = self.heap.alloc(Obj::Range { lo, hi, inclusive });
        self.stack.push(Value::Obj(h));
        Ok(())
    }

    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_make_struct_empty(&mut self, def: u32) {
        self.gc_checkpoint();
        let nfields = match &self.program.defs[def as usize] {
            RtDef::Struct { fields, .. } => fields.len(),
            _ => 0,
        };
        let h = self.heap.alloc(Obj::Struct {
            def,
            fields: vec![Value::Unit; nfields],
        });
        self.stack.push(Value::Obj(h));
    }

    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_for_prep(&mut self, ip: usize) -> Result<(), VmError> {
        let v = self.peek(0);
        let state = match v {
            Value::Obj(h) => match self.heap.get(h) {
                Obj::List(_) | Obj::Str(_) => Value::Int(0),
                Obj::Range { lo, .. } => Value::Int(*lo),
                _ => return Err(self.err_at(ip, "internal: bad iterable (VM bug)")),
            },
            _ => return Err(self.err_at(ip, "internal: bad iterable (VM bug)")),
        };
        self.stack.push(state);
        Ok(())
    }

    /// Returns the (possibly jumped) next `ip`.
    #[cfg_attr(not(monolithic_dispatch), inline(never))]
    #[cfg_attr(monolithic_dispatch, inline(always))]
    fn op_for_next(&mut self, off: i32, ip: usize) -> Result<usize, VmError> {
        let state = self.peek(0);
        let iter = self.peek(1);
        let Value::Obj(h) = iter else {
            return Err(self.err_at(ip, "internal: bad iterable (VM bug)"));
        };
        enum Next {
            Done,
            Elem(Value, Value),
            Char(char, Value),
        }
        let next = match (self.heap.get(h), state) {
            (Obj::List(items), Value::Int(i)) => {
                if (i as usize) < items.len() {
                    Next::Elem(items[i as usize], Value::Int(i + 1))
                } else {
                    Next::Done
                }
            }
            (Obj::Range { hi, inclusive, .. }, Value::Int(cur)) => {
                let in_range = if *inclusive { cur <= *hi } else { cur < *hi };
                if in_range {
                    let next_state =
                        cur.checked_add(1).map(Value::Int).unwrap_or(Value::Unit);
                    Next::Elem(Value::Int(cur), next_state)
                } else {
                    Next::Done
                }
            }
            (Obj::Range { .. }, Value::Unit) => Next::Done,
            (Obj::Str(s), Value::Int(i)) => {
                let i = i as usize;
                match s[i..].chars().next() {
                    Some(c) => Next::Char(c, Value::Int((i + c.len_utf8()) as i64)),
                    None => Next::Done,
                }
            }
            _ => return Err(self.err_at(ip, "internal: bad iterable (VM bug)")),
        };
        match next {
            Next::Done => Ok((ip as i64 + off as i64) as usize),
            Next::Elem(elem, new_state) => {
                let len = self.stack.len();
                self.stack[len - 1] = new_state;
                self.stack.push(elem);
                Ok(ip)
            }
            Next::Char(c, new_state) => {
                let sv = self.char_str(c);
                let len = self.stack.len();
                self.stack[len - 1] = new_state;
                self.stack.push(sv);
                Ok(ip)
            }
        }
    }

    fn op_add(&mut self, ip: usize) -> Result<(), VmError> {
        let b = self.peek(0);
        let a = self.peek(1);
        let r = match (a, b) {
            (Value::Int(x), Value::Int(y)) => Value::Int(
                x.checked_add(y).ok_or_else(|| self.err_at(ip, "integer overflow"))?,
            ),
            (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
            (Value::Obj(x), Value::Obj(y)) => {
                let (Obj::Str(sx), Obj::Str(sy)) = (self.heap.get(x), self.heap.get(y)) else {
                    return Err(self.err_at(ip, "internal: bad `+` operands (VM bug)"));
                };
                let mut s = String::with_capacity(sx.len() + sy.len());
                s.push_str(sx);
                s.push_str(sy);
                self.alloc_str(s)
            }
            _ => return Err(self.err_at(ip, "internal: bad `+` operands (VM bug)")),
        };
        let n = self.stack.len() - 2;
        self.stack[n] = r;
        self.stack.truncate(n + 1);
        Ok(())
    }

    fn op_arith(&mut self, op: Op, ip: usize) -> Result<(), VmError> {
        let b = self.peek(0);
        let a = self.peek(1);
        let r = match (a, b) {
            (Value::Int(x), Value::Int(y)) => {
                let v = match op {
                    Op::Sub => x.checked_sub(y),
                    Op::Mul => x.checked_mul(y),
                    Op::Div => {
                        if y == 0 {
                            return Err(self.err_at(ip, "division by zero"));
                        }
                        x.checked_div(y)
                    }
                    Op::Rem => {
                        if y == 0 {
                            return Err(self.err_at(ip, "modulo by zero"));
                        }
                        x.checked_rem(y)
                    }
                    _ => unreachable!(),
                };
                Value::Int(v.ok_or_else(|| self.err_at(ip, "integer overflow"))?)
            }
            (Value::Float(x), Value::Float(y)) => Value::Float(match op {
                Op::Sub => x - y,
                Op::Mul => x * y,
                Op::Div => x / y,
                _ => return Err(self.err_at(ip, "internal: `%` on Float (VM bug)")),
            }),
            _ => return Err(self.err_at(ip, "internal: bad arithmetic operands (VM bug)")),
        };
        let n = self.stack.len() - 2;
        self.stack[n] = r;
        self.stack.truncate(n + 1);
        Ok(())
    }

    /// Bitwise ops (v0.7): Int-only by the checker. `>>` is arithmetic
    /// (sign-extending), matching the two's-complement Int; shift counts
    /// outside 0..=63 panic rather than quietly wrapping.
    fn op_bitwise(&mut self, op: Op, ip: usize) -> Result<(), VmError> {
        let b = self.peek(0);
        let a = self.peek(1);
        let (Value::Int(x), Value::Int(y)) = (a, b) else {
            return Err(self.err_at(ip, "internal: bad bitwise operands (VM bug)"));
        };
        let v = match op {
            Op::BitAnd => x & y,
            Op::BitOr => x | y,
            Op::BitXor => x ^ y,
            Op::Shl | Op::Shr => {
                if !(0..64).contains(&y) {
                    return Err(self.err_at(
                        ip,
                        format!("shift amount out of range: {y} (must be 0..=63)"),
                    ));
                }
                if matches!(op, Op::Shl) {
                    x.wrapping_shl(y as u32)
                } else {
                    x >> y
                }
            }
            _ => unreachable!(),
        };
        let n = self.stack.len() - 2;
        self.stack[n] = Value::Int(v);
        self.stack.truncate(n + 1);
        Ok(())
    }

    fn compare(&self, a: Value, b: Value) -> Result<std::cmp::Ordering, String> {
        use std::cmp::Ordering;
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(x.cmp(&y)),
            (Value::Float(x), Value::Float(y)) => {
                Ok(x.partial_cmp(&y).unwrap_or(Ordering::Greater))
            }
            (Value::Obj(x), Value::Obj(y)) => match (self.heap.get(x), self.heap.get(y)) {
                (Obj::Str(sx), Obj::Str(sy)) => Ok(sx.cmp(sy)),
                _ => Err("internal: bad comparison operands (VM bug)".into()),
            },
            _ => Err("internal: bad comparison operands (VM bug)".into()),
        }
    }

    // ------------------------------------------------------------------
    // Indexing
    // ------------------------------------------------------------------

    fn index_get(&mut self, base: Value, idx: Value, ip: usize) -> Result<Value, VmError> {
        let Value::Obj(h) = base else {
            return Err(self.err_at(ip, "internal: bad index base (VM bug)"));
        };
        match self.heap.get(h) {
            Obj::List(items) => {
                let Value::Int(i) = idx else {
                    return Err(self.err_at(ip, "internal: bad list index (VM bug)"));
                };
                if i < 0 || i as usize >= items.len() {
                    let len = items.len();
                    return Err(self.err_at(
                        ip,
                        format!("list index out of bounds: index {i}, length {len}"),
                    ));
                }
                Ok(items[i as usize])
            }
            Obj::Map(_) => {
                let hash = self.hash_value(idx, 0).map_err(|m| self.err_at(ip, m))?;
                let Obj::Map(m) = self.heap.get(h) else { unreachable!() };
                let found = self.map_find(m, hash, idx).map_err(|m| self.err_at(ip, m))?;
                match found {
                    Some(i) => {
                        let Obj::Map(m) = self.heap.get(h) else { unreachable!() };
                        Ok(m.entries[i as usize].2)
                    }
                    None => {
                        self.sync_ip(ip);
                        let ks = self.display_value(idx)?;
                        Err(self.error(format!("key not found in map: {ks}")))
                    }
                }
            }
            _ => Err(self.err_at(ip, "internal: bad index base (VM bug)")),
        }
    }

    fn index_set(&mut self, base: Value, idx: Value, v: Value, ip: usize) -> Result<(), VmError> {
        let Value::Obj(h) = base else {
            return Err(self.err_at(ip, "internal: bad index base (VM bug)"));
        };
        match self.heap.get(h) {
            Obj::List(items) => {
                let Value::Int(i) = idx else {
                    return Err(self.err_at(ip, "internal: bad list index (VM bug)"));
                };
                let len = items.len();
                if i < 0 || i as usize >= len {
                    return Err(self.err_at(
                        ip,
                        format!("list index out of bounds: index {i}, length {len}"),
                    ));
                }
                match self.heap.get_mut(h) {
                    Obj::List(items) => items[i as usize] = v,
                    _ => unreachable!(),
                }
                Ok(())
            }
            Obj::Map(_) => {
                self.sync_ip(ip);
                self.map_insert(h, idx, v)?;
                Ok(())
            }
            _ => Err(self.err_at(ip, "internal: bad index base (VM bug)")),
        }
    }

    /// Find an entry index in a map by hash + structural equality.
    pub fn map_find(&self, m: &FMap, hash: u64, key: Value) -> Result<Option<u32>, String> {
        for &i in m.candidates(hash) {
            let (_, k, _) = m.entries[i as usize];
            if self.value_eq(k, key, 0)? {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// Insert/overwrite a map entry; returns the previous value if any.
    pub fn map_insert(&mut self, map_h: Handle, key: Value, v: Value) -> Result<Option<Value>, VmError> {
        let hash = self.hash_value(key, 0).map_err(|m| self.error(m))?;
        let found = {
            let Obj::Map(m) = self.heap.get(map_h) else {
                return Err(self.error("internal: bad map (VM bug)"));
            };
            self.map_find(m, hash, key).map_err(|m| self.error(m))?
        };
        let Obj::Map(m) = self.heap.get_mut(map_h) else { unreachable!() };
        match found {
            Some(i) => Ok(Some(m.set_at(i, v))),
            None => {
                m.push(hash, key, v);
                Ok(None)
            }
        }
    }

    // ------------------------------------------------------------------
    // Structural equality / hashing / display
    // ------------------------------------------------------------------

    /// Deep structural equality, iterative (no Rust-stack recursion in the
    /// spine). Cycles are handled coinductively: a pair of objects already
    /// under comparison is assumed equal, so isomorphic cyclic structures
    /// compare equal and the walk always terminates. NaN follows IEEE-754
    /// (a container holding NaN is unequal even to itself).
    pub fn value_eq(&self, a: Value, b: Value, _depth: u32) -> Result<bool, String> {
        // Scalar fast path: no worklist allocation for the common cases
        // (Int==Int in hot loops, literal pattern tests).
        match (a, b) {
            (Value::Unit, Value::Unit) => return Ok(true),
            (Value::Bool(x), Value::Bool(y)) => return Ok(x == y),
            (Value::Int(x), Value::Int(y)) => return Ok(x == y),
            (Value::Float(x), Value::Float(y)) => return Ok(x == y),
            (Value::Native(_), _) | (_, Value::Native(_)) => {
                return Err("cannot compare functions".into())
            }
            (Value::Obj(x), Value::Obj(y)) => {
                if let (Obj::Str(sx), Obj::Str(sy)) = (self.heap.get(x), self.heap.get(y)) {
                    return Ok(sx == sy);
                }
            }
            _ => return Ok(false),
        }
        self.value_eq_impl(a, b, 0)
    }

    fn value_eq_impl(&self, a: Value, b: Value, map_depth: u32) -> Result<bool, String> {
        // Rust recursion happens only per *map* nesting level (map keys need
        // an immediate equality decision); everything else is a worklist.
        if map_depth > 64 {
            return Err("map nesting exceeds 64 levels in comparison".into());
        }
        let mut in_progress: std::collections::HashSet<(Handle, Handle)> =
            std::collections::HashSet::new();
        let mut work: Vec<(Value, Value)> = vec![(a, b)];
        while let Some((a, b)) = work.pop() {
            match (a, b) {
                (Value::Unit, Value::Unit) => {}
                (Value::Bool(x), Value::Bool(y)) => {
                    if x != y {
                        return Ok(false);
                    }
                }
                (Value::Int(x), Value::Int(y)) => {
                    if x != y {
                        return Ok(false);
                    }
                }
                (Value::Float(x), Value::Float(y)) => {
                    if x != y {
                        return Ok(false);
                    }
                }
                (Value::Native(_), _) | (_, Value::Native(_)) => {
                    return Err("cannot compare functions".into())
                }
                (Value::Obj(x), Value::Obj(y)) => {
                    match (self.heap.get(x), self.heap.get(y)) {
                        (Obj::Closure { .. }, _) | (_, Obj::Closure { .. }) => {
                            return Err("cannot compare functions".into())
                        }
                        (Obj::Worker(_), _) | (_, Obj::Worker(_)) => {
                            return Err("cannot compare workers".into())
                        }
                        (Obj::Window(_), _) | (_, Obj::Window(_)) => {
                            return Err("cannot compare windows".into())
                        }
                        (Obj::Str(sx), Obj::Str(sy)) => {
                            if sx != sy {
                                return Ok(false);
                            }
                        }
                        (Obj::Bytes(bx), Obj::Bytes(by)) => {
                            if bx != by {
                                return Ok(false);
                            }
                        }
                        (
                            Obj::Range { lo: l1, hi: h1, inclusive: i1 },
                            Obj::Range { lo: l2, hi: h2, inclusive: i2 },
                        ) => {
                            if !(l1 == l2 && h1 == h2 && i1 == i2) {
                                return Ok(false);
                            }
                        }
                        (Obj::List(xs), Obj::List(ys)) | (Obj::Tuple(xs), Obj::Tuple(ys)) => {
                            if xs.len() != ys.len() {
                                return Ok(false);
                            }
                            if in_progress.insert((x, y)) {
                                work.extend(xs.iter().copied().zip(ys.iter().copied()));
                            }
                        }
                        (
                            Obj::Struct { def: d1, fields: f1 },
                            Obj::Struct { def: d2, fields: f2 },
                        ) => {
                            if d1 != d2 {
                                return Ok(false);
                            }
                            if in_progress.insert((x, y)) {
                                work.extend(f1.iter().copied().zip(f2.iter().copied()));
                            }
                        }
                        (
                            Obj::Variant { def: d1, variant: v1, fields: f1 },
                            Obj::Variant { def: d2, variant: v2, fields: f2 },
                        ) => {
                            if d1 != d2 || v1 != v2 {
                                return Ok(false);
                            }
                            if in_progress.insert((x, y)) {
                                work.extend(f1.iter().copied().zip(f2.iter().copied()));
                            }
                        }
                        (Obj::Map(mx), Obj::Map(my)) => {
                            if mx.len() != my.len() {
                                return Ok(false);
                            }
                            if in_progress.insert((x, y)) {
                                for (hash, k, v) in &mx.entries {
                                    let mut found = None;
                                    for &cand in my.candidates(*hash) {
                                        let (_, k2, _) = my.entries[cand as usize];
                                        if self.value_eq_impl(*k, k2, map_depth + 1)? {
                                            found = Some(cand);
                                            break;
                                        }
                                    }
                                    let Some(i) = found else { return Ok(false) };
                                    let (_, _, v2) = my.entries[i as usize];
                                    work.push((*v, v2));
                                }
                            }
                        }
                        _ => return Ok(false),
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Structural hash, iterative over a DFS of the value (so equal-by-
    /// structure values hash equally even when one shares subobjects and the
    /// other doesn't). Cyclic or absurdly large keys exhaust the node budget
    /// and error; map entries combine order-insensitively via nested calls.
    pub fn hash_value(&self, v: Value, _depth: u32) -> Result<u64, String> {
        // Scalar/string fast path: identical hashes to the worklist walk
        // below, without allocating it (Int and Str keys dominate hot maps).
        match v {
            Value::Unit => return Ok(mix(FNV_OFFSET, 1)),
            Value::Bool(b) => return Ok(mix(FNV_OFFSET, 2 + b as u64)),
            Value::Int(i) => return Ok(mix(mix(FNV_OFFSET, 4), i as u64)),
            Value::Float(f) => {
                let bits = if f == 0.0 { 0u64 } else { f.to_bits() };
                return Ok(mix(mix(FNV_OFFSET, 5), bits));
            }
            Value::Undefined => return Ok(mix(FNV_OFFSET, 6)),
            Value::Native(_) => return Err("functions cannot be used as map keys".into()),
            Value::Obj(h) => {
                if let Obj::Str(s) = self.heap.get(h) {
                    let mut acc = mix(FNV_OFFSET, 7);
                    for b in s.bytes() {
                        acc = mix(acc, b as u64);
                    }
                    return Ok(acc);
                }
            }
        }
        let mut budget: u32 = 4_000_000;
        self.hash_value_impl(v, 0, &mut budget)
    }

    fn hash_value_impl(
        &self,
        v: Value,
        map_depth: u32,
        budget: &mut u32,
    ) -> Result<u64, String> {
        if map_depth > 64 {
            return Err("map nesting exceeds 64 levels in key".into());
        }
        let mut acc = FNV_OFFSET;
        let mut work: Vec<Value> = vec![v];
        while let Some(v) = work.pop() {
            if *budget == 0 {
                return Err("map key is too large or cyclic".into());
            }
            *budget -= 1;
            match v {
                Value::Unit => acc = mix(acc, 1),
                Value::Bool(b) => acc = mix(acc, 2 + b as u64),
                Value::Int(i) => acc = mix(mix(acc, 4), i as u64),
                Value::Float(f) => {
                    let bits = if f == 0.0 { 0u64 } else { f.to_bits() };
                    acc = mix(mix(acc, 5), bits);
                }
                Value::Undefined => acc = mix(acc, 6),
                Value::Native(_) => {
                    return Err("functions cannot be used as map keys".into())
                }
                Value::Obj(h) => match self.heap.get(h) {
                    Obj::Str(s) => {
                        acc = mix(acc, 7);
                        for b in s.bytes() {
                            acc = mix(acc, b as u64);
                        }
                    }
                    Obj::Bytes(bs) => {
                        acc = mix(mix(acc, 14), bs.len() as u64);
                        for b in bs {
                            acc = mix(acc, *b as u64);
                        }
                    }
                    Obj::List(items) => {
                        acc = mix(mix(acc, 8), items.len() as u64);
                        work.extend(items.iter().rev().copied());
                    }
                    Obj::Tuple(items) => {
                        acc = mix(mix(acc, 9), items.len() as u64);
                        work.extend(items.iter().rev().copied());
                    }
                    Obj::Map(m) => {
                        // Order-insensitive combine, matching set-like map
                        // equality: sum the entry hashes.
                        let mut sum: u64 = 0;
                        for (_, k, v) in &m.entries {
                            let e = mix(
                                mix(FNV_OFFSET, self.hash_value_impl(*k, map_depth + 1, budget)?),
                                self.hash_value_impl(*v, map_depth + 1, budget)?,
                            );
                            sum = sum.wrapping_add(e);
                        }
                        acc = mix(mix(acc, 10), sum);
                    }
                    Obj::Struct { def, fields } => {
                        acc = mix(mix(acc, 11), *def as u64);
                        work.extend(fields.iter().rev().copied());
                    }
                    Obj::Variant { def, variant, fields } => {
                        acc = mix(mix(mix(acc, 12), *def as u64), *variant as u64);
                        work.extend(fields.iter().rev().copied());
                    }
                    Obj::Range { lo, hi, inclusive } => {
                        acc = mix(
                            mix(mix(mix(acc, 13), *lo as u64), *hi as u64),
                            *inclusive as u64,
                        );
                    }
                    Obj::Closure { .. } => {
                        return Err("functions cannot be used as map keys".into())
                    }
                    Obj::Worker(_) => {
                        return Err("workers cannot be used as map keys".into())
                    }
                    Obj::Window(_) => {
                        return Err("windows cannot be used as map keys".into())
                    }
                    Obj::Upvalue(_) | Obj::Free => {
                        return Err("internal: bad map key (VM bug)".into())
                    }
                },
            }
        }
        Ok(acc)
    }

    /// `str(x)` semantics: bare strings at the top level, quoted in containers.
    /// Nesting deeper than 10,000 levels renders as `...` (as do cycles).
    pub fn display_value(&self, v: Value) -> Result<String, VmError> {
        // Scalar/string fast paths: byte-identical output to `display_inner`,
        // skipping the fmt machinery and the seen-list plumbing.
        match v {
            Value::Unit => return Ok("()".to_string()),
            Value::Bool(b) => return Ok((if b { "true" } else { "false" }).to_string()),
            Value::Int(i) => {
                let mut s = String::new();
                push_int(&mut s, i);
                return Ok(s);
            }
            Value::Float(f) => return Ok(fmt_float(f)),
            Value::Obj(h) => {
                if let Obj::Str(s) = self.heap.get(h) {
                    return Ok(s.clone());
                }
            }
            _ => {}
        }
        let mut out = String::new();
        let mut seen = Vec::new();
        self.display_inner(v, true, &mut seen, &mut out, 0)
            .map_err(|m| self.error(m))?;
        Ok(out)
    }

    fn display_inner(
        &self,
        v: Value,
        top: bool,
        seen: &mut Vec<Handle>,
        out: &mut String,
        depth: u32,
    ) -> Result<(), String> {
        if depth > 10_000 {
            out.push_str("...");
            return Ok(());
        }
        match v {
            Value::Unit => out.push_str("()"),
            Value::Bool(b) => out.push_str(if b { "true" } else { "false" }),
            Value::Int(i) => push_int(out, i),
            Value::Float(f) => out.push_str(&fmt_float(f)),
            Value::Native(n) => {
                out.push_str("<fn ");
                out.push_str(n.name());
                out.push('>');
            }
            Value::Undefined => out.push_str("<undefined>"),
            Value::Obj(h) => {
                if seen.contains(&h) {
                    out.push_str("...");
                    return Ok(());
                }
                match self.heap.get(h) {
                    Obj::Bytes(bs) => {
                        out.push_str("<bytes ");
                        push_int(out, bs.len() as i64);
                        out.push('>');
                        return Ok(());
                    }
                    Obj::Worker(_) => {
                        out.push_str("<worker>");
                        return Ok(());
                    }
                    Obj::Window(_) => {
                        out.push_str("<window>");
                        return Ok(());
                    }
                    Obj::Str(s) => {
                        if top {
                            out.push_str(s);
                        } else {
                            // Copy escape-free runs whole; the escaped
                            // characters are all ASCII, so slicing one byte
                            // past a match stays on a char boundary.
                            out.push('"');
                            let mut rest = s.as_str();
                            while let Some(i) =
                                rest.find(['\n', '\t', '\r', '"', '\\'])
                            {
                                out.push_str(&rest[..i]);
                                out.push_str(match rest.as_bytes()[i] {
                                    b'\n' => "\\n",
                                    b'\t' => "\\t",
                                    b'\r' => "\\r",
                                    b'"' => "\\\"",
                                    _ => "\\\\",
                                });
                                rest = &rest[i + 1..];
                            }
                            out.push_str(rest);
                            out.push('"');
                        }
                    }
                    Obj::List(items) => {
                        seen.push(h);
                        out.push('[');
                        for (i, it) in items.iter().enumerate() {
                            if i > 0 {
                                out.push_str(", ");
                            }
                            self.display_inner(*it, false, seen, out, depth + 1)?;
                        }
                        out.push(']');
                        seen.pop();
                    }
                    Obj::Tuple(items) => {
                        seen.push(h);
                        out.push('(');
                        for (i, it) in items.iter().enumerate() {
                            if i > 0 {
                                out.push_str(", ");
                            }
                            self.display_inner(*it, false, seen, out, depth + 1)?;
                        }
                        out.push(')');
                        seen.pop();
                    }
                    Obj::Map(m) => {
                        seen.push(h);
                        if m.is_empty() {
                            out.push_str("{:}");
                        } else {
                            out.push('{');
                            for (i, (_, k, v)) in m.entries.iter().enumerate() {
                                if i > 0 {
                                    out.push_str(", ");
                                }
                                self.display_inner(*k, false, seen, out, depth + 1)?;
                                out.push_str(": ");
                                self.display_inner(*v, false, seen, out, depth + 1)?;
                            }
                            out.push('}');
                        }
                        seen.pop();
                    }
                    Obj::Struct { def, fields } => {
                        seen.push(h);
                        let RtDef::Struct { name, fields: fnames } =
                            &self.program.defs[*def as usize]
                        else {
                            return Err("internal: bad struct def (VM bug)".into());
                        };
                        out.push_str(name);
                        out.push_str(" { ");
                        for (i, (fv, fname)) in fields.iter().zip(fnames).enumerate() {
                            if i > 0 {
                                out.push_str(", ");
                            }
                            out.push_str(fname);
                            out.push_str(": ");
                            self.display_inner(*fv, false, seen, out, depth + 1)?;
                        }
                        out.push_str(" }");
                        seen.pop();
                    }
                    Obj::Variant { def, variant, fields } => {
                        seen.push(h);
                        let RtDef::Enum { variants, .. } = &self.program.defs[*def as usize]
                        else {
                            return Err("internal: bad enum def (VM bug)".into());
                        };
                        out.push_str(&variants[*variant as usize].0);
                        if !fields.is_empty() {
                            out.push('(');
                            for (i, fv) in fields.iter().enumerate() {
                                if i > 0 {
                                    out.push_str(", ");
                                }
                                self.display_inner(*fv, false, seen, out, depth + 1)?;
                            }
                            out.push(')');
                        }
                        seen.pop();
                    }
                    Obj::Range { lo, hi, inclusive } => {
                        push_int(out, *lo);
                        out.push_str(if *inclusive { "..=" } else { ".." });
                        push_int(out, *hi);
                    }
                    Obj::Closure { proto, .. } => {
                        let name = &self.program.protos[*proto as usize].name;
                        if name == "<lambda>" {
                            out.push_str("<fn>");
                        } else {
                            out.push_str("<fn ");
                            out.push_str(name);
                            out.push('>');
                        }
                    }
                    Obj::Upvalue(_) | Obj::Free => out.push_str("<internal>"),
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Misc runtime services for natives
    // ------------------------------------------------------------------

    pub fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn rng_next(&mut self) -> f64 {
        let r = self.rng_next_u64();
        (r >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn rng_next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// A uniform integer in `[lo, hi]` (inclusive). Callers must check
    /// `lo <= hi`. Lemire's widening-multiply method **with rejection**, so
    /// the distribution is exactly uniform even for spans near 2^64 (the
    /// rejection probability is `(2^64 mod span) / 2^64` per draw).
    pub fn rng_range(&mut self, lo: i64, hi: i64) -> i64 {
        // `span == 0` encodes the full 2^64 range (lo = i64::MIN, hi = i64::MAX).
        let span = hi.wrapping_sub(lo).wrapping_add(1) as u64;
        if span == 0 {
            return self.rng_next_u64() as i64;
        }
        let threshold = span.wrapping_neg() % span;
        loop {
            let m = (self.rng_next_u64() as u128) * (span as u128);
            if (m as u64) >= threshold {
                return lo.wrapping_add((m >> 64) as i64);
            }
        }
    }

    pub fn rng_seed(&mut self, seed: i64) {
        // SplitMix64: adjacent seeds must produce unrelated streams (the old
        // `seed | 1` collapsed 2k and 2k+1 to the same state), and the state
        // must never be zero (xorshift's fixed point).
        let mut z = (seed as u64).wrapping_add(0x9E3779B97F4A7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        self.rng = if z == 0 { 0x9E3779B97F4A7C15 } else { z };
    }
}

/// Append `n`'s decimal form — byte-identical to `write!(out, "{n}")` —
/// without going through `fmt`'s `Arguments` machinery (hot in container
/// display).
fn push_int(out: &mut String, n: i64) {
    let mut buf = [0u8; 20]; // i64::MIN is 20 bytes: 19 digits + sign
    let mut i = buf.len();
    let mut m = n.unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (m % 10) as u8;
        m /= 10;
        if m == 0 {
            break;
        }
    }
    if n < 0 {
        i -= 1;
        buf[i] = b'-';
    }
    out.push_str(std::str::from_utf8(&buf[i..]).expect("ASCII digits"));
}

/// Float display: shortest round-trip digits; positional notation (`123.5`,
/// always with a decimal point) for magnitudes in `[1e-4, 1e16)` and for
/// zero, exponent notation (`1e300`, `2.5e-7`) outside that range.
pub fn fmt_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let mag = f.abs();
    if f != 0.0 && !(1e-4..1e16).contains(&mag) {
        return format!("{f:e}");
    }
    let mut positional = format!("{f}");
    if !positional.contains('.') {
        positional.push_str(".0");
    }
    positional
}
