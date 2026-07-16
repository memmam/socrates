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
#[repr(u8)] // explicit tag: a plain byte load beats a niche-encoded discriminant here
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
    /// A window handle (v0.8, Linux-only for now). A GC leaf: OS/GL handles
    /// only, never GC'd values.
    Window(std::rc::Rc<std::cell::RefCell<crate::window::WindowHandle>>),
}

/// An insertion-ordered map with structural keys. Entries keep their insertion
/// order for iteration/display; lookups go through a hash index (key hashes are
/// precomputed; equal hashes fall back to deep equality checked by the VM).
#[derive(Debug, Clone, Default)]
pub struct FMap {
    /// (key hash, key, value) in insertion order.
    pub entries: Vec<(u64, Value, Value)>,
    /// hash → entry indices.
    index: std::collections::HashMap<u64, Bucket, BuildMixHasher>,
}

/// The index keys are already FNV-mixed structural hashes; instead of
/// SipHash-ing them again, finish with one splitmix64-style round (FNV's
/// low bits alone avalanche poorly, and hashbrown derives both the bucket
/// index and its control byte from the hash).
#[derive(Debug, Clone, Copy, Default)]
struct BuildMixHasher;

#[derive(Default)]
struct MixHasher(u64);

impl std::hash::BuildHasher for BuildMixHasher {
    type Hasher = MixHasher;
    fn build_hasher(&self) -> MixHasher {
        MixHasher(0)
    }
}

impl std::hash::Hasher for MixHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write_u64(&mut self, n: u64) {
        let mut x = n;
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58476d1ce4e5b9);
        x ^= x >> 31;
        self.0 = x;
    }

    fn write(&mut self, bytes: &[u8]) {
        // Not reached for u64 keys; kept correct for completeness.
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
}

/// Entry indices sharing one key hash. True 64-bit collisions are rare, so
/// the single-index case is stored inline (no per-key heap allocation).
#[derive(Debug, Clone)]
enum Bucket {
    One(u32),
    Many(Vec<u32>),
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
        match self.index.get(&hash) {
            None => &[],
            Some(Bucket::One(i)) => std::slice::from_ref(i),
            Some(Bucket::Many(v)) => v,
        }
    }

    pub fn push(&mut self, hash: u64, key: Value, value: Value) {
        let idx = self.entries.len() as u32;
        self.entries.push((hash, key, value));
        match self.index.entry(hash) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(Bucket::One(idx));
            }
            std::collections::hash_map::Entry::Occupied(mut e) => match e.get_mut() {
                Bucket::One(first) => {
                    let first = *first;
                    e.insert(Bucket::Many(vec![first, idx]));
                }
                Bucket::Many(v) => v.push(idx),
            },
        }
    }

    pub fn set_at(&mut self, idx: u32, value: Value) -> Value {
        std::mem::replace(&mut self.entries[idx as usize].2, value)
    }

    pub fn remove_at(&mut self, idx: u32) -> (u64, Value, Value) {
        let e = self.entries.remove(idx as usize);
        // Drop the removed entry from its bucket, then shift the indices
        // that sat above it down by one (entries.remove shifted them).
        match self.index.get_mut(&e.0) {
            Some(Bucket::One(_)) => {
                self.index.remove(&e.0);
            }
            Some(Bucket::Many(v)) => {
                v.retain(|&i| i != idx);
                if let [only] = v.as_slice() {
                    let only = *only;
                    self.index.insert(e.0, Bucket::One(only));
                }
            }
            None => {}
        }
        for bucket in self.index.values_mut() {
            match bucket {
                Bucket::One(i) => {
                    if *i > idx {
                        *i -= 1;
                    }
                }
                Bucket::Many(v) => {
                    for i in v {
                        if *i > idx {
                            *i -= 1;
                        }
                    }
                }
            }
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
                | Obj::Worker(_) | Obj::Window(_) => {}
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

