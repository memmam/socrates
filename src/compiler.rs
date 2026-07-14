//! Bytecode compiler: checked AST → `CompiledProgram`.
//!
//! The compiler tracks a *virtual stack depth* per function so locals declared
//! mid-expression (match bindings, anonymous scrutinee slots, `for` iterator
//! state) always land on correct slots. Every emitted instruction adjusts the
//! depth by its static stack effect; statement boundaries assert balance.
//!
//! Pattern matching compiles to depth-tracked test sequences: each arm
//! pre-pushes its binding slots (as Unit), tests against a copy of the
//! scrutinee (extraction via `Dup` + field ops, bindings stored with
//! `SetLocal`), and jumps to per-depth failure stubs that unwind temporaries
//! before falling through to the next arm.

use std::collections::HashMap;

use crate::ast::*;
use crate::bytecode::{CompiledProgram, Const, FnProto, Op, RtDef, UpvalDesc};
use crate::check::{Checker, Res};
use crate::span::{NodeId, Span};
use crate::types::TypeDef;

const ANON: u32 = u32::MAX;

/// Persistent compiler state. One-shot compilation uses it once; the REPL
/// keeps it alive so each chunk appends to the same proto/const tables (never
/// invalidating indices captured by live closures).
#[derive(Default)]
pub struct ProgramBuilder {
    consts: Vec<Const>,
    const_map: HashMap<ConstKey, u32>,
    protos: Vec<FnProto>,
    /// checker fn index → proto index.
    fn_proto_map: Vec<u32>,
}

impl ProgramBuilder {
    pub fn new() -> ProgramBuilder {
        ProgramBuilder::default()
    }

    /// Compile one checked chunk; returns the accumulated program with this
    /// chunk's entry proto.
    pub fn compile_chunk(
        &mut self,
        program: &Program,
        checker: &Checker,
        source_idx: u32,
    ) -> CompiledProgram {
        let mut c = Compiler {
            res: &checker.res,
            consts: std::mem::take(&mut self.consts),
            const_map: std::mem::take(&mut self.const_map),
            protos: std::mem::take(&mut self.protos),
            fn_proto_map: std::mem::take(&mut self.fn_proto_map),
            ctxs: Vec::new(),
            source_idx,
        };

        // Reserve proto slots for functions newly declared in this chunk.
        for f in checker.fns.iter().skip(c.fn_proto_map.len()) {
            let idx = c.protos.len() as u32;
            let mut proto = FnProto::new(f.name.clone(), f.params.len() as u8);
            proto.source = source_idx;
            c.protos.push(proto);
            c.fn_proto_map.push(idx);
        }

        // Compile declared function and method bodies.
        for stmt in &program.stmts {
            match &stmt.kind {
                StmtKind::Fn(f) => c.compile_fn_decl(f),
                StmtKind::Impl(im) => {
                    for m in &im.methods {
                        c.compile_fn_decl(m);
                    }
                }
                _ => {}
            }
        }

        // Compile this chunk's top-level script.
        let entry = c.protos.len() as u32;
        let mut script = FnProto::new("<script>", 0);
        script.source = source_idx;
        c.protos.push(script);
        let _ = entry;
        c.ctxs.push(FnCtx::new());
        for stmt in &program.stmts {
            c.stmt(stmt);
        }
        let end = program.stmts.last().map(|s| s.span).unwrap_or(Span::point(0));
        c.emit(Op::Unit, end);
        c.emit(Op::Return, end);
        let ctx = c.ctxs.pop().unwrap();
        let mut proto = ctx.proto;
        proto.max_locals = ctx.max_depth;
        proto.source = source_idx;
        proto.name = "<script>".into();
        c.protos[entry as usize] = proto;

        let defs = checker
            .defs
            .types
            .iter()
            .map(|d| match d {
                TypeDef::Struct(s) => RtDef::Struct {
                    name: s.name.clone(),
                    fields: s.fields.iter().map(|(n, _)| n.clone()).collect(),
                },
                TypeDef::Enum(e) => RtDef::Enum {
                    name: e.name.clone(),
                    variants: e
                        .variants
                        .iter()
                        .map(|v| (v.name.clone(), v.fields.len() as u16))
                        .collect(),
                },
            })
            .collect();

        self.consts = c.consts;
        self.const_map = c.const_map;
        self.protos = c.protos;
        self.fn_proto_map = c.fn_proto_map;

        CompiledProgram {
            protos: self.protos.clone(),
            consts: self.consts.clone(),
            defs,
            globals: checker.globals.len() as u32,
            global_names: checker.globals.iter().map(|g| g.name.clone()).collect(),
            entry,
        }
    }
}

/// One-shot compilation of a whole program.
pub fn compile(program: &Program, checker: &Checker) -> CompiledProgram {
    ProgramBuilder::new().compile_chunk(program, checker, 0)
}

#[derive(Hash, PartialEq, Eq)]
enum ConstKey {
    I(i64),
    F(u64),
    S(String),
}

struct LocalSlot {
    local_id: u32,
    /// Absolute stack slot (relative to the frame base). NOT the index into
    /// `locals`: expression temporaries can sit between locals on the stack
    /// (e.g. `1 + match x { .. }` declares the scrutinee slot above the `1`).
    slot: u16,
    captured: bool,
}

struct LoopCtx {
    /// Jump target for `continue`.
    continue_to: usize,
    break_jumps: Vec<usize>,
    /// Virtual stack depth at loop entry / body start: break/continue emit a
    /// PopScope down to these, which also unwinds expression TEMPORARIES a
    /// break inside a block-expression operand would otherwise leak.
    depth_at_entry: u16,
    depth_at_body: u16,
}

struct FnCtx {
    proto: FnProto,
    locals: Vec<LocalSlot>,
    scope_marks: Vec<usize>,
    loops: Vec<LoopCtx>,
    depth: u16,
    max_depth: u16,
}

impl FnCtx {
    fn new() -> FnCtx {
        FnCtx {
            proto: FnProto::new("", 0),
            locals: Vec::new(),
            scope_marks: Vec::new(),
            loops: Vec::new(),
            depth: 0,
            max_depth: 0,
        }
    }
}

struct Compiler<'a> {
    res: &'a HashMap<NodeId, Res>,
    consts: Vec<Const>,
    const_map: HashMap<ConstKey, u32>,
    protos: Vec<FnProto>,
    fn_proto_map: Vec<u32>,
    ctxs: Vec<FnCtx>,
    source_idx: u32,
}

impl<'a> Compiler<'a> {
    // ------------------------------------------------------------------
    // Emission primitives
    // ------------------------------------------------------------------

    fn ctx(&mut self) -> &mut FnCtx {
        self.ctxs.last_mut().unwrap()
    }

