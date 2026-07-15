//! Runtime values, heap objects, and the mark-and-sweep garbage collector.
//!
//! Values are small tagged immediates; everything compound lives on the
//! [`Heap`] behind a `Handle` (an index into a slot vector with a free list).
//! The heap never collects on its own: the VM calls `Vm::gc_checkpoint()` at
//! points where every live object is reachable from a root (the value stack,
//! globals, frames, open upvalues, interned constants, and explicit temp
//! roots), then allocates freely until the next checkpoint.

use crate::builtins::Native;

pub type Handle = u32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A native function as a first-class value.
    Native(Native),
    Obj(Handle),
    /// Internal marker for a global read before its `let` ran.
    Undefined,
}

#[derive(Debug, Clone)]
pub enum Upval {
    /// Points at a live stack slot.
    Open(usize),
    /// The variable escaped its scope; the value lives here now.
    Closed(Value),
}

#[derive(Debug, Clone)]
pub enum Obj {
    /// Recycled slot (member of the free list).
    Free,
    Str(String),
    List(Vec<Value>),
    Map(FMap),
    Tuple(Vec<Value>),
    Struct { def: u32, fields: Vec<Value> },
    Variant { def: u32, variant: u32, fields: Vec<Value> },
    Closure { proto: u32, upvals: Vec<Handle> },
    Upvalue(Upval),
    Range { lo: i64, hi: i64, inclusive: bool },
    /// Packed byte buffer (v0.7). A GC leaf: no traced children.
    Bytes(Vec<u8>),
    /// A worker handle (v0.7). A GC leaf: channels and a join handle,
    /// never GC'd values (only `String`s cross the thread boundary).
    Worker(std::rc::Rc<std::cell::RefCell<crate::worker::WorkerHandle>>),
}

/// An insertion-ordered map with structural keys. Entries keep their insertion
/// order for iteration/display; lookups go through a hash index (key hashes are
/// precomputed; equal hashes fall back to deep equality checked by the VM).
#[derive(Debug, Clone, Default)]
pub struct FMap {
    /// (key hash, key, value) in insertion order.
    pub entries: Vec<(u64, Value, Value)>,
    /// hash → entry indices.
    index: std::collections::HashMap<u64, Vec<u32>>,
}

