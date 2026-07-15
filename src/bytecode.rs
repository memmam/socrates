//! Bytecode: instructions, function prototypes, and the compiled program.
//!
//! The VM is a stack machine. Design notes:
//! - Values pushed by expressions; statements leave the stack balanced.
//! - `Set*` instructions POP the value they store (assignments are statements).
//! - Jumps are relative to the *next* instruction (ip has already advanced).
//! - Match compilation uses `Dup`/`TestVariant`/field-extraction ops with
//!   depth-tracked failure stubs; see `compiler.rs`.

use crate::builtins::Native;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Push `consts[i]`.
    Const(u32),
    Unit,
    True,
    False,
    Pop,
    /// Pop `n` values.
    PopN(u16),
    /// Duplicate the top value.
    Dup,
    /// Duplicate the top two values (`a b` → `a b a b`).
    Dup2,
    /// Remove `n` values *under* the top value (block-expression epilogue),
    /// closing any upvalues that pointed into the removed range.
    EndBlock(u16),
    /// Pop `n` values, closing any upvalues that pointed at them (scope exit).
    PopScope(u16),

    GetLocal(u16),
    SetLocal(u16),
    GetGlobal(u16),
    SetGlobal(u16),
    GetUpvalue(u16),
    SetUpvalue(u16),

    /// Push the pre-allocated closure for top-level function `i`.
    PushFn(u32),
    /// Push a native function as a first-class value.
    PushNative(Native),
    /// Create a closure over `protos[i]`, capturing per its upvalue descriptors.
    Closure(u32),

    Jump(i32),
    /// Pop the Bool on top; jump if it was false.
    JumpIfFalse(i32),
    /// Peek the Bool on top; jump if false (leaves the value) — for `&&`.
    JumpIfFalsePeek(i32),
    /// Peek the Bool on top; jump if true (leaves the value) — for `||`.
    JumpIfTruePeek(i32),

    /// Call a callee with `n` args: stack is `[..., callee, a1..an]`.
    Call(u8),
    /// Call top-level function `i` directly (no closure allocation).
    CallFn(u32, u8),
    /// Call a native: args (including the receiver, for methods) are the top
    /// `n` values.
    CallNative(Native, u8),
    /// Tail-call forms of `Call`/`CallFn`: close this frame's upvalues, slide
    /// the callee+args over the current frame, and reuse it instead of
    /// pushing a new one (so tail recursion runs in constant frame space).
    /// A `TailCall` of a native value calls it and returns its result.
    TailCall(u8),
    TailCallFn(u32, u8),
    Return,

    // Arithmetic / logic (operand types guaranteed compatible by the checker;
    // the VM dispatches on the runtime tag).
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    Not,
    /// Bitwise on Int (v0.7). Shifts panic when the count is outside 0..64.
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    /// Structural equality.
    Eq,
    Lt,
    Le,
    Gt,
    Ge,

    /// Convert the top value to its display string.
    ToString,
    /// Pop `n` strings, push their concatenation (string interpolation).
    Concat(u16),

    MakeList(u16),
    /// Pop `2n` values (k1 v1 k2 v2 ...), push a map.
    MakeMap(u16),
    MakeTuple(u16),
    /// Pop hi, lo; push a range.
    MakeRange { inclusive: bool },
    /// Push an instance of struct `def` with all fields set to Unit.
    MakeStructEmpty(u32),
    /// Pop a value, store it into field `i` of the struct at the (new) top.
    StructSetField(u16),
    /// Pop `arity` values, push variant instance.
    MakeVariant { def: u32, variant: u16, arity: u16 },

    /// Pop struct, push its field `i`.
    GetField(u16),
    /// Pop value then struct, store value into field `i`.
    SetField(u16),
    /// Pop tuple, push component `i`.
    TupleGet(u16),
    /// Pop enum value, push its payload field `i`.
    GetVariantField(u16),
    /// Peek enum value, push Bool: is it variant `i`?
    TestVariant(u16),

    /// Pop index then container; push element (panics OOB / missing key).
    Index,
    /// Pop value, index, container; store.
    IndexSet,

    /// `for` support. ForPrep: pop the iterable, push [iterable, state].
    ForPrep,
    /// Advance: push next element, or jump (popping nothing) when done.
    ForNext(i32),
    /// `for` over an Int range *literal*, allocation-free: the compiler
    /// pushes the two bounds directly as the loop slots `[cur, hi]` (no
    /// heap Range, no ForPrep). Advance: push `cur` and step it, or jump
    /// when done; `cur` becomes Unit past `i64::MAX`, exactly like the
    /// heap Range's iteration state.
    ForNextRange { off: i32, inclusive: bool },

    /// A `match` fell through all arms (unreachable if the checker passed).
    MatchFail,
}

/// A compile-time constant.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Int(i64),
    Float(f64),
    Str(String),
}

/// How a closure captures one upvalue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpvalDesc {
    /// True: capture the enclosing frame's local at `index`.
    /// False: share the enclosing closure's upvalue at `index`.
    pub from_local: bool,
    pub index: u16,
}

#[derive(Debug, Clone)]
pub struct FnProto {
    pub name: String,
    pub arity: u8,
    pub code: Vec<Op>,
    /// Source span per instruction (for stack traces).
    pub spans: Vec<Span>,
    pub upvals: Vec<UpvalDesc>,
    /// Total local slots ever live (stack headroom hint; informational).
    pub max_locals: u16,
    /// Which source (VM `sources` index) this proto's spans refer to.
    pub source: u32,
}

impl FnProto {
    pub fn new(name: impl Into<String>, arity: u8) -> FnProto {
        FnProto {
            name: name.into(),
            arity,
            code: Vec::new(),
            spans: Vec::new(),
            upvals: Vec::new(),
            max_locals: 0,
            source: 0,
        }
    }
}

/// Runtime metadata for user types (display, field names).
#[derive(Debug, Clone)]
pub enum RtDef {
    Struct { name: String, fields: Vec<String> },
    Enum { name: String, variants: Vec<(String, u16)> },
}

/// A fully compiled program (or, for REPL sessions, the accumulated program:
/// each chunk's tables are supersets of the previous chunk's, so proto and
/// const indices held by live closures stay valid across updates).
#[derive(Debug, Clone)]
pub struct CompiledProgram {
    pub protos: Vec<FnProto>,
    pub consts: Vec<Const>,
    pub defs: Vec<RtDef>,
    pub globals: u32,
    /// Global names (for the disassembler and REPL).
    pub global_names: Vec<String>,
    /// The proto to execute for this chunk.
    pub entry: u32,
}