    fn emit(&mut self, op: Op, span: Span) -> usize {
        let effect = stack_effect(&op);
        let ctx = self.ctx();
        let idx = ctx.proto.code.len();
        ctx.proto.code.push(op);
        ctx.proto.spans.push(span);
        ctx.depth = (ctx.depth as i32 + effect) as u16;
        ctx.max_depth = ctx.max_depth.max(ctx.depth);
        idx
    }

    /// Emit a jump with a placeholder offset; patch later.
    fn emit_jump(&mut self, op: Op, span: Span) -> usize {
        self.emit(op, span)
    }

    fn here(&mut self) -> usize {
        self.ctx().proto.code.len()
    }

    fn patch_jump(&mut self, at: usize) {
        let target = self.ctx().proto.code.len() as i32;
        let offset = target - (at as i32 + 1);
        let ctx = self.ctx();
        match &mut ctx.proto.code[at] {
            Op::Jump(o)
            | Op::JumpIfFalse(o)
            | Op::JumpIfFalsePeek(o)
            | Op::JumpIfTruePeek(o)
            | Op::ForNext(o) => *o = offset,
            other => unreachable!("patch_jump on non-jump {other:?}"),
        }
    }

    fn emit_loop(&mut self, to: usize, span: Span) {
        let at = self.ctx().proto.code.len() as i32;
        let offset = to as i32 - (at + 1);
        self.emit(Op::Jump(offset), span);
    }

    fn set_depth(&mut self, d: u16) {
        let ctx = self.ctx();
        ctx.depth = d;
        ctx.max_depth = ctx.max_depth.max(d);
    }

    fn depth(&mut self) -> u16 {
        self.ctx().depth
    }

    fn konst(&mut self, c: Const) -> u32 {
        let key = match &c {
            Const::Int(i) => ConstKey::I(*i),
            Const::Float(f) => ConstKey::F(f.to_bits()),
            Const::Str(s) => ConstKey::S(s.clone()),
        };
        if let Some(&i) = self.const_map.get(&key) {
            return i;
        }
        let i = self.consts.len() as u32;
        self.consts.push(c);
        self.const_map.insert(key, i);
        i
    }

    fn emit_const(&mut self, c: Const, span: Span) {
        let i = self.konst(c);
        self.emit(Op::Const(i), span);
    }

    // ------------------------------------------------------------------
    // Locals / scopes / upvalues
    // ------------------------------------------------------------------

    /// Register the value just pushed (stack top) as a local.
    fn declare_local(&mut self, local_id: u32) -> u16 {
        let ctx = self.ctx();
        debug_assert!(ctx.depth > 0, "declare without value");
        let slot = ctx.depth - 1;
        ctx.locals.push(LocalSlot { local_id, slot, captured: false });
        slot
    }

    fn begin_scope(&mut self) {
        let ctx = self.ctx();
        let mark = ctx.locals.len();
        ctx.scope_marks.push(mark);
    }

    /// Close a statement-position scope: pop its locals.
    fn end_scope_stmt(&mut self, span: Span) {
        let mark = self.ctx().scope_marks.pop().unwrap();
        let n = self.ctx().locals.len() - mark;
        if n > 0 {
            self.emit(Op::PopScope(n as u16), span);
            self.ctx().locals.truncate(mark);
        }
    }

    /// Close an expression-position scope: the block's value is on top; remove
    /// the locals beneath it.
    fn end_scope_expr(&mut self, span: Span) {
        let mark = self.ctx().scope_marks.pop().unwrap();
        let n = self.ctx().locals.len() - mark;
        if n > 0 {
            self.emit(Op::EndBlock(n as u16), span);
            self.ctx().locals.truncate(mark);
        }
    }

    fn find_local(&self, ctx_i: usize, local_id: u32) -> Option<u16> {
        self.ctxs[ctx_i]
            .locals
            .iter()
            .rev()
            .find(|l| l.local_id == local_id && local_id != ANON)
            .map(|l| l.slot)
    }

    fn add_upvalue(&mut self, ctx_i: usize, desc: UpvalDesc) -> u16 {
        let proto = &mut self.ctxs[ctx_i].proto;
        if let Some(i) = proto.upvals.iter().position(|u| *u == desc) {
            return i as u16;
        }
        proto.upvals.push(desc);
        (proto.upvals.len() - 1) as u16
    }

    fn resolve_upvalue(&mut self, ctx_i: usize, local_id: u32) -> Option<u16> {
        if ctx_i == 0 {
            return None;
        }
        let parent = ctx_i - 1;
        if let Some(slot) = self.find_local(parent, local_id) {
            if let Some(l) = self.ctxs[parent]
                .locals
                .iter_mut()
                .rev()
                .find(|l| l.local_id == local_id)
            {
                l.captured = true;
            }
            return Some(self.add_upvalue(ctx_i, UpvalDesc { from_local: true, index: slot }));
        }
        if let Some(up) = self.resolve_upvalue(parent, local_id) {
            return Some(self.add_upvalue(ctx_i, UpvalDesc { from_local: false, index: up }));
        }
        None
    }

    /// Emit a read of a checker local (as local or upvalue).
    fn emit_get_local(&mut self, local_id: u32, span: Span) {
        let top = self.ctxs.len() - 1;
        if let Some(slot) = self.find_local(top, local_id) {
            self.emit(Op::GetLocal(slot), span);
        } else if let Some(up) = self.resolve_upvalue(top, local_id) {
            self.emit(Op::GetUpvalue(up), span);
        } else {
            unreachable!("unresolved local {local_id}");
        }
    }

    fn emit_set_local(&mut self, local_id: u32, span: Span) {
        let top = self.ctxs.len() - 1;
        if let Some(slot) = self.find_local(top, local_id) {
            self.emit(Op::SetLocal(slot), span);
        } else if let Some(up) = self.resolve_upvalue(top, local_id) {
            self.emit(Op::SetUpvalue(up), span);
        } else {
            unreachable!("unresolved local {local_id}");
        }
    }

    // ------------------------------------------------------------------
    // Functions
    // ------------------------------------------------------------------

    fn compile_fn_decl(&mut self, f: &FnDecl) {
        let Some(Res::Fn(idx)) = self.res.get(&f.id) else { return };
        let proto_idx = self.fn_proto_map[*idx as usize];
        let mut ctx = FnCtx::new();
        ctx.proto = FnProto::new(f.name.name.clone(), f.params.len() as u8);
        ctx.proto.source = self.source_idx;
        ctx.depth = f.params.len() as u16;
        ctx.max_depth = ctx.depth;
        self.ctxs.push(ctx);
        for (i, p) in f.params.iter().enumerate() {
            let id = match self.res.get(&p.id) {
                Some(Res::Local(id)) => *id,
                _ => ANON,
            };
            self.ctx().locals.push(LocalSlot { local_id: id, slot: i as u16, captured: false });
        }
        self.block_expr_tail(&f.body);
        self.emit(Op::Return, f.body.span);
        let ctx = self.ctxs.pop().unwrap();
        let mut proto = ctx.proto;
        proto.max_locals = ctx.max_depth;
        self.protos[proto_idx as usize] = proto;
    }