impl FMap {
    pub fn new() -> FMap {
        FMap::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Candidate entry indices for a key hash.
    pub fn candidates(&self, hash: u64) -> &[u32] {
        self.index.get(&hash).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn push(&mut self, hash: u64, key: Value, value: Value) {
        let idx = self.entries.len() as u32;
        self.entries.push((hash, key, value));
        self.index.entry(hash).or_default().push(idx);
    }

    pub fn set_at(&mut self, idx: u32, value: Value) -> Value {
        std::mem::replace(&mut self.entries[idx as usize].2, value)
    }

    pub fn remove_at(&mut self, idx: u32) -> (u64, Value, Value) {
        let e = self.entries.remove(idx as usize);
        // Indices shifted; rebuild the index (removal is the rare operation).
        self.index.clear();
        for (i, (h, _, _)) in self.entries.iter().enumerate() {
            self.index.entry(*h).or_default().push(i as u32);
        }
        e
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }
}

pub struct Heap {
    /// Objects, indexed by `Handle`. Mark bits live in the parallel `marks`
    /// vector so the mark phase can read an object's children while flagging
    /// other slots (disjoint field borrows) — no copying, no per-object
    /// allocation inside the trace loop.
    objs: Vec<Obj>,
    marks: Vec<bool>,
    free: Vec<Handle>,
    /// Reusable mark-phase work list (kept across collections).
    work: Vec<Handle>,
    live: usize,
    next_gc: usize,
    pub stress: bool,
    pub log: bool,
    pub collections: u64,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    pub fn new() -> Heap {
        let stress = std::env::var("FABLE_GC_STRESS").map(|v| v == "1").unwrap_or(false);
        let log = std::env::var("FABLE_GC_LOG").map(|v| v == "1").unwrap_or(false);
        Heap {
            objs: Vec::new(),
            marks: Vec::new(),
            free: Vec::new(),
            work: Vec::new(),
            live: 0,
            next_gc: 4096,
            stress,
            log,
            collections: 0,
        }
    }

    /// Allocate without collecting (the VM checkpoints separately).
    pub fn alloc(&mut self, obj: Obj) -> Handle {
        self.live += 1;
        if let Some(h) = self.free.pop() {
            self.objs[h as usize] = obj;
            h
        } else {
            self.objs.push(obj);
            self.marks.push(false);
            (self.objs.len() - 1) as Handle
        }
    }

    pub fn get(&self, h: Handle) -> &Obj {
        &self.objs[h as usize]
    }

    pub fn get_mut(&mut self, h: Handle) -> &mut Obj {
        &mut self.objs[h as usize]
    }

    pub fn wants_gc(&self) -> bool {
        self.stress || self.live >= self.next_gc
    }

    /// Mark phase entry: flag a root value for tracing.
    pub fn mark_value(&mut self, v: Value) {
        if let Value::Obj(h) = v {
            self.mark_handle(h);
        }
    }

    pub fn mark_handle(&mut self, h: Handle) {
        if !self.marks[h as usize] {
            self.marks[h as usize] = true;
            self.work.push(h);
        }
    }

    /// Drain the work list, tracing children. Mark bits live apart from the
    /// objects, so children are flagged in place while the parent is borrowed
    /// — no allocation inside the loop.
    pub fn trace(&mut self) {
        let Heap { objs, marks, work, .. } = self;
        #[inline]
        fn mark(marks: &mut [bool], work: &mut Vec<Handle>, h: Handle) {
            if !marks[h as usize] {
                marks[h as usize] = true;
                work.push(h);
            }
        }
        #[inline]
        fn mark_v(marks: &mut [bool], work: &mut Vec<Handle>, v: Value) {
            if let Value::Obj(h) = v {
                mark(marks, work, h);
            }
        }
        while let Some(h) = work.pop() {
            match &objs[h as usize] {
                Obj::Free | Obj::Str(_) | Obj::Range { .. } | Obj::Bytes(_)
                | Obj::Worker(_) => {}
                Obj::List(items) | Obj::Tuple(items) => {
                    for &v in items {
                        mark_v(marks, work, v);
                    }
                }
                Obj::Map(m) => {
                    for &(_, k, v) in &m.entries {
                        mark_v(marks, work, k);
                        mark_v(marks, work, v);
                    }
                }
                Obj::Struct { fields, .. } | Obj::Variant { fields, .. } => {
                    for &v in fields {
                        mark_v(marks, work, v);
                    }
                }
                Obj::Closure { upvals, .. } => {
                    for &c in upvals {
                        mark(marks, work, c);
                    }
                }
                Obj::Upvalue(Upval::Open(_)) => {}
                Obj::Upvalue(Upval::Closed(v)) => mark_v(marks, work, *v),
            }
        }
    }

    /// Sweep phase: free unmarked slots, clear marks, retune the threshold.
    pub fn sweep(&mut self) {
        let before = self.live;
        for (i, m) in self.marks.iter_mut().enumerate() {
            if *m {
                *m = false;
            } else if !matches!(self.objs[i], Obj::Free) {
                self.objs[i] = Obj::Free;
                self.free.push(i as Handle);
                self.live -= 1;
            }
        }
        self.collections += 1;
        // A low floor makes small working sets collect every few hundred
        // allocations, and every sweep walks the whole slot table — so keep
        // a healthy minimum headroom (a few hundred KB at worst).
        self.next_gc = (self.live * 2).max(4096);
        if self.log {
            eprintln!(
                "[gc] collected {} of {} objects ({} live, next at {})",
                before - self.live,
                before,
                self.live,
                self.next_gc
            );
        }
    }

}
