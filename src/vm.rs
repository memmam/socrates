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
use crate::value::{FMap, Handle, Heap, Obj, Upval, Value};

const MAX_FRAMES: usize = 4096;

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
    /// Cached zero-upvalue closures for `PushFn`.
    fn_closures: Vec<Option<Value>>,
    pub temp_roots: Vec<Value>,
    pub out: Box<dyn Write>,
    start: Instant,
    rng: u64,
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
            fn_closures: Vec::new(),
            temp_roots: Vec::new(),
            out,
            start: Instant::now(),
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
        let entry = self.program.entry;
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
        let mut work: Vec<Handle> = Vec::new();
        for v in &self.stack {
            self.heap.mark_value(*v, &mut work);
        }
        for v in &self.globals {
            self.heap.mark_value(*v, &mut work);
        }
        for v in &self.temp_roots {
            self.heap.mark_value(*v, &mut work);
        }
        for v in &self.interned {
            self.heap.mark_value(*v, &mut work);
        }
        for v in self.fn_closures.iter().flatten() {
            self.heap.mark_value(*v, &mut work);
        }
        for h in &self.open_upvalues {
            self.heap.mark_handle(*h, &mut work);
        }
        for f in &self.frames {
            if let Some(h) = f.closure {
                self.heap.mark_handle(h, &mut work);
            }
        }
        self.heap.trace(&mut work);
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

    fn as_bool(&self, v: Value) -> Result<bool, VmError> {
        match v {
            Value::Bool(b) => Ok(b),
            _ => Err(self.error("internal: expected Bool (VM bug)")),
        }
    }

    /// Native calling convention: argument `i` of `argc` (stack-top relative).
    pub fn native_arg(&self, argc: u8, i: u8) -> Value {
        self.stack[self.stack.len() - argc as usize + i as usize]
    }

    /// Pop a native call's arguments and push its result.
    pub fn finish_native(&mut self, argc: u8, result: Value) {
        let n = self.stack.len() - argc as usize;
        self.stack.truncate(n);
        self.stack.push(result);
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
        let mut kept = Vec::with_capacity(self.open_upvalues.len());
        for &h in &self.open_upvalues {
            let close = match self.heap.get(h) {
                Obj::Upvalue(Upval::Open(i)) if *i >= from => Some(*i),
                _ => None,
            };
            match close {
                Some(i) => {
                    let v = self.stack[i];
                    *self.heap.get_mut(h) = Obj::Upvalue(Upval::Closed(v));
                }
                None => kept.push(h),
            }
        }
        self.open_upvalues = kept;
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
        if self.frames.len() >= MAX_FRAMES {
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
        loop {
            let frame = self.frames.last_mut().expect("no frame");
            let proto_idx = frame.proto as usize;
            let ip = frame.ip;
            frame.ip += 1;
            let base = frame.base;
            let op = self.program.protos[proto_idx].code[ip];

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
                Op::SetLocal(s) => {
                    let v = self.pop();
                    self.stack[base + s as usize] = v;
                }
                Op::GetGlobal(g) => {
                    let v = self.globals[g as usize];
                    if matches!(v, Value::Undefined) {
                        let name = self
                            .program
                            .global_names
                            .get(g as usize)
                            .cloned()
                            .unwrap_or_default();
                        return Err(
                            self.error(format!("global `{name}` used before initialization"))
                        );
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
                        Obj::Closure { upvals, .. } => upvals[i as usize],
                        _ => return Err(self.error("internal: bad closure (VM bug)")),
                    };
                    let v = match self.heap.get(uh) {
                        Obj::Upvalue(Upval::Open(idx)) => self.stack[*idx],
                        Obj::Upvalue(Upval::Closed(v)) => *v,
                        _ => return Err(self.error("internal: bad upvalue (VM bug)")),
                    };
                    self.stack.push(v);
                }
                Op::SetUpvalue(i) => {
                    let v = self.pop();
                    let closure = self.frames.last().unwrap().closure.expect("no closure");
                    let uh = match self.heap.get(closure) {
                        Obj::Closure { upvals, .. } => upvals[i as usize],
                        _ => return Err(self.error("internal: bad closure (VM bug)")),
                    };
                    match self.heap.get_mut(uh) {
                        Obj::Upvalue(u @ Upval::Open(_)) => {
                            if let Upval::Open(idx) = *u {
                                self.stack[idx] = v;
                            }
                        }
                        Obj::Upvalue(u) => *u = Upval::Closed(v),
                        _ => return Err(self.error("internal: bad upvalue (VM bug)")),
                    }
                }

                Op::PushFn(p) => {
                    if let Some(v) = self.fn_closures[p as usize] {
                        self.stack.push(v);
                    } else {
                        let h = self.alloc(Obj::Closure { proto: p, upvals: Vec::new() });
                        let v = Value::Obj(h);
                        self.fn_closures[p as usize] = Some(v);
                        self.stack.push(v);
                    }
                }
                Op::PushNative(n) => self.stack.push(Value::Native(n)),
                Op::Closure(p) => {
                    self.gc_checkpoint();
                    let descs = self.program.protos[p as usize].upvals.clone();
                    let parent_closure = self.frames.last().unwrap().closure;
                    let mut upvals = Vec::with_capacity(descs.len());
                    for d in descs {
                        if d.from_local {
                            upvals.push(self.capture_upvalue(base + d.index as usize));
                        } else {
                            let pc = parent_closure.expect("upvalue chain without closure");
                            let uh = match self.heap.get(pc) {
                                Obj::Closure { upvals, .. } => upvals[d.index as usize],
                                _ => return Err(self.error("internal: bad closure (VM bug)")),
                            };
                            upvals.push(uh);
                        }
                    }
                    let h = self.heap.alloc(Obj::Closure { proto: p, upvals });
                    self.stack.push(Value::Obj(h));
                }

                Op::Jump(off) => {
                    let f = self.frames.last_mut().unwrap();
                    f.ip = (f.ip as i64 + off as i64) as usize;
                }
                Op::JumpIfFalse(off) => {
                    let v = self.pop();
                    if !self.as_bool(v)? {
                        let f = self.frames.last_mut().unwrap();
                        f.ip = (f.ip as i64 + off as i64) as usize;
                    }
                }
                Op::JumpIfFalsePeek(off) => {
                    let v = self.peek(0);
                    if !self.as_bool(v)? {
                        let f = self.frames.last_mut().unwrap();
                        f.ip = (f.ip as i64 + off as i64) as usize;
                    }
                }
                Op::JumpIfTruePeek(off) => {
                    let v = self.peek(0);
                    if self.as_bool(v)? {
                        let f = self.frames.last_mut().unwrap();
                        f.ip = (f.ip as i64 + off as i64) as usize;
                    }
                }

                Op::Call(argc) => {
                    let callee = self.peek(argc as usize);
                    match callee {
                        Value::Obj(h) => {
                            let proto = match self.heap.get(h) {
                                Obj::Closure { proto, .. } => *proto,
                                _ => {
                                    return Err(self.error("value is not callable"));
                                }
                            };
                            self.push_frame(proto, Some(h), argc, true)?;
                        }
                        Value::Native(n) => {
                            crate::natives::call_native(self, n, argc)?;
                            // Remove the callee slot beneath the result.
                            let result = self.pop();
                            self.pop();
                            self.stack.push(result);
                        }
                        _ => return Err(self.error("value is not callable")),
                    }
                }
                Op::CallFn(p, argc) => {
                    self.push_frame(p, None, argc, false)?;
                }
                Op::TailCallFn(p, argc) => {
                    self.reuse_frame(p, None, argc)?;
                }
                Op::TailCall(argc) => {
                    let callee = self.peek(argc as usize);
                    match callee {
                        Value::Obj(h) => {
                            let proto = match self.heap.get(h) {
                                Obj::Closure { proto, .. } => *proto,
                                _ => {
                                    return Err(self.error("value is not callable"));
                                }
                            };
                            self.reuse_frame(proto, Some(h), argc)?;
                        }
                        Value::Native(n) => {
                            // A native in tail position pushes no frame; call
                            // it and return its result like `Op::Return`.
                            crate::natives::call_native(self, n, argc)?;
                            let result = self.pop();
                            self.pop(); // the callee slot
                            let f = self.frames.pop().unwrap();
                            self.close_upvalues(f.base);
                            let cut = f.base - usize::from(f.callee_slot);
                            self.stack.truncate(cut);
                            self.stack.push(result);
                            if self.frames.len() < min_frames {
                                return Ok(());
                            }
                        }
                        _ => return Err(self.error("value is not callable")),
                    }
                }
                Op::CallNative(n, argc) => {
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
                }

                Op::Add => self.op_add()?,
                Op::Sub => self.op_arith(op)?,
                Op::Mul => self.op_arith(op)?,
                Op::Div => self.op_arith(op)?,
                Op::Rem => self.op_arith(op)?,
                Op::Neg => {
                    let v = self.pop();
                    let r = match v {
                        Value::Int(i) => Value::Int(
                            i.checked_neg().ok_or_else(|| self.error("integer overflow"))?,
                        ),
                        Value::Float(f) => Value::Float(-f),
                        _ => return Err(self.error("internal: bad negate operand (VM bug)")),
                    };
                    self.stack.push(r);
                }
                Op::Not => {
                    let v = self.pop();
                    let b = self.as_bool(v)?;
                    self.stack.push(Value::Bool(!b));
                }
                Op::Eq => {
                    let b = self.peek(0);
                    let a = self.peek(1);
                    let eq = self.value_eq(a, b, 0).map_err(|m| self.error(m))?;
                    self.stack.truncate(self.stack.len() - 2);
                    self.stack.push(Value::Bool(eq));
                }
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let b = self.pop();
                    let a = self.pop();
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
                        let ord = self.compare(a, b)?;
                        match op {
                            Op::Lt => ord.is_lt(),
                            Op::Le => ord.is_le(),
                            Op::Gt => ord.is_gt(),
                            _ => ord.is_ge(),
                        }
                    };
                    self.stack.push(Value::Bool(r));
                }

                Op::ToString => {
                    let v = self.peek(0);
                    let s = self.display_value(v)?;
                    let sv = self.alloc_str(s);
                    self.pop();
                    self.stack.push(sv);
                }
                Op::Concat(n) => {
                    let n = n as usize;
                    let mut s = String::new();
                    for i in (0..n).rev() {
                        let part = self.peek(i);
                        s.push_str(&self.str_of(part)?);
                    }
                    let sv = self.alloc_str(s);
                    let len = self.stack.len() - n;
                    self.stack.truncate(len);
                    self.stack.push(sv);
                }

                Op::MakeList(n) => {
                    self.gc_checkpoint();
                    let start = self.stack.len() - n as usize;
                    let items: Vec<Value> = self.stack.split_off(start);
                    let h = self.heap.alloc(Obj::List(items));
                    self.stack.push(Value::Obj(h));
                }
                Op::MakeMap(n) => {
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
                }
                Op::MakeTuple(n) => {
                    self.gc_checkpoint();
                    let start = self.stack.len() - n as usize;
                    let items: Vec<Value> = self.stack.split_off(start);
                    let h = self.heap.alloc(Obj::Tuple(items));
                    self.stack.push(Value::Obj(h));
                }
                Op::MakeRange { inclusive } => {
                    self.gc_checkpoint();
                    let hi = self.pop();
                    let lo = self.pop();
                    let (Value::Int(lo), Value::Int(hi)) = (lo, hi) else {
                        return Err(self.error("internal: bad range bounds (VM bug)"));
                    };
                    let h = self.heap.alloc(Obj::Range { lo, hi, inclusive });
                    self.stack.push(Value::Obj(h));
                }
                Op::MakeStructEmpty(def) => {
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
                Op::StructSetField(i) => {
                    let v = self.pop();
                    let s = self.peek(0);
                    let Value::Obj(h) = s else {
                        return Err(self.error("internal: bad struct (VM bug)"));
                    };
                    match self.heap.get_mut(h) {
                        Obj::Struct { fields, .. } => fields[i as usize] = v,
                        _ => return Err(self.error("internal: bad struct (VM bug)")),
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
                        return Err(self.error("internal: bad struct (VM bug)"));
                    };
                    let v = match self.heap.get(h) {
                        Obj::Struct { fields, .. } => fields[i as usize],
                        _ => return Err(self.error("internal: bad struct (VM bug)")),
                    };
                    self.stack.push(v);
                }
                Op::SetField(i) => {
                    let v = self.pop();
                    let s = self.pop();
                    let Value::Obj(h) = s else {
                        return Err(self.error("internal: bad struct (VM bug)"));
                    };
                    match self.heap.get_mut(h) {
                        Obj::Struct { fields, .. } => fields[i as usize] = v,
                        _ => return Err(self.error("internal: bad struct (VM bug)")),
                    }
                }
                Op::TupleGet(i) => {
                    let t = self.pop();
                    let Value::Obj(h) = t else {
                        return Err(self.error("internal: bad tuple (VM bug)"));
                    };
                    let v = match self.heap.get(h) {
                        Obj::Tuple(items) => items[i as usize],
                        _ => return Err(self.error("internal: bad tuple (VM bug)")),
                    };
                    self.stack.push(v);
                }
                Op::GetVariantField(i) => {
                    let t = self.pop();
                    let Value::Obj(h) = t else {
                        return Err(self.error("internal: bad enum value (VM bug)"));
                    };
                    let v = match self.heap.get(h) {
                        Obj::Variant { fields, .. } => fields[i as usize],
                        _ => return Err(self.error("internal: bad enum value (VM bug)")),
                    };
                    self.stack.push(v);
                }
                Op::TestVariant(i) => {
                    let t = self.peek(0);
                    let Value::Obj(h) = t else {
                        return Err(self.error("internal: bad enum value (VM bug)"));
                    };
                    let b = match self.heap.get(h) {
                        Obj::Variant { variant, .. } => *variant == i as u32,
                        _ => return Err(self.error("internal: bad enum value (VM bug)")),
                    };
                    self.stack.push(Value::Bool(b));
                }

                Op::Index => {
                    let idx = self.pop();
                    let base_v = self.pop();
                    let v = self.index_get(base_v, idx)?;
                    self.stack.push(v);
                }
                Op::IndexSet => {
                    let v = self.pop();
                    let idx = self.pop();
                    let base_v = self.pop();
                    self.index_set(base_v, idx, v)?;
                }

                Op::ForPrep => {
                    let v = self.peek(0);
                    let state = match v {
                        Value::Obj(h) => match self.heap.get(h) {
                            Obj::List(_) | Obj::Str(_) => Value::Int(0),
                            Obj::Range { lo, .. } => Value::Int(*lo),
                            _ => return Err(self.error("internal: bad iterable (VM bug)")),
                        },
                        _ => return Err(self.error("internal: bad iterable (VM bug)")),
                    };
                    self.stack.push(state);
                }
                Op::ForNext(off) => {
                    let state = self.peek(0);
                    let iter = self.peek(1);
                    let Value::Obj(h) = iter else {
                        return Err(self.error("internal: bad iterable (VM bug)"));
                    };
                    enum Next {
                        Done,
                        Elem(Value, Value),
                        Char(String, Value),
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
                                let next_state = cur
                                    .checked_add(1)
                                    .map(Value::Int)
                                    .unwrap_or(Value::Unit);
                                Next::Elem(Value::Int(cur), next_state)
                            } else {
                                Next::Done
                            }
                        }
                        (Obj::Range { .. }, Value::Unit) => Next::Done,
                        (Obj::Str(s), Value::Int(i)) => {
                            let i = i as usize;
                            match s[i..].chars().next() {
                                Some(c) => Next::Char(
                                    c.to_string(),
                                    Value::Int((i + c.len_utf8()) as i64),
                                ),
                                None => Next::Done,
                            }
                        }
                        _ => return Err(self.error("internal: bad iterable (VM bug)")),
                    };
                    match next {
                        Next::Done => {
                            let f = self.frames.last_mut().unwrap();
                            f.ip = (f.ip as i64 + off as i64) as usize;
                        }
                        Next::Elem(elem, new_state) => {
                            let len = self.stack.len();
                            self.stack[len - 1] = new_state;
                            self.stack.push(elem);
                        }
                        Next::Char(c, new_state) => {
                            let sv = self.alloc_str(c);
                            let len = self.stack.len();
                            self.stack[len - 1] = new_state;
                            self.stack.push(sv);
                        }
                    }
                }

                Op::MatchFail => {
                    return Err(self.error(
                        "match did not cover the scrutinee value (all arms failed)",
                    ));
                }
            }
        }
    }

    fn op_add(&mut self) -> Result<(), VmError> {
        let b = self.peek(0);
        let a = self.peek(1);
        let r = match (a, b) {
            (Value::Int(x), Value::Int(y)) => Value::Int(
                x.checked_add(y).ok_or_else(|| self.error("integer overflow"))?,
            ),
            (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
            (Value::Obj(x), Value::Obj(y)) => {
                let (Obj::Str(sx), Obj::Str(sy)) = (self.heap.get(x), self.heap.get(y)) else {
                    return Err(self.error("internal: bad `+` operands (VM bug)"));
                };
                let mut s = String::with_capacity(sx.len() + sy.len());
                s.push_str(sx);
                s.push_str(sy);
                self.alloc_str(s)
            }
            _ => return Err(self.error("internal: bad `+` operands (VM bug)")),
        };
        self.stack.truncate(self.stack.len() - 2);
        self.stack.push(r);
        Ok(())
    }

    fn op_arith(&mut self, op: Op) -> Result<(), VmError> {
        let b = self.pop();
        let a = self.pop();
        let r = match (a, b) {
            (Value::Int(x), Value::Int(y)) => {
                let v = match op {
                    Op::Sub => x.checked_sub(y),
                    Op::Mul => x.checked_mul(y),
                    Op::Div => {
                        if y == 0 {
                            return Err(self.error("division by zero"));
                        }
                        x.checked_div(y)
                    }
                    Op::Rem => {
                        if y == 0 {
                            return Err(self.error("modulo by zero"));
                        }
                        x.checked_rem(y)
                    }
                    _ => unreachable!(),
                };
                Value::Int(v.ok_or_else(|| self.error("integer overflow"))?)
            }
            (Value::Float(x), Value::Float(y)) => Value::Float(match op {
                Op::Sub => x - y,
                Op::Mul => x * y,
                Op::Div => x / y,
                _ => return Err(self.error("internal: `%` on Float (VM bug)")),
            }),
            _ => return Err(self.error("internal: bad arithmetic operands (VM bug)")),
        };
        self.stack.push(r);
        Ok(())
    }

    fn compare(&self, a: Value, b: Value) -> Result<std::cmp::Ordering, VmError> {
        use std::cmp::Ordering;
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(x.cmp(&y)),
            (Value::Float(x), Value::Float(y)) => {
                Ok(x.partial_cmp(&y).unwrap_or(Ordering::Greater))
            }
            (Value::Obj(x), Value::Obj(y)) => match (self.heap.get(x), self.heap.get(y)) {
                (Obj::Str(sx), Obj::Str(sy)) => Ok(sx.cmp(sy)),
                _ => Err(self.error("internal: bad comparison operands (VM bug)")),
            },
            _ => Err(self.error("internal: bad comparison operands (VM bug)")),
        }
    }

    // ------------------------------------------------------------------
    // Indexing
    // ------------------------------------------------------------------

    fn index_get(&mut self, base: Value, idx: Value) -> Result<Value, VmError> {
        let Value::Obj(h) = base else {
            return Err(self.error("internal: bad index base (VM bug)"));
        };
        match self.heap.get(h) {
            Obj::List(items) => {
                let Value::Int(i) = idx else {
                    return Err(self.error("internal: bad list index (VM bug)"));
                };
                if i < 0 || i as usize >= items.len() {
                    let len = items.len();
                    return Err(self.error(format!(
                        "list index out of bounds: index {i}, length {len}"
                    )));
                }
                Ok(items[i as usize])
            }
            Obj::Map(_) => {
                let hash = self.hash_value(idx, 0).map_err(|m| self.error(m))?;
                let Obj::Map(m) = self.heap.get(h) else { unreachable!() };
                let found = self.map_find(m, hash, idx).map_err(|m| self.error(m))?;
                match found {
                    Some(i) => {
                        let Obj::Map(m) = self.heap.get(h) else { unreachable!() };
                        Ok(m.entries[i as usize].2)
                    }
                    None => {
                        let ks = self.display_value(idx)?;
                        Err(self.error(format!("key not found in map: {ks}")))
                    }
                }
            }
            _ => Err(self.error("internal: bad index base (VM bug)")),
        }
    }

    fn index_set(&mut self, base: Value, idx: Value, v: Value) -> Result<(), VmError> {
        let Value::Obj(h) = base else {
            return Err(self.error("internal: bad index base (VM bug)"));
        };
        match self.heap.get(h) {
            Obj::List(items) => {
                let Value::Int(i) = idx else {
                    return Err(self.error("internal: bad list index (VM bug)"));
                };
                let len = items.len();
                if i < 0 || i as usize >= len {
                    return Err(self.error(format!(
                        "list index out of bounds: index {i}, length {len}"
                    )));
                }
                match self.heap.get_mut(h) {
                    Obj::List(items) => items[i as usize] = v,
                    _ => unreachable!(),
                }
                Ok(())
            }
            Obj::Map(_) => {
                self.map_insert(h, idx, v)?;
                Ok(())
            }
            _ => Err(self.error("internal: bad index base (VM bug)")),
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
                        (Obj::Str(sx), Obj::Str(sy)) => {
                            if sx != sy {
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
        const FNV_OFFSET: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;
        fn mix(h: u64, x: u64) -> u64 {
            (h ^ x).wrapping_mul(FNV_PRIME)
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
        use std::fmt::Write;
        if depth > 10_000 {
            out.push_str("...");
            return Ok(());
        }
        match v {
            Value::Unit => out.push_str("()"),
            Value::Bool(b) => {
                let _ = write!(out, "{b}");
            }
            Value::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Value::Float(f) => out.push_str(&fmt_float(f)),
            Value::Native(n) => {
                let _ = write!(out, "<fn {}>", n.name());
            }
            Value::Undefined => out.push_str("<undefined>"),
            Value::Obj(h) => {
                if seen.contains(&h) {
                    out.push_str("...");
                    return Ok(());
                }
                match self.heap.get(h) {
                    Obj::Str(s) => {
                        if top {
                            out.push_str(s);
                        } else {
                            out.push('"');
                            for c in s.chars() {
                                match c {
                                    '\n' => out.push_str("\\n"),
                                    '\t' => out.push_str("\\t"),
                                    '\r' => out.push_str("\\r"),
                                    '"' => out.push_str("\\\""),
                                    '\\' => out.push_str("\\\\"),
                                    _ => out.push(c),
                                }
                            }
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
                        let _ = write!(out, "{name} {{ ");
                        for (i, (fv, fname)) in fields.iter().zip(fnames).enumerate() {
                            if i > 0 {
                                out.push_str(", ");
                            }
                            let _ = write!(out, "{fname}: ");
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
                        let _ = write!(out, "{}..{}{}", lo, if *inclusive { "=" } else { "" }, hi);
                    }
                    Obj::Closure { proto, .. } => {
                        let name = &self.program.protos[*proto as usize].name;
                        if name == "<lambda>" {
                            out.push_str("<fn>");
                        } else {
                            let _ = write!(out, "<fn {name}>");
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
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let r = x.wrapping_mul(0x2545F4914F6CDD1D);
        (r >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn rng_seed(&mut self, seed: i64) {
        self.rng = (seed as u64) | 1;
    }
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