    fn compile_lambda(&mut self, e: &Expr, params: &[LambdaParam], body: &Expr) {
        let proto_idx = self.protos.len() as u32;
        self.protos.push(FnProto::new("<lambda>", params.len() as u8));

        let mut ctx = FnCtx::new();
        ctx.proto = FnProto::new("<lambda>", params.len() as u8);
        ctx.proto.source = self.source_idx;
        ctx.depth = params.len() as u16;
        ctx.max_depth = ctx.depth;
        self.ctxs.push(ctx);
        for (i, p) in params.iter().enumerate() {
            let id = match self.res.get(&p.id) {
                Some(Res::Local(id)) => *id,
                _ => ANON,
            };
            self.ctx().locals.push(LocalSlot { local_id: id, slot: i as u16, captured: false });
        }
        self.expr_tail(body);
        self.emit(Op::Return, body.span);
        let ctx = self.ctxs.pop().unwrap();
        let mut proto = ctx.proto;
        proto.max_locals = ctx.max_depth;
        self.protos[proto_idx as usize] = proto;

        self.emit(Op::Closure(proto_idx), e.span);
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Fn(_)
            | StmtKind::Struct(_)
            | StmtKind::Enum(_)
            | StmtKind::Impl(_)
            | StmtKind::Import { .. } => {}
            StmtKind::Let { pattern, init, .. } => self.let_stmt(pattern, init, stmt.span),
            StmtKind::Assign { target, op, value } => self.assign(target, *op, value, stmt.span),
            StmtKind::Expr { expr, tail } => {
                self.expr(expr);
                if !tail {
                    self.emit(Op::Pop, stmt.span);
                }
            }
            StmtKind::While { cond, body } => {
                let d = self.depth();
                let start = self.here();
                self.ctx().loops.push(LoopCtx {
                    continue_to: start,
                    break_jumps: Vec::new(),
                    depth_at_entry: d,
                    depth_at_body: d,
                });
                self.expr(cond);
                let exit = self.emit_jump(Op::JumpIfFalse(0), cond.span);
                self.block_stmt(body);
                self.emit_loop(start, stmt.span);
                self.patch_jump(exit);
                let lp = self.ctx().loops.pop().unwrap();
                for j in lp.break_jumps {
                    self.patch_jump(j);
                }
            }
            StmtKind::For { pattern, iter, body } => {
                let locals_at_entry = self.ctx().locals.len();
                let depth_at_entry = self.depth();
                self.expr(iter);
                self.emit(Op::ForPrep, iter.span);
                // The iterator + its state occupy two anonymous local slots.
                let d = self.depth();
                self.ctx().locals.push(LocalSlot { local_id: ANON, slot: d - 2, captured: false });
                self.ctx().locals.push(LocalSlot { local_id: ANON, slot: d - 1, captured: false });

                let depth_at_body = self.depth(); // [.., iter, state]
                let next = self.here();
                let done = self.emit(Op::ForNext(0), pattern.span);
                // ForNext pushed the element on the fall-through path.
                let d = self.depth();
                self.set_depth(d + 1);
                self.ctx().loops.push(LoopCtx {
                    continue_to: next,
                    break_jumps: Vec::new(),
                    depth_at_entry,
                    depth_at_body,
                });

                self.begin_scope();
                self.bind_loop_pattern(pattern);
                for s in &body.stmts {
                    self.stmt_discard(s);
                }
                self.end_scope_stmt(body.span);
                self.emit_loop(next, stmt.span);

                self.patch_jump(done);
                // The done-jump was taken before the element was pushed, so the
                // depth here equals the depth after the (balanced) body — which
                // is what `self.depth()` already reads; no adjustment.
                self.emit(Op::PopScope(2), stmt.span);
                self.ctx().locals.truncate(locals_at_entry);
                let lp = self.ctx().loops.pop().unwrap();
                for j in lp.break_jumps {
                    self.patch_jump(j);
                }
            }
            StmtKind::Return(value) => {
                match value {
                    Some(v) => self.expr_tail(v),
                    None => {
                        self.emit(Op::Unit, stmt.span);
                    }
                }
                self.emit(Op::Return, stmt.span);
                // (Op::Return's stack effect already accounts for the
                // consumed value; no extra depth adjustment.)
            }
            StmtKind::Break => {
                let Some(lp) = self.ctx().loops.last() else { return };
                let target_depth = lp.depth_at_entry;
                // Unwind by DEPTH, not by declared-local count: a break inside
                // a block-expression operand also has expression temporaries
                // on the stack (e.g. the `1` in `1 + { ... break; ... }`).
                let unwind = self.depth() - target_depth;
                if unwind > 0 {
                    self.emit(Op::PopScope(unwind), stmt.span);
                    // Compile-time locals stay (later code in this block may
                    // still reference them on other paths); only depth moves.
                    let d = self.depth();
                    self.set_depth(d + unwind); // PopScope adjusted; restore
                }
                let j = self.emit_jump(Op::Jump(0), stmt.span);
                self.ctx().loops.last_mut().unwrap().break_jumps.push(j);
            }
            StmtKind::Continue => {
                let Some(lp) = self.ctx().loops.last() else { return };
                let target = lp.continue_to;
                let target_depth = lp.depth_at_body;
                let unwind = self.depth() - target_depth;
                if unwind > 0 {
                    self.emit(Op::PopScope(unwind), stmt.span);
                    let d = self.depth();
                    self.set_depth(d + unwind);
                }
                self.emit_loop(target, stmt.span);
            }
        }
    }

    /// Bind a `for`-loop pattern to the element ForNext just pushed. The
    /// element (and any extracted bindings) live in the loop's body scope,
    /// so they unwind on every iteration via the scope machinery.
    fn bind_loop_pattern(&mut self, pattern: &Pattern) {
        if let PatternKind::Binding(_) = &pattern.kind {
            if let Some(Res::Local(id)) = self.res.get(&pattern.id) {
                let id = *id;
                self.declare_local(id);
            } else {
                self.declare_local(ANON);
            }
            return;
        }
        if matches!(pattern.kind, PatternKind::Wildcard) {
            self.declare_local(ANON);
            return;
        }
        // Destructuring: keep the element as an anonymous local and extract
        // each binding by path, exactly like `let` destructuring.
        let mut paths = Vec::new();
        self.collect_bind_paths(pattern, &mut Vec::new(), &mut paths);
        let anon_slot = self.declare_local(ANON);
        for (path, node) in &paths {
            self.emit(Op::GetLocal(anon_slot), pattern.span);
            for step in path {
                self.emit(*step, pattern.span);
            }
            match self.res.get(node) {
                Some(Res::Local(id)) => {
                    let id = *id;
                    self.declare_local(id);
                }
                _ => {
                    // Checker errored (or the binding is a variant misparse);
                    // keep the stack balanced.
                    self.emit(Op::Pop, pattern.span);
                }
            }
        }
    }

    fn let_stmt(&mut self, pattern: &Pattern, init: &Expr, span: Span) {
        // Fast path: single-name binding.
        if let PatternKind::Binding(_) = &pattern.kind {
            match self.res.get(&pattern.id) {
                Some(Res::Local(id)) => {
                    let id = *id;
                    self.expr(init);
                    self.declare_local(id);
                }
                Some(Res::Global(slot)) => {
                    let slot = *slot;
                    self.expr(init);
                    self.emit(Op::SetGlobal(slot as u16), span);
                }
                _ => {
                    // Variant-reinterpreted binding (`let None = ..`) — checker
                    // errored; keep the stack balanced anyway.
                    self.expr(init);
                    self.emit(Op::Pop, span);
                }
            }
            return;
        }
        if matches!(pattern.kind, PatternKind::Wildcard) {
            self.expr(init);
            self.emit(Op::Pop, span);
            return;
        }

        // Destructuring: evaluate once, then extract each binding by path.
        let mut paths = Vec::new();
        self.collect_bind_paths(pattern, &mut Vec::new(), &mut paths);
        self.expr(init);

        // Keep the initializer as an anonymous local while extracting.
        let anon_slot = self.declare_local(ANON);
        for (path, node) in &paths {
            self.emit(Op::GetLocal(anon_slot), span);
            for step in path {
                self.emit(*step, span);
            }
            match self.res.get(node) {
                Some(Res::Local(id)) => {
                    let id = *id;
                    self.declare_local(id);
                }
                Some(Res::Global(slot)) => {
                    let slot = *slot;
                    self.emit(Op::SetGlobal(slot as u16), span);
                }
                _ => {
                    self.emit(Op::Pop, span);
                }
            }
        }
        // The anonymous initializer slot stays until scope end (or, at top
        // level, until the script frame unwinds) — harmless.
    }

    /// Collect (navigation ops, binding node) pairs for an irrefutable pattern.
    fn collect_bind_paths(
        &self,
        pat: &Pattern,
        path: &mut Vec<Op>,
        out: &mut Vec<(Vec<Op>, NodeId)>,
    ) {
        match &pat.kind {
            PatternKind::Binding(_) => {
                if !matches!(self.res.get(&pat.id), Some(Res::Variant { .. })) {
                    out.push((path.clone(), pat.id));
                }
            }
            PatternKind::Tuple(items) => {
                for (i, p) in items.iter().enumerate() {
                    path.push(Op::TupleGet(i as u16));
                    self.collect_bind_paths(p, path, out);
                    path.pop();
                }
            }
            PatternKind::Struct { fields, .. } => {
                let order = match self.res.get(&pat.id) {
                    Some(Res::StructPat { field_order, .. }) => field_order.clone(),
                    _ => (0..fields.len() as u32).collect(),
                };
                for ((_, p), idx) in fields.iter().zip(order) {
                    if idx == u32::MAX {
                        continue;
                    }
                    path.push(Op::GetField(idx as u16));
                    self.collect_bind_paths(p, path, out);
                    path.pop();
                }
            }
            PatternKind::Variant { fields, .. } => {
                for (i, p) in fields.iter().enumerate() {
                    path.push(Op::GetVariantField(i as u16));
                    self.collect_bind_paths(p, path, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    fn assign(&mut self, target: &Expr, op: Option<BinOp>, value: &Expr, span: Span) {
        match &target.kind {
            ExprKind::Var(_) => {
                let res = self.res.get(&target.id).cloned();
                match res {
                    Some(Res::Local(id)) => {
                        if let Some(op) = op {
                            self.emit_get_local(id, target.span);
                            self.expr(value);
                            self.emit(bin_op(op), span);
                        } else {
                            self.expr(value);
                        }
                        self.emit_set_local(id, span);
                    }
                    Some(Res::Global(slot)) => {
                        if let Some(op) = op {
                            self.emit(Op::GetGlobal(slot as u16), target.span);
                            self.expr(value);
                            self.emit(bin_op(op), span);
                        } else {
                            self.expr(value);
                        }
                        self.emit(Op::SetGlobal(slot as u16), span);
                    }
                    _ => {
                        // Checker errored; keep balance.
                        self.expr(value);
                        self.emit(Op::Pop, span);
                    }
                }
            }
            ExprKind::Field { base, .. } => {
                let res = self.res.get(&target.id).cloned();
                match res {
                    Some(Res::Field { index, .. }) => {
                        self.expr(base);
                        if let Some(op) = op {
                            self.emit(Op::Dup, span);
                            self.emit(Op::GetField(index as u16), target.span);
                            self.expr(value);
                            self.emit(bin_op(op), span);
                        } else {
                            self.expr(value);
                        }
                        self.emit(Op::SetField(index as u16), span);
                    }
                    _ => {
                        self.expr(value);
                        self.emit(Op::Pop, span);
                    }
                }
            }
            ExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
                if let Some(op) = op {
                    self.emit(Op::Dup2, span);
                    self.emit(Op::Index, target.span);
                    self.expr(value);
                    self.emit(bin_op(op), span);
                } else {
                    self.expr(value);
                }
                self.emit(Op::IndexSet, span);
            }
            _ => {
                self.expr(value);
                self.emit(Op::Pop, span);
            }
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Int(v) => self.emit_const(Const::Int(*v), e.span),
            ExprKind::Float(v) => self.emit_const(Const::Float(*v), e.span),
            ExprKind::Bool(true) => {
                self.emit(Op::True, e.span);
            }
            ExprKind::Bool(false) => {
                self.emit(Op::False, e.span);
            }
            ExprKind::Str(s) => self.emit_const(Const::Str(s.clone()), e.span),
            ExprKind::Unit => {
                self.emit(Op::Unit, e.span);
            }
            ExprKind::StringInterp { parts, exprs } => {
                let mut n: u16 = 0;
                for (i, part) in parts.iter().enumerate() {
                    if !part.is_empty() {
                        self.emit_const(Const::Str(part.clone()), e.span);
                        n += 1;
                    }
                    if i < exprs.len() {
                        self.expr(&exprs[i]);
                        self.emit(Op::ToString, exprs[i].span);
                        n += 1;
                    }
                }
                match n {
                    0 => self.emit_const(Const::Str(String::new()), e.span),
                    1 => {}
                    _ => {
                        self.emit(Op::Concat(n), e.span);
                    }
                }
            }
            ExprKind::Var(_) => {
                let res = self.res.get(&e.id).cloned();
                match res {
                    Some(Res::Local(id)) => self.emit_get_local(id, e.span),
                    Some(Res::Global(slot)) => {
                        self.emit(Op::GetGlobal(slot as u16), e.span);
                    }
                    Some(Res::Fn(idx)) => {
                        let p = self.fn_proto_map[idx as usize];
                        self.emit(Op::PushFn(p), e.span);
                    }
                    Some(Res::NativeFn(n)) => {
                        self.emit(Op::PushNative(n), e.span);
                    }
                    Some(Res::Variant { def, variant }) => {
                        self.emit(
                            Op::MakeVariant { def, variant: variant as u16, arity: 0 },
                            e.span,
                        );
                    }
                    _ => {
                        // Checker errored (undefined name); keep balance.
                        self.emit(Op::Unit, e.span);
                    }
                }
            }
            ExprKind::Field { base, .. } => {
                let res = self.res.get(&e.id).cloned();
                match res {
                    Some(Res::Field { index, .. }) => {
                        self.expr(base);
                        self.emit(Op::GetField(index as u16), e.span);
                    }
                    Some(Res::TupleIndex(i)) => {
                        self.expr(base);
                        self.emit(Op::TupleGet(i as u16), e.span);
                    }
                    Some(Res::FloatConst(v)) => self.emit_const(Const::Float(v), e.span),
                    Some(Res::NativeFn(n)) => {
                        self.emit(Op::PushNative(n), e.span);
                    }
                    Some(Res::Variant { def, variant }) => {
                        self.emit(
                            Op::MakeVariant { def, variant: variant as u16, arity: 0 },
                            e.span,
                        );
                    }
                    Some(Res::Global(slot)) => {
                        // A module global: `alias.value`.
                        self.emit(Op::GetGlobal(slot as u16), e.span);
                    }
                    Some(Res::Fn(idx)) => {
                        // A module function as a value: `alias.f`.
                        let p = self.fn_proto_map[idx as usize];
                        self.emit(Op::PushFn(p), e.span);
                    }
                    _ => {
                        self.emit(Op::Unit, e.span);
                    }
                }
            }
            ExprKind::MethodCall { recv, args, .. } => {
                let res = self.res.get(&e.id).cloned();
                match res {
                    Some(Res::Method(native)) => {
                        self.expr(recv);
                        for a in args {
                            self.expr(a);
                        }
                        self.emit(Op::CallNative(native, args.len() as u8 + 1), e.span);
                    }
                    Some(Res::Fn(idx)) => {
                        // User-defined method: the receiver is argument 0.
                        self.expr(recv);
                        for a in args {
                            self.expr(a);
                        }
                        let p = self.fn_proto_map[idx as usize];
                        self.emit(Op::CallFn(p, args.len() as u8 + 1), e.span);
                    }
                    Some(Res::ModuleFn(idx)) => {
                        // `alias.f(args)` — the receiver is a namespace.
                        for a in args {
                            self.expr(a);
                        }
                        let p = self.fn_proto_map[idx as usize];
                        self.emit(Op::CallFn(p, args.len() as u8), e.span);
                    }
                    Some(Res::Global(slot)) => {
                        // `alias.value(args)` — a module global holding a
                        // function value.
                        self.emit(Op::GetGlobal(slot as u16), e.span);
                        for a in args {
                            self.expr(a);
                        }
                        self.emit(Op::Call(args.len() as u8), e.span);
                    }
                    Some(Res::NativeFn(native)) => {
                        // `math.sqrt(x)` — the receiver is a namespace.
                        for a in args {
                            self.expr(a);
                        }
                        self.emit(Op::CallNative(native, args.len() as u8), e.span);
                    }
                    Some(Res::Variant { def, variant }) => {
                        for a in args {
                            self.expr(a);
                        }
                        self.emit(
                            Op::MakeVariant {
                                def,
                                variant: variant as u16,
                                arity: args.len() as u16,
                            },
                            e.span,
                        );
                    }
                    _ => {
                        for a in args {
                            self.expr(a);
                            self.emit(Op::Pop, a.span);
                        }
                        self.emit(Op::Unit, e.span);
                    }
                }
            }
            ExprKind::Call { callee, args } => {
                let callee_res = self.res.get(&callee.id).cloned();
                // The Call node itself may carry a Variant resolution
                // (prelude constructors like `Some(x)`).
                if let Some(Res::Variant { def, variant }) = self.res.get(&e.id).cloned() {
                    for a in args {
                        self.expr(a);
                    }
                    self.emit(
                        Op::MakeVariant { def, variant: variant as u16, arity: args.len() as u16 },
                        e.span,
                    );
                    return;
                }
                match callee_res {
                    Some(Res::Fn(idx)) => {
                        for a in args {
                            self.expr(a);
                        }
                        let p = self.fn_proto_map[idx as usize];
                        self.emit(Op::CallFn(p, args.len() as u8), e.span);
                    }
                    Some(Res::NativeFn(native)) => {
                        for a in args {
                            self.expr(a);
                        }
                        self.emit(Op::CallNative(native, args.len() as u8), e.span);
                    }
                    Some(Res::Variant { def, variant }) => {
                        for a in args {
                            self.expr(a);
                        }
                        self.emit(
                            Op::MakeVariant {
                                def,
                                variant: variant as u16,
                                arity: args.len() as u16,
                            },
                            e.span,
                        );
                    }
                    _ => {
                        self.expr(callee);
                        for a in args {
                            self.expr(a);
                        }
                        self.emit(Op::Call(args.len() as u8), e.span);
                    }
                }
            }
            ExprKind::Unary { op, expr } => {
                self.expr(expr);
                match op {
                    UnOp::Neg => {
                        // `-x` on a user type dispatches to its `neg` method.
                        if let Some(Res::Fn(idx)) = self.res.get(&e.id).cloned() {
                            let p = self.fn_proto_map[idx as usize];
                            self.emit(Op::CallFn(p, 1), e.span)
                        } else {
                            self.emit(Op::Neg, e.span)
                        }
                    }
                    UnOp::Not => self.emit(Op::Not, e.span),
                };
            }
            ExprKind::Try(inner) => {
                // `Some`/`Ok` are both variant 0 of their prelude enums, and
                // `None`/`Err` are variant 1 — the failure value on the stack
                // IS the propagated return value, so no re-wrapping is needed.
                self.expr(inner);
                self.emit(Op::TestVariant(0), e.span);
                let fail = self.emit_jump(Op::JumpIfFalse(0), e.span);
                self.emit(Op::GetVariantField(0), e.span);
                let end = self.emit_jump(Op::Jump(0), e.span);
                let depth_after = self.depth();
                self.patch_jump(fail);
                // Fail path: the None/Err value is on top; return it as-is.
                self.emit(Op::Return, e.span);
                self.patch_jump(end);
                // Both paths leave one value; the tracked depth followed the
                // fail path's Return, so restore the success-path depth.
                self.set_depth(depth_after);
            }
            ExprKind::Binary { op, op_span, lhs, rhs } => match op {
                BinOp::And => {
                    self.expr(lhs);
                    let short = self.emit_jump(Op::JumpIfFalsePeek(0), *op_span);
                    self.emit(Op::Pop, *op_span);
                    self.expr(rhs);
                    self.patch_jump(short);
                }
                BinOp::Or => {
                    self.expr(lhs);
                    let short = self.emit_jump(Op::JumpIfTruePeek(0), *op_span);
                    self.emit(Op::Pop, *op_span);
                    self.expr(rhs);
                    self.patch_jump(short);
                }
                BinOp::Ne => {
                    self.expr(lhs);
                    self.expr(rhs);
                    self.emit(Op::Eq, *op_span);
                    self.emit(Op::Not, *op_span);
                }
                other => {
                    // An operator method (`a + b` → `a.add(b)`): the checker
                    // recorded the target fn on the Binary node.
                    if let Some(Res::Fn(idx)) = self.res.get(&e.id).cloned() {
                        self.expr(lhs);
                        self.expr(rhs);
                        let p = self.fn_proto_map[idx as usize];
                        self.emit(Op::CallFn(p, 2), *op_span);
                    } else {
                        self.expr(lhs);
                        self.expr(rhs);
                        self.emit(bin_op(*other), *op_span);
                    }
                }
            },
            ExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
                self.emit(Op::Index, e.span);
            }
            ExprKind::List(items) => {
                for item in items {
                    self.expr(item);
                }
                self.emit(Op::MakeList(items.len() as u16), e.span);
            }
            ExprKind::MapLit(entries) => {
                for (k, v) in entries {
                    self.expr(k);
                    self.expr(v);
                }
                self.emit(Op::MakeMap(entries.len() as u16), e.span);
            }
            ExprKind::Tuple(items) => {
                for item in items {
                    self.expr(item);
                }
                self.emit(Op::MakeTuple(items.len() as u16), e.span);
            }
            ExprKind::Range { lo, hi, inclusive } => {
                self.expr(lo);
                self.expr(hi);
                self.emit(Op::MakeRange { inclusive: *inclusive }, e.span);
            }
            ExprKind::StructLit { fields, .. } => {
                let res = self.res.get(&e.id).cloned();
                let Some(Res::StructLit { def, field_order }) = res else {
                    for (_, v) in fields {
                        self.expr(v);
                        self.emit(Op::Pop, v.span);
                    }
                    self.emit(Op::Unit, e.span);
                    return;
                };
                self.emit(Op::MakeStructEmpty(def), e.span);
                for ((_, value), idx) in fields.iter().zip(field_order) {
                    self.expr(value);
                    if idx == u32::MAX {
                        self.emit(Op::Pop, value.span);
                    } else {
                        self.emit(Op::StructSetField(idx as u16), value.span);
                    }
                }
            }
            ExprKind::Lambda { params, body, .. } => {
                self.compile_lambda(e, params, body);
            }
            ExprKind::If { cond, then, els } => {
                self.expr(cond);
                let jf = self.emit_jump(Op::JumpIfFalse(0), cond.span);
                let d0 = self.depth();
                self.block_expr(then);
                match els {
                    Some(els) => {
                        let jend = self.emit_jump(Op::Jump(0), e.span);
                        self.patch_jump(jf);
                        self.set_depth(d0);
                        self.expr(els);
                        self.patch_jump(jend);
                    }
                    None => {
                        // Then-branch value is Unit (checker enforced); emit an
                        // else producing Unit too.
                        let jend = self.emit_jump(Op::Jump(0), e.span);
                        self.patch_jump(jf);
                        self.set_depth(d0);
                        self.emit(Op::Unit, e.span);
                        self.patch_jump(jend);
                    }
                }
            }
            ExprKind::Block(block) => self.block_expr(block),
            ExprKind::Match { scrutinee, arms } => self.match_expr(e, scrutinee, arms),
        }
    }

    /// Compile an expression in tail position: the emitted value immediately
    /// becomes the enclosing function's return value. Calls become tail
    /// calls; `if`/`match`/block forward tail position into their result
    /// branches. Any op sequence emitted after a rewritten call is dead on
    /// that path (the frame is replaced), but is kept for the other paths
    /// and for depth bookkeeping (tail variants share their originals'
    /// stack effects).
    fn expr_tail(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => {
                self.expr(e);
                self.retarget_last_call();
            }
            ExprKind::If { cond, then, els } => {
                self.expr(cond);
                let jf = self.emit_jump(Op::JumpIfFalse(0), cond.span);
                let d0 = self.depth();
                self.block_expr_tail(then);
                match els {
                    Some(els) => {
                        let jend = self.emit_jump(Op::Jump(0), e.span);
                        self.patch_jump(jf);
                        self.set_depth(d0);
                        self.expr_tail(els);
                        self.patch_jump(jend);
                    }
                    None => {
                        let jend = self.emit_jump(Op::Jump(0), e.span);
                        self.patch_jump(jf);
                        self.set_depth(d0);
                        self.emit(Op::Unit, e.span);
                        self.patch_jump(jend);
                    }
                }
            }
            ExprKind::Block(block) => self.block_expr_tail(block),
            ExprKind::Match { scrutinee, arms } => {
                self.match_expr_impl(e, scrutinee, arms, true)
            }
            _ => self.expr(e),
        }
    }

    /// Rewrite a just-emitted `Call`/`CallFn` into its tail variant. The
    /// replacement is 1:1 in place, so no jump offsets shift. Natives and
    /// variant constructors push no frame and are left as-is.
    fn retarget_last_call(&mut self) {
        match self.ctx().proto.code.last_mut() {
            Some(op @ Op::CallFn(..)) => {
                let Op::CallFn(p, n) = *op else { unreachable!() };
                *op = Op::TailCallFn(p, n);
            }
            Some(op @ Op::Call(..)) => {
                let Op::Call(n) = *op else { unreachable!() };
                *op = Op::TailCall(n);
            }
            _ => {}
        }
    }

    /// `block_expr`, but the final tail expression is in tail position.
    fn block_expr_tail(&mut self, block: &Block) {
        self.begin_scope();
        let n = block.stmts.len();
        let mut has_value = false;
        for (i, stmt) in block.stmts.iter().enumerate() {
            let last = i + 1 == n;
            if last {
                match &stmt.kind {
                    StmtKind::Expr { expr, tail: true } => {
                        self.expr_tail(expr);
                        has_value = true;
                    }
                    _ => {
                        self.stmt(stmt);
                    }
                }
            } else {
                self.stmt(stmt);
            }
        }
        if !has_value {
            self.emit(Op::Unit, block.span);
        }
        self.end_scope_expr(block.span);
    }

    /// Compile a block that yields a value.
    fn block_expr(&mut self, block: &Block) {
        self.begin_scope();
        let n = block.stmts.len();
        let mut has_value = false;
        for (i, stmt) in block.stmts.iter().enumerate() {
            let last = i + 1 == n;
            if last {
                match &stmt.kind {
                    StmtKind::Expr { expr, tail: true } => {
                        self.expr(expr);
                        has_value = true;
                    }
                    _ => {
                        self.stmt(stmt);
                    }
                }
            } else {
                self.stmt(stmt);
            }
        }
        if !has_value {
            self.emit(Op::Unit, block.span);
        }
        self.end_scope_expr(block.span);
    }

    /// Compile a block in statement position (no value). A tail expression's
    /// value (always Unit here, per the checker) is discarded.
    fn block_stmt(&mut self, block: &Block) {
        self.begin_scope();
        for stmt in &block.stmts {
            self.stmt_discard(stmt);
        }
        self.end_scope_stmt(block.span);
    }

    /// Like `stmt`, but pops the value of a tail expression statement.
    fn stmt_discard(&mut self, stmt: &Stmt) {
        if let StmtKind::Expr { expr, tail: true } = &stmt.kind {
            self.expr(expr);
            self.emit(Op::Pop, stmt.span);
        } else {
            self.stmt(stmt);
        }
    }

    // ------------------------------------------------------------------
    // Match compilation
    // ------------------------------------------------------------------

    fn match_expr(&mut self, e: &Expr, scrutinee: &Expr, arms: &[MatchArm]) {
        self.match_expr_impl(e, scrutinee, arms, false)
    }

    fn match_expr_impl(
        &mut self,
        e: &Expr,
        scrutinee: &Expr,
        arms: &[MatchArm],
        tail: bool,
    ) {
        self.expr(scrutinee);
        let scrut_slot = self.declare_local(ANON);
        let base_depth = self.depth();
        let mut end_jumps = Vec::new();
        let mut next_arm_jumps: Vec<usize> = Vec::new();

        for arm in arms {
            for j in next_arm_jumps.drain(..) {
                self.patch_jump(j);
            }
            self.set_depth(base_depth);
            // 1. Binding slots (walk order, deduped for or-alternatives).
            let mut bind_ids = Vec::new();
            self.collect_binding_ids(&arm.pattern, &mut bind_ids);
            let nbinds = bind_ids.len() as u16;
            let locals_before = self.ctx().locals.len();
            for id in &bind_ids {
                self.emit(Op::Unit, arm.span);
                self.declare_local(*id);
            }

            // 2. Test against a copy of the scrutinee.
            let mut fails: Vec<(usize, u16)> = Vec::new();
            self.emit(Op::GetLocal(scrut_slot), arm.pattern.span);
            self.pattern_test(&arm.pattern, 1, &mut fails);

            // 3. Guard.
            if let Some(guard) = &arm.guard {
                self.expr(guard);
                let j = self.emit_jump(Op::JumpIfFalse(0), guard.span);
                fails.push((j, 0));
            }

            // 4. Body.
            if tail {
                self.expr_tail(&arm.body);
            } else {
                self.expr(&arm.body);
            }
            if nbinds > 0 {
                self.emit(Op::EndBlock(nbinds), arm.span);
            }
            self.ctx().locals.truncate(locals_before);
            end_jumps.push(self.emit_jump(Op::Jump(0), arm.span));

            // 5. Failure stubs: unwind temporaries, then binding slots, then
            //    jump to the next arm (patched at the top of the next
            //    iteration, or to MatchFail after the last arm).
            for (j, pops) in fails {
                self.patch_jump(j);
                self.set_depth(base_depth + nbinds + pops);
                if pops > 0 {
                    self.emit(Op::PopN(pops), arm.span);
                }
                if nbinds > 0 {
                    self.emit(Op::PopScope(nbinds), arm.span);
                }
                next_arm_jumps.push(self.emit_jump(Op::Jump(0), arm.span));
            }
        }

        // All arms failed (unreachable when the checker's exhaustiveness pass
        // succeeded, and a backstop for guard-only fall-through).
        for j in next_arm_jumps.drain(..) {
            self.patch_jump(j);
        }
        self.set_depth(base_depth);
        self.emit(Op::MatchFail, e.span);
        // MatchFail never returns; static depth is arbitrary here. Provide the
        // value slot expected at the join point.
        self.emit(Op::Unit, e.span);

        for j in end_jumps {
            self.patch_jump(j);
        }
        self.set_depth(base_depth + 1);
        // Remove the anonymous scrutinee beneath the result.
        self.emit(Op::EndBlock(1), e.span);
        let ctx = self.ctx();
        let keep = ctx.locals.len() - 1;
        ctx.locals.truncate(keep);
    }

    fn collect_binding_ids(&self, pat: &Pattern, out: &mut Vec<u32>) {
        match &pat.kind {
            PatternKind::Binding(_) => {
                if let Some(Res::Local(id)) = self.res.get(&pat.id) {
                    if !out.contains(id) {
                        out.push(*id);
                    }
                }
            }
            PatternKind::Tuple(items) => {
                for p in items {
                    self.collect_binding_ids(p, out);
                }
            }
            PatternKind::Variant { fields, .. } => {
                for p in fields {
                    self.collect_binding_ids(p, out);
                }
            }
            PatternKind::Struct { fields, .. } => {
                for (_, p) in fields {
                    self.collect_binding_ids(p, out);
                }
            }
            PatternKind::Or(alts) => {
                for p in alts {
                    self.collect_binding_ids(p, out);
                }
            }
            _ => {}
        }
    }

    /// Emit a test of the value on top of the stack against `pat`.
    ///
    /// Invariants: on success the tested value has been consumed (net stack
    /// effect −1, bindings written via SetLocal). On failure, a jump is
    /// recorded in `fails` with the number of temporaries (including the
    /// tested value's survivors) left to pop; `temps` counts the temporaries
    /// currently on the stack including the value under test.
    fn pattern_test(&mut self, pat: &Pattern, temps: u16, fails: &mut Vec<(usize, u16)>) {
        let span = pat.span;
        match &pat.kind {
            PatternKind::Wildcard | PatternKind::Unit => {
                self.emit(Op::Pop, span);
            }
            PatternKind::Binding(_) => match self.res.get(&pat.id).cloned() {
                Some(Res::Variant { variant, .. }) => {
                    self.emit(Op::TestVariant(variant as u16), span);
                    let j = self.emit_jump(Op::JumpIfFalse(0), span);
                    fails.push((j, temps));
                    self.emit(Op::Pop, span);
                }
                Some(Res::Local(id)) => {
                    self.emit_set_local(id, span);
                }
                _ => {
                    self.emit(Op::Pop, span);
                }
            },
            PatternKind::Int(v) => {
                self.emit_const(Const::Int(*v), span);
                self.emit(Op::Eq, span);
                let j = self.emit_jump(Op::JumpIfFalse(0), span);
                fails.push((j, temps - 1));
            }
            PatternKind::Float(v) => {
                self.emit_const(Const::Float(*v), span);
                self.emit(Op::Eq, span);
                let j = self.emit_jump(Op::JumpIfFalse(0), span);
                fails.push((j, temps - 1));
            }
            PatternKind::Str(s) => {
                self.emit_const(Const::Str(s.clone()), span);
                self.emit(Op::Eq, span);
                let j = self.emit_jump(Op::JumpIfFalse(0), span);
                fails.push((j, temps - 1));
            }
            PatternKind::Bool(b) => {
                self.emit(if *b { Op::True } else { Op::False }, span);
                self.emit(Op::Eq, span);
                let j = self.emit_jump(Op::JumpIfFalse(0), span);
                fails.push((j, temps - 1));
            }
            PatternKind::Tuple(items) => {
                for (i, p) in items.iter().enumerate() {
                    self.emit(Op::Dup, span);
                    self.emit(Op::TupleGet(i as u16), p.span);
                    self.pattern_test(p, temps + 1, fails);
                }
                self.emit(Op::Pop, span);
            }
            PatternKind::Variant { fields, .. } => {
                let Some(Res::Variant { variant, .. }) = self.res.get(&pat.id).cloned() else {
                    self.emit(Op::Pop, span);
                    return;
                };
                self.emit(Op::TestVariant(variant as u16), span);
                let j = self.emit_jump(Op::JumpIfFalse(0), span);
                fails.push((j, temps));
                for (i, p) in fields.iter().enumerate() {
                    self.emit(Op::Dup, span);
                    self.emit(Op::GetVariantField(i as u16), p.span);
                    self.pattern_test(p, temps + 1, fails);
                }
                self.emit(Op::Pop, span);
            }
            PatternKind::Struct { fields, .. } => {
                let order = match self.res.get(&pat.id) {
                    Some(Res::StructPat { field_order, .. }) => field_order.clone(),
                    _ => (0..fields.len() as u32).collect(),
                };
                for ((_, p), idx) in fields.iter().zip(order) {
                    if idx == u32::MAX {
                        continue;
                    }
                    self.emit(Op::Dup, span);
                    self.emit(Op::GetField(idx as u16), p.span);
                    self.pattern_test(p, temps + 1, fails);
                }
                self.emit(Op::Pop, span);
            }
            PatternKind::Or(alts) => {
                let entry_depth = self.depth(); // value under test on top
                let mut success_jumps = Vec::new();
                for (i, alt) in alts.iter().enumerate() {
                    let last = i + 1 == alts.len();
                    self.set_depth(entry_depth);
                    if last {
                        self.pattern_test(alt, temps, fails);
                        // Success falls through with the value consumed.
                    } else {
                        self.emit(Op::Dup, alt.span);
                        let mut alt_fails: Vec<(usize, u16)> = Vec::new();
                        self.pattern_test(alt, temps + 1, &mut alt_fails);
                        // Matched via the dup: the original is still on top.
                        self.emit(Op::Pop, alt.span);
                        success_jumps.push(self.emit_jump(Op::Jump(0), alt.span));
                        // Failure stubs: unwind back down to the original
                        // value, then jump PAST the other stubs to the next
                        // alternative (stubs must not fall into each other).
                        let mut to_next_alt = Vec::new();
                        for (j, pops) in alt_fails {
                            self.patch_jump(j);
                            self.set_depth(entry_depth - temps + pops);
                            let extra = pops - temps;
                            if extra > 0 {
                                self.emit(Op::PopN(extra), alt.span);
                            }
                            to_next_alt.push(self.emit_jump(Op::Jump(0), alt.span));
                        }
                        for j in to_next_alt {
                            self.patch_jump(j);
                        }
                    }
                }
                for j in success_jumps {
                    self.patch_jump(j);
                }
                self.set_depth(entry_depth - 1);
            }
        }
    }
}

fn bin_op(op: BinOp) -> Op {
    match op {
        BinOp::Add => Op::Add,
        BinOp::Sub => Op::Sub,
        BinOp::Mul => Op::Mul,
        BinOp::Div => Op::Div,
        BinOp::Rem => Op::Rem,
        BinOp::Eq => Op::Eq,
        BinOp::Lt => Op::Lt,
        BinOp::Le => Op::Le,
        BinOp::Gt => Op::Gt,
        BinOp::Ge => Op::Ge,
        BinOp::Ne | BinOp::And | BinOp::Or => unreachable!("lowered separately"),
    }
}

/// Static stack effect of an instruction (fall-through path).
fn stack_effect(op: &Op) -> i32 {
    use Op::*;
    match op {
        Const(_) | Unit | True | False | Dup | GetLocal(_) | GetGlobal(_) | GetUpvalue(_)
        | PushFn(_) | PushNative(_) | Closure(_) | TestVariant(_) | MakeStructEmpty(_) => 1,
        Dup2 => 2,
        Pop | SetLocal(_) | SetGlobal(_) | SetUpvalue(_) | JumpIfFalse(_) | Add | Sub | Mul
        | Div | Rem | Eq | Lt | Le | Gt | Ge | StructSetField(_) | Index | MakeRange { .. } => -1,
        PopN(n) | EndBlock(n) | PopScope(n) => -(*n as i32),
        Jump(_) | JumpIfFalsePeek(_) | JumpIfTruePeek(_) | Neg | Not | ToString | GetField(_)
        | TupleGet(_) | GetVariantField(_) | MatchFail => 0,
        Call(n) | TailCall(n) => -(*n as i32),
        CallFn(_, n) | TailCallFn(_, n) | CallNative(_, n) => 1 - (*n as i32),
        Return => -1,
        Concat(n) | MakeList(n) | MakeTuple(n) => 1 - (*n as i32),
        MakeMap(n) => 1 - 2 * (*n as i32),
        MakeVariant { arity, .. } => 1 - (*arity as i32),
        SetField(_) => -2,
        IndexSet => -3,
        ForPrep => 1,
        // ForNext's push is accounted manually at the call site.
        ForNext(_) => 0,
    }
}
