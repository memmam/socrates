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

struct Slot {
    marked: bool,
    obj: Obj,
}

pub struct Heap {
    slots: Vec<Slot>,
    free: Vec<Handle>,
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
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
            next_gc: 256,
            stress,
            log,
            collections: 0,
        }
    }

    /// Allocate without collecting (the VM checkpoints separately).
    pub fn alloc(&mut self, obj: Obj) -> Handle {
        self.live += 1;
        if let Some(h) = self.free.pop() {
            self.slots[h as usize] = Slot { marked: false, obj };
            h
        } else {
            self.slots.push(Slot { marked: false, obj });
            (self.slots.len() - 1) as Handle
        }
    }

    pub fn get(&self, h: Handle) -> &Obj {
        &self.slots[h as usize].obj
    }

    pub fn get_mut(&mut self, h: Handle) -> &mut Obj {
        &mut self.slots[h as usize].obj
    }

    pub fn wants_gc(&self) -> bool {
        self.stress || self.live >= self.next_gc
    }

    /// Mark phase entry: mark a root value and everything reachable from it.
    pub fn mark_value(&mut self, v: Value, work: &mut Vec<Handle>) {
        if let Value::Obj(h) = v {
            self.mark_handle(h, work);
        }
    }

    pub fn mark_handle(&mut self, h: Handle, work: &mut Vec<Handle>) {
        let slot = &mut self.slots[h as usize];
        if !slot.marked {
            slot.marked = true;
            work.push(h);
        }
    }

    /// Drain the work list, tracing children.
    pub fn trace(&mut self, work: &mut Vec<Handle>) {
        while let Some(h) = work.pop() {
            // Take the object's child handles without holding a borrow.
            let mut children: Vec<Handle> = Vec::new();
            let mut child_values: Vec<Value> = Vec::new();
            match &self.slots[h as usize].obj {
                Obj::Free | Obj::Str(_) | Obj::Range { .. } => {}
                Obj::List(items) | Obj::Tuple(items) => child_values.extend(items.iter().copied()),
                Obj::Map(m) => {
                    for (_, k, v) in &m.entries {
                        child_values.push(*k);
                        child_values.push(*v);
                    }
                }
                Obj::Struct { fields, .. } | Obj::Variant { fields, .. } => {
                    child_values.extend(fields.iter().copied())
                }
                Obj::Closure { upvals, .. } => children.extend(upvals.iter().copied()),
                Obj::Upvalue(Upval::Open(_)) => {}
                Obj::Upvalue(Upval::Closed(v)) => child_values.push(*v),
            }
            for v in child_values {
                self.mark_value(v, work);
            }
            for c in children {
                self.mark_handle(c, work);
            }
        }
    }

    /// Sweep phase: free unmarked slots, clear marks, retune the threshold.
    pub fn sweep(&mut self) {
        let before = self.live;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.marked {
                slot.marked = false;
            } else if !matches!(slot.obj, Obj::Free) {
                slot.obj = Obj::Free;
                self.free.push(i as Handle);
                self.live -= 1;
            }
        }
        self.collections += 1;
        self.next_gc = (self.live * 2).max(256);
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
