//! The type checker: name resolution, type inference, and pattern analysis.
//!
//! Runs in four passes over a program:
//! 1. predeclare user types (so mutually recursive types resolve), then fill in
//!    their field/variant types;
//! 2. collect function signatures (functions are hoisted);
//! 3. check top-level statements in order, defining global slots;
//! 4. check function bodies (which can see every global).
//!
//! Produces side tables keyed by `NodeId` — inferred types and name/method
//! resolutions — that the bytecode compiler consumes. Inference is local
//! unification: explicit `[T]` lists introduce rigid `Type::Param`s inside a
//! declaration and are instantiated with fresh variables at each use site.
//! The checker survives errors (reporting and continuing with fresh types), so
//! one run reports as much as possible.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::builtins::{MathMember, Native, Recv};
use crate::diag::Diagnostic;
use crate::patterns::{self, Ctor, DPat};
use crate::span::{NodeId, Span};
use crate::types::{
    display_type, substitute, DefId, Defs, EnumDef, StructDef, Type, TypeDef, Unifier, Variant,
    OPTION_DEF, RESULT_DEF,
};

/// Name/method resolution for a node, consumed by the compiler.
#[derive(Debug, Clone)]
pub enum Res {
    /// A local slot (unique LocalId across the program).
    Local(u32),
    /// A global slot.
    Global(u32),
    /// A declared function (index into the checker's `fns`).
    Fn(u32),
    /// A native free function or `math.*` function.
    NativeFn(Native),
    /// A float constant (`math.pi`).
    FloatConst(f64),
    /// Enum variant (construction site or pattern).
    Variant { def: DefId, variant: u32 },
    /// Struct field access `expr.field`.
    Field { def: DefId, index: u32 },
    /// Tuple component access `expr.0`.
    TupleIndex(u32),
    /// Builtin method call.
    Method(Native),
    /// A module-qualified function call `alias.f(args)`: like `Fn`, but the
    /// receiver is a namespace, so the compiler must not push it.
    ModuleFn(u32),
    /// Struct literal: `field_order[i]` is the def-order index of the i-th
    /// written field.
    StructLit { def: DefId, field_order: Vec<u32> },
    /// Struct pattern, same convention.
    StructPat { def: DefId, field_order: Vec<u32> },
}

#[derive(Debug, Clone)]
pub struct FnInfo {
    /// Visible to importing modules (`pub fn` / `pub` method).
    pub is_pub: bool,
    pub name: String,
    pub generics: Vec<String>,
    /// Parameter types (may contain `Param(i)`).
    pub params: Vec<Type>,
    pub ret: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct GlobalInfo {
    pub is_pub: bool,
    pub name: String,
    pub mutable: bool,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct LocalInfo {
    pub name: String,
    pub mutable: bool,
    pub ty: Type,
    pub span: Span,
}

/// Where a fresh inference variable came from, for "cannot infer" reporting.
#[derive(Clone)]
struct VarOrigin {
    var: u32,
    span: Span,
    what: &'static str,
    /// If true, an unconstrained variable silently defaults to `Unit`
    /// (used for `panic(..)` in statement position).
    default_unit: bool,
}

#[derive(Clone)]
enum FnCtx {
    Fn { ret: Type },
    Lambda { ret: Type },
}

#[derive(Clone)]
pub struct Checker {
    pub defs: Defs,
    pub uni: Unifier,
    pub fns: Vec<FnInfo>,
    fn_by_name: HashMap<String, u32>,
    /// User-defined methods: `(type def, method name)` → index into `fns`.
    methods: HashMap<(DefId, String), u32>,
    pub globals: Vec<GlobalInfo>,
    global_by_name: HashMap<String, u32>,
    pub res: HashMap<NodeId, Res>,
    pub types: HashMap<NodeId, Type>,
    pub locals: Vec<LocalInfo>,
    pub diags: Vec<Diagnostic>,

    scopes: Vec<HashMap<String, u32>>,
    fn_stack: Vec<FnCtx>,
    loop_depth: Vec<u32>,
    /// Generic parameter names of the declaration being checked (for display
    /// and for resolving `T` in type expressions).
    generic_scope: Vec<String>,
    var_origins: Vec<VarOrigin>,
    reported_vars: HashSet<u32>,
    /// Expression nesting depth (recursion guard; the parser caps syntactic
    /// nesting, but left-associative operator chains build deep ASTs without
    /// parser recursion).
    expr_depth: u32,
    depth_error: bool,
    deferred_matches: Vec<DeferredMatch>,
    /// Locals allocated in the current function (slot-width guard).
    cur_fn_locals: u32,
    /// Names of all top-level `let`s in the current program (for better
    /// "used before declaration" errors), stored qualified.
    pending_globals: HashSet<String>,
    /// Name-mangling prefix of the module being checked ("" for the root
    /// module and the REPL): its top-level names register as "prefix.name".
    module_prefix: String,
    /// The current module's imports: alias → module key.
    imports: HashMap<String, String>,
    /// Whether the `let` statement currently being checked is `pub`.
    cur_let_is_pub: bool,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    pub fn new() -> Checker {
        Checker {
            defs: Defs::new(),
            uni: Unifier::new(),
            fns: Vec::new(),
            fn_by_name: HashMap::new(),
            methods: HashMap::new(),
            globals: Vec::new(),
            global_by_name: HashMap::new(),
            res: HashMap::new(),
            types: HashMap::new(),
            locals: Vec::new(),
            diags: Vec::new(),
            scopes: Vec::new(),
            fn_stack: Vec::new(),
            loop_depth: vec![0],
            generic_scope: Vec::new(),
            var_origins: Vec::new(),
            reported_vars: HashSet::new(),
            expr_depth: 0,
            depth_error: false,
            deferred_matches: Vec::new(),
            cur_fn_locals: 0,
            pending_globals: HashSet::new(),
            module_prefix: String::new(),
            imports: HashMap::new(),
            cur_let_is_pub: false,
        }
    }

    /// Check a whole program (callable repeatedly for REPL sessions; state
    /// accumulates).
    pub fn check_program(&mut self, program: &Program) {
        self.check_module(program, "", HashMap::new());
    }

    /// Check one module of a multi-file program. `prefix` mangles the
    /// module's top-level names ("" for the root module); `imports` maps this
    /// module's aliases to module keys. Modules must be checked in dependency
    /// order (the loader's output order).
    pub fn check_module(
        &mut self,
        program: &Program,
        prefix: &str,
        imports: HashMap<String, String>,
    ) {
        self.module_prefix = prefix.to_string();
        self.imports = imports;
        self.predeclare_types(program);
        self.define_type_bodies(program);
        self.collect_fns(program);

        self.pending_globals.clear();
        let mut let_names = HashSet::new();
        for stmt in &program.stmts {
            if let StmtKind::Let { pattern, .. } = &stmt.kind {
                collect_pattern_names(pattern, &mut let_names);
            }
        }
        self.pending_globals = let_names.iter().map(|n| self.qualify(n)).collect();

        debug_assert!(self.scopes.is_empty());
        for stmt in &program.stmts {
            self.check_stmt(stmt, false);
        }
        for stmt in &program.stmts {
            match &stmt.kind {
                StmtKind::Fn(f) => self.check_fn_body(f),
                StmtKind::Impl(im) => {
                    for m in &im.methods {
                        self.check_fn_body(m);
                    }
                }
                _ => {}
            }
        }
        self.finalize();
    }

    pub fn take_diags(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diags)
    }

    // ------------------------------------------------------------------
    // Pass 1: types
    // ------------------------------------------------------------------

    // ------------------------------------------------------------------
    // Module-aware name lookup
    // ------------------------------------------------------------------

    /// The stored (mangled) form of one of the current module's top-level
    /// names.
    fn qualify(&self, name: &str) -> String {
        if self.module_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}.{name}", self.module_prefix)
        }
    }

    fn lookup_fn(&self, name: &str) -> Option<u32> {
        self.fn_by_name.get(&self.qualify(name)).copied()
    }

    fn lookup_global(&self, name: &str) -> Option<u32> {
        self.global_by_name.get(&self.qualify(name)).copied()
    }

    /// A type visible under its unqualified name: one of the current
    /// module's own, or a prelude type (Option/Result are visible
    /// everywhere).
    fn lookup_def_local(&self, name: &str) -> Option<DefId> {
        if let Some(d) = self.defs.lookup(&self.qualify(name)) {
            return Some(d);
        }
        if !self.module_prefix.is_empty() && matches!(name, "Option" | "Result") {
            return self.defs.lookup(name);
        }
        None
    }

    /// Resolve a possibly module-qualified type name: "alias.Type" through
    /// the current imports, otherwise a local/prelude lookup. A foreign type
    /// must be `pub` (E0339); the def is still returned so checking
    /// continues gracefully.
    fn resolve_def_name(&mut self, name: &str, span: Span) -> Option<DefId> {
        match name.split_once('.') {
            Some((alias, rest)) => {
                let key = self.imports.get(alias)?;
                let def = self.defs.lookup(&format!("{key}.{rest}"))?;
                let is_pub = match self.defs.get(def) {
                    TypeDef::Struct(s) => s.is_pub,
                    TypeDef::Enum(e) => e.is_pub,
                };
                if !is_pub {
                    self.private_item(span, "type", name, rest);
                }
                Some(def)
            }
            None => self.lookup_def_local(name),
        }
    }

    /// The module prefix embedded in a stored (qualified) name: everything
    /// before the last `.` segment ("" for root-module names).
    fn name_module(stored: &str) -> &str {
        match stored.rfind('.') {
            Some(i) => &stored[..i],
            None => "",
        }
    }

    fn private_item(&mut self, span: Span, what: &str, shown: &str, bare: &str) {
        self.diags.push(
            Diagnostic::error("E0339", format!("{what} `{shown}` is private"))
                .with_label(span, "not exported by its module")
                .with_note(format!("add `pub` to `{bare}` in the defining module")),
        );
    }

    fn predeclare_types(&mut self, program: &Program) {
        const RESERVED: &[&str] = &[
            "Int", "Float", "Bool", "String", "Unit", "List", "Map", "Range", "Bytes", "Worker",
            "Window",
        ];
        for stmt in &program.stmts {
            let (name, span, is_struct, generics, is_pub) = match &stmt.kind {
                StmtKind::Struct(s) => (&s.name, s.name.span, true, &s.generics, s.is_pub),
                StmtKind::Enum(e) => (&e.name, e.name.span, false, &e.generics, e.is_pub),
                _ => continue,
            };
            if RESERVED.contains(&name.name.as_str()) {
                self.diags.push(
                    Diagnostic::error(
                        "E0402",
                        format!("cannot redefine builtin type `{}`", name.name),
                    )
                    .with_label(span, ""),
                );
                continue;
            }
            if self.defs.lookup(&self.qualify(&name.name)).is_some() {
                self.diags.push(
                    Diagnostic::error(
                        "E0403",
                        format!("duplicate type name `{}`", name.name),
                    )
                    .with_label(span, "redefined here"),
                );
                continue;
            }
            let mut seen = HashSet::new();
            for g in generics {
                if !seen.insert(g.name.clone()) {
                    self.diags.push(
                        Diagnostic::error(
                            "E0404",
                            format!("duplicate type parameter `{}`", g.name),
                        )
                        .with_label(g.span, ""),
                    );
                }
            }
            let generic_names: Vec<String> = generics.iter().map(|g| g.name.clone()).collect();
            let stored = self.qualify(&name.name);
            let def = if is_struct {
                TypeDef::Struct(StructDef {
                    is_pub,
                    name: stored,
                    generics: generic_names,
                    fields: Vec::new(),
                })
            } else {
                TypeDef::Enum(EnumDef {
                    is_pub,
                    name: stored,
                    generics: generic_names,
                    variants: Vec::new(),
                })
            };
            self.defs.add(def);
        }
    }

    fn define_type_bodies(&mut self, program: &Program) {
        for stmt in &program.stmts {
            match &stmt.kind {
                StmtKind::Struct(s) => {
                    let Some(def_id) = self.defs.lookup(&self.qualify(&s.name.name)) else {
                        continue;
                    };
                    if self.defs.get(def_id).name() != self.qualify(&s.name.name)
                        || !matches!(self.defs.get(def_id), TypeDef::Struct(_))
                    {
                        continue;
                    }
                    self.generic_scope = s.generics.iter().map(|g| g.name.clone()).collect();
                    if s.fields.len() > 60_000 {
                        self.diags.push(
                            Diagnostic::error(
                                "E0325",
                                format!(
                                    "struct `{}` has {} fields; the limit is 60,000",
                                    s.name.name,
                                    s.fields.len()
                                ),
                            )
                            .with_label(s.name.span, ""),
                        );
                    }
                    let mut fields = Vec::new();
                    let mut seen = HashSet::new();
                    for f in &s.fields {
                        if !seen.insert(f.name.name.clone()) {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0405",
                                    format!("duplicate field `{}`", f.name.name),
                                )
                                .with_label(f.name.span, ""),
                            );
                            continue;
                        }
                        let ty = self.resolve_type_expr(&f.ty);
                        fields.push((f.name.name.clone(), ty));
                    }
                    if let TypeDef::Struct(sd) = &mut self.defs.types[def_id as usize] {
                        sd.fields = fields;
                    }
                    self.generic_scope.clear();
                }
                StmtKind::Enum(e) => {
                    let Some(def_id) = self.defs.lookup(&self.qualify(&e.name.name)) else {
                        continue;
                    };
                    if !matches!(self.defs.get(def_id), TypeDef::Enum(_)) {
                        continue;
                    }
                    self.generic_scope = e.generics.iter().map(|g| g.name.clone()).collect();
                    let mut variants = Vec::new();
                    let mut seen = HashSet::new();
                    for v in &e.variants {
                        if !seen.insert(v.name.name.clone()) {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0406",
                                    format!("duplicate variant `{}`", v.name.name),
                                )
                                .with_label(v.name.span, ""),
                            );
                            continue;
                        }
                        if v.fields.len() > 255 {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0325",
                                    format!(
                                        "variant `{}` has {} fields; the limit is 255",
                                        v.name.name,
                                        v.fields.len()
                                    ),
                                )
                                .with_label(v.name.span, ""),
                            );
                        }
                        let fields: Vec<Type> =
                            v.fields.iter().map(|t| self.resolve_type_expr(t)).collect();
                        variants.push(Variant { name: v.name.name.clone(), fields });
                    }
                    if let TypeDef::Enum(ed) = &mut self.defs.types[def_id as usize] {
                        ed.variants = variants;
                    }
                    self.generic_scope.clear();
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------
    // Pass 2: function signatures
    // ------------------------------------------------------------------

    fn collect_fns(&mut self, program: &Program) {
        for stmt in &program.stmts {
            match &stmt.kind {
                StmtKind::Fn(f) => {
                    let stored = self.qualify(&f.name.name);
                    if self.fn_by_name.contains_key(&stored) {
                        self.diags.push(
                            Diagnostic::error(
                                "E0407",
                                format!("duplicate function `{}`", f.name.name),
                            )
                            .with_label(f.name.span, "redefined here"),
                        );
                        // Fall through: register anyway so the body still
                        // checks (the later definition wins for name lookup).
                    }
                    let idx = self.register_fn(f, &[], stored.clone());
                    self.fn_by_name.insert(stored, idx);
                }
                StmtKind::Impl(im) => self.collect_impl(im),
                _ => {}
            }
        }
    }

    /// Validate and register one function (or method) signature. `outer`
    /// holds enclosing generic binders (an impl block's); the function's own
    /// generics are appended after them, matching `Param` indices.
    fn register_fn(&mut self, f: &FnDecl, outer: &[String], name: String) -> u32 {
        let mut seen: HashSet<String> = outer.iter().cloned().collect();
        for g in &f.generics {
            if !seen.insert(g.name.clone()) {
                self.diags.push(
                    Diagnostic::error(
                        "E0404",
                        format!("duplicate type parameter `{}`", g.name),
                    )
                    .with_label(g.span, ""),
                );
            }
        }
        if f.params.len() > 255 {
            self.diags.push(
                Diagnostic::error(
                    "E0325",
                    format!(
                        "function `{}` has {} parameters; the limit is 255",
                        name,
                        f.params.len()
                    ),
                )
                .with_label(f.name.span, ""),
            );
        }
        let generics: Vec<String> = outer
            .iter()
            .cloned()
            .chain(f.generics.iter().map(|g| g.name.clone()))
            .collect();
        self.generic_scope = generics.clone();
        let params: Vec<Type> = f.params.iter().map(|p| self.resolve_type_expr(&p.ty)).collect();
        let ret = match &f.ret {
            Some(t) => self.resolve_type_expr(t),
            None => Type::Unit,
        };
        self.generic_scope.clear();

        let idx = self.fns.len() as u32;
        self.fns.push(FnInfo {
            is_pub: f.is_pub,
            name,
            generics,
            params,
            ret,
            span: f.name.span,
        });
        self.res.insert(f.id, Res::Fn(idx));
        idx
    }

    fn collect_impl(&mut self, im: &ImplDecl) {
        let tname = im.ty_name.name.as_str();
        let def = match self.lookup_def_local(tname) {
            Some(d) if d != OPTION_DEF && d != RESULT_DEF => d,
            _ => {
                let msg = if matches!(
                    tname,
                    "Int" | "Float" | "Bool" | "String" | "Unit" | "Range" | "Bytes" | "Worker"
                        | "Window" | "List" | "Map" | "Option" | "Result"
                ) {
                    format!("cannot define methods on the built-in type `{tname}`")
                } else {
                    format!("cannot `impl` unknown type `{tname}`")
                };
                self.diags.push(
                    Diagnostic::error("E0331", msg)
                        .with_label(im.ty_name.span, "not a user-defined struct or enum"),
                );
                return;
            }
        };
        let want = self.defs.get(def).generics().len();
        if im.generics.len() != want {
            self.diags.push(
                Diagnostic::error(
                    "E0332",
                    format!(
                        "`{tname}` has {want} type parameter{}, but this impl binds {}",
                        if want == 1 { "" } else { "s" },
                        im.generics.len()
                    ),
                )
                .with_label(im.ty_name.span, "type parameter count must match the declaration"),
            );
            return;
        }
        let outer: Vec<String> = im.generics.iter().map(|g| g.name.clone()).collect();
        for m in &im.methods {
            let key = (def, m.name.name.clone());
            if let Some(&prev) = self.methods.get(&key) {
                let prev_span = self.fns[prev as usize].span;
                self.diags.push(
                    Diagnostic::error(
                        "E0333",
                        format!("duplicate method `{}` on `{tname}`", m.name.name),
                    )
                    .with_label(m.name.span, "redefined here")
                    .with_secondary(prev_span, "first defined here"),
                );
                continue;
            }
            let stored = format!("{}.{}", self.defs.get(def).name(), m.name.name);
            let idx = self.register_fn(m, &outer, stored);
            self.methods.insert(key, idx);
        }
    }

    // ------------------------------------------------------------------
    // Type expressions
    // ------------------------------------------------------------------

    fn resolve_type_expr(&mut self, t: &TypeExpr) -> Type {
        let ty = match &t.kind {
            TypeExprKind::Unit => Type::Unit,
            TypeExprKind::Tuple(ts) => {
                Type::Tuple(ts.iter().map(|t| self.resolve_type_expr(t)).collect())
            }
            TypeExprKind::Fn { params, ret } => Type::Fn(
                params.iter().map(|t| self.resolve_type_expr(t)).collect(),
                Box::new(match ret {
                    Some(r) => self.resolve_type_expr(r),
                    None => Type::Unit,
                }),
            ),
            TypeExprKind::Named { name, args } => {
                let n = name.name.as_str();
                let arity_error = |me: &mut Self, expected: usize| {
                    me.diags.push(
                        Diagnostic::error(
                            "E0408",
                            format!(
                                "wrong number of type arguments for `{}`: expected {}, found {}",
                                n,
                                expected,
                                args.len()
                            ),
                        )
                        .with_label(t.span, ""),
                    );
                };
                match n {
                    "Int" | "Float" | "Bool" | "String" | "Unit" | "Range" | "Bytes"
                    | "Worker" | "Window" => {
                        if !args.is_empty() {
                            arity_error(self, 0);
                        }
                        match n {
                            "Int" => Type::Int,
                            "Float" => Type::Float,
                            "Bool" => Type::Bool,
                            "String" => Type::Str,
                            "Unit" => Type::Unit,
                            "Bytes" => Type::Bytes,
                            "Worker" => Type::Worker,
                            "Window" => Type::Window,
                            _ => Type::Range,
                        }
                    }
                    "List" => {
                        if args.len() != 1 {
                            arity_error(self, 1);
                            Type::List(Box::new(self.fresh(t.span, "type argument")))
                        } else {
                            Type::List(Box::new(self.resolve_type_expr(&args[0])))
                        }
                    }
                    "Map" => {
                        if args.len() != 2 {
                            arity_error(self, 2);
                            let k = self.fresh(t.span, "type argument");
                            let v = self.fresh(t.span, "type argument");
                            Type::Map(Box::new(k), Box::new(v))
                        } else {
                            Type::Map(
                                Box::new(self.resolve_type_expr(&args[0])),
                                Box::new(self.resolve_type_expr(&args[1])),
                            )
                        }
                    }
                    _ => {
                        // Generic parameter in scope?
                        if let Some(i) = self.generic_scope.iter().position(|g| g == n) {
                            if !args.is_empty() {
                                arity_error(self, 0);
                            }
                            Type::Param(i as u32)
                        } else if let Some(def) = self.resolve_def_name(n, name.span) {
                            let want = self.defs.get(def).generics().len();
                            if args.len() != want {
                                arity_error(self, want);
                                let fresh: Vec<Type> = (0..want)
                                    .map(|_| self.fresh(t.span, "type argument"))
                                    .collect();
                                Type::Named(def, fresh)
                            } else {
                                Type::Named(
                                    def,
                                    args.iter().map(|a| self.resolve_type_expr(a)).collect(),
                                )
                            }
                        } else {
                            let mut d = Diagnostic::error(
                                "E0401",
                                format!("unknown type `{}`", n),
                            )
                            .with_label(name.span, "not found");
                            if let Some((alias, _)) = n.split_once('.') {
                                if !self.imports.contains_key(alias) {
                                    d = d.with_note(format!(
                                        "`{alias}` is not an imported module in this file"
                                    ));
                                }
                            } else if let Some(sugg) = self.suggest_type(n) {
                                d = d.with_note(format!("did you mean `{sugg}`?"));
                            }
                            self.diags.push(d);
                            self.fresh(t.span, "unknown type")
                        }
                    }
                }
            }
        };
        self.types.insert(t.id, ty.clone());
        ty
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    fn check_stmt(&mut self, stmt: &Stmt, in_fn: bool) {
        match &stmt.kind {
            StmtKind::Fn(_) | StmtKind::Struct(_) | StmtKind::Enum(_) | StmtKind::Impl(_) => {}
            StmtKind::Import { path, alias, .. } => {
                // Loading happened before checking; here we only validate that
                // the alias was actually provided by the module loader (it is
                // absent in the REPL and in one-shot string evaluation).
                let alias_name = alias
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| path.last().unwrap().name.clone());
                if !self.imports.contains_key(&alias_name) {
                    self.diags.push(
                        Diagnostic::error(
                            "E0334",
                            "imports are not available in one-shot evaluation",
                        )
                        .with_label(stmt.span, "cannot import here"),
                    );
                }
            }
            StmtKind::Let { is_pub, mutable, pattern, ty, init } => {
                self.cur_let_is_pub = *is_pub;
                let annotated = ty.as_ref().map(|t| self.resolve_type_expr(t));
                let init_ty = self.check_expr(init, annotated.as_ref());
                let bound_ty = match &annotated {
                    Some(want) => {
                        // `let m: Map[..] = {};` — `{}` is an empty block, and
                        // the generic Unit-vs-Map mismatch hides the fix.
                        let empty_block_as_map = matches!(
                            &init.kind,
                            ExprKind::Block(b) if b.stmts.is_empty()
                        ) && matches!(
                            self.uni.shallow_resolve(want),
                            Type::Map(_, _)
                        );
                        if empty_block_as_map {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0301",
                                    "`{}` is an empty block, not an empty map",
                                )
                                .with_label(init.span, "this has type `Unit`")
                                .with_note("the empty map literal is `{:}`"),
                            );
                        } else {
                            self.expect_type(
                                want,
                                &init_ty,
                                init.span,
                                ty.as_ref().map(|t| t.span),
                            );
                        }
                        want.clone()
                    }
                    None => init_ty,
                };
                let mut binds = PatBinds::default();
                let mut seen = HashSet::new();
                self.check_pattern(pattern, &bound_ty, &mut binds, &mut seen);
                self.assert_irrefutable(pattern, "a `let` binding");
                self.materialize_binds(binds, *mutable);
                let _ = in_fn;
            }
            StmtKind::Assign { target, op, value } => {
                let target_ty = self.check_expr(target, None);
                self.check_assignable(target);
                let value_ty = self.check_expr(value, Some(&target_ty));
                self.expect_type(&target_ty, &value_ty, value.span, Some(target.span));
                if let Some(op) = op {
                    self.check_arith_operand(*op, &target_ty, target.span);
                }
            }
            StmtKind::Expr { expr, .. } => {
                self.check_expr(expr, None);
            }
            StmtKind::While { cond, body } => {
                let cond_ty = self.check_expr(cond, Some(&Type::Bool));
                self.expect_type(&Type::Bool, &cond_ty, cond.span, None);
                *self.loop_depth.last_mut().unwrap() += 1;
                let body_ty = self.check_block(body, None);
                self.expect_unit_body(&body_ty, body);
                *self.loop_depth.last_mut().unwrap() -= 1;
            }
            StmtKind::For { pattern, iter, body } => {
                let iter_ty = self.check_expr(iter, None);
                let elem_ty = match self.uni.shallow_resolve(&iter_ty) {
                    Type::List(t) => *t,
                    Type::Range => Type::Int,
                    Type::Str => Type::Str,
                    Type::Var(_) => {
                        self.diags.push(
                            Diagnostic::error(
                                "E0302",
                                "the type of this expression must be known to iterate over it",
                            )
                            .with_label(iter.span, "type is not yet known here"),
                        );
                        self.fresh(iter.span, "loop element")
                    }
                    other => {
                        self.diags.push(
                            Diagnostic::error(
                                "E0303",
                                format!(
                                    "cannot iterate over a value of type `{}`",
                                    self.show(&other)
                                ),
                            )
                            .with_label(iter.span, "`for` needs a List, Range, or String"),
                        );
                        self.fresh(iter.span, "loop element")
                    }
                };
                self.scopes.push(HashMap::new());
                let mut binds = PatBinds::default();
                let mut seen = HashSet::new();
                self.check_pattern(pattern, &elem_ty, &mut binds, &mut seen);
                self.assert_irrefutable(pattern, "a `for` loop");
                self.materialize_binds(binds, false);
                *self.loop_depth.last_mut().unwrap() += 1;
                let body_ty = self.check_block(body, None);
                self.expect_unit_body(&body_ty, body);
                *self.loop_depth.last_mut().unwrap() -= 1;
                self.scopes.pop();
            }
            StmtKind::Return(value) => {
                let Some(ctx) = self.fn_stack.last() else {
                    self.diags.push(
                        Diagnostic::error("E0304", "`return` outside of a function")
                            .with_label(stmt.span, ""),
                    );
                    if let Some(v) = value {
                        self.check_expr(v, None);
                    }
                    return;
                };
                let ret = match ctx {
                    FnCtx::Fn { ret } | FnCtx::Lambda { ret } => ret.clone(),
                };
                match value {
                    Some(v) => {
                        let t = self.check_expr(v, Some(&ret));
                        self.expect_type(&ret, &t, v.span, None);
                    }
                    None => {
                        self.expect_type(&ret, &Type::Unit, stmt.span, None);
                    }
                }
            }
            StmtKind::Break | StmtKind::Continue => {
                if *self.loop_depth.last().unwrap() == 0 {
                    let what = if matches!(stmt.kind, StmtKind::Break) { "break" } else { "continue" };
                    self.diags.push(
                        Diagnostic::error(
                            "E0305",
                            format!("`{what}` outside of a loop"),
                        )
                        .with_label(stmt.span, ""),
                    );
                }
            }
        }
    }

    fn expect_unit_body(&mut self, body_ty: &Type, body: &Block) {
        let t = self.uni.shallow_resolve(body_ty);
        if matches!(t, Type::Var(_)) {
            let _ = self.uni.unify(body_ty, &Type::Unit);
            return;
        }
        if t != Type::Unit {
            let span = body
                .stmts
                .last()
                .map(|s| s.span)
                .unwrap_or(body.span);
            self.diags.push(
                Diagnostic::error(
                    "E0306",
                    format!(
                        "loop body must not produce a value (found `{}`)",
                        self.show(&t)
                    ),
                )
                .with_label(span, "help: add a `;` to discard the value"),
            );
        }
    }

    fn check_assignable(&mut self, target: &Expr) {
        match &target.kind {
            ExprKind::Var(name) => match self.res.get(&target.id) {
                Some(Res::Local(id)) => {
                    let info = &self.locals[*id as usize];
                    if !info.mutable {
                        let (dspan, dname) = (info.span, info.name.clone());
                        self.diags.push(
                            Diagnostic::error(
                                "E0307",
                                format!("cannot assign to immutable binding `{dname}`"),
                            )
                            .with_label(target.span, "cannot assign")
                            .with_secondary(dspan, "declared without `mut` here")
                            .with_note(format!("declare it as `let mut {dname} = ...`")),
                        );
                    }
                }
                Some(Res::Global(slot)) => {
                    let info = &self.globals[*slot as usize];
                    if !info.mutable {
                        let (dspan, dname) = (info.span, info.name.clone());
                        self.diags.push(
                            Diagnostic::error(
                                "E0307",
                                format!("cannot assign to immutable binding `{dname}`"),
                            )
                            .with_label(target.span, "cannot assign")
                            .with_secondary(dspan, "declared without `mut` here")
                            .with_note(format!("declare it as `let mut {dname} = ...`")),
                        );
                    }
                }
                _ => {
                    self.diags.push(
                        Diagnostic::error(
                            "E0308",
                            format!("`{name}` is not an assignable variable"),
                        )
                        .with_label(target.span, ""),
                    );
                }
            },
            ExprKind::Field { .. } => {
                if matches!(self.res.get(&target.id), Some(Res::TupleIndex(_))) {
                    self.diags.push(
                        Diagnostic::error("E0309", "tuples are immutable")
                            .with_label(target.span, "cannot assign to a tuple component"),
                    );
                }
                if matches!(
                    self.res.get(&target.id),
                    Some(Res::Global(_) | Res::Fn(_))
                ) {
                    self.diags.push(
                        Diagnostic::error(
                            "E0308",
                            "cannot assign to a module member from outside its module",
                        )
                        .with_label(target.span, "modules own their bindings"),
                    );
                }
                // Struct fields are mutable; other Field resolutions
                // (constants, variants) already errored during checking.
            }
            ExprKind::Index { .. } => {
                // List/Map index assignment is always allowed; type errors are
                // reported by the index check itself.
            }
            _ => {
                // Parser already rejected other targets.
            }
        }
    }

    // ------------------------------------------------------------------
    // Blocks
    // ------------------------------------------------------------------

    fn check_block(&mut self, block: &Block, expected: Option<&Type>) -> Type {
        self.scopes.push(HashMap::new());
        let mut result = Type::Unit;
        let n = block.stmts.len();
        for (i, stmt) in block.stmts.iter().enumerate() {
            let last = i + 1 == n;
            if last {
                match &stmt.kind {
                    StmtKind::Expr { expr, tail: true } => {
                        result = self.check_expr(expr, expected);
                    }
                    StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue => {
                        self.check_stmt(stmt, true);
                        // Diverges: the block can take any type.
                        result = self.fresh_defaulting(stmt.span, "diverging block");
                    }
                    // `while true { .. }` with no way to break only leaves via
                    // `return` (or never) — it diverges like a trailing
                    // `return`, so no dead expression is needed after it.
                    StmtKind::While { cond, body }
                        if matches!(cond.kind, ExprKind::Bool(true))
                            && !block_contains_break(body) =>
                    {
                        self.check_stmt(stmt, true);
                        result = self.fresh_defaulting(stmt.span, "diverging block");
                    }
                    _ => {
                        self.check_stmt(stmt, true);
                        result = Type::Unit;
                    }
                }
            } else {
                self.check_stmt(stmt, true);
            }
        }
        self.scopes.pop();
        self.types.insert(block.id, result.clone());
        result
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn record(&mut self, id: NodeId, ty: Type) -> Type {
        self.types.insert(id, ty.clone());
        ty
    }

    const MAX_EXPR_DEPTH: u32 = 20_000;

    fn check_expr(&mut self, e: &Expr, expected: Option<&Type>) -> Type {
        self.expr_depth += 1;
        if self.expr_depth > Self::MAX_EXPR_DEPTH {
            if !self.depth_error {
                self.depth_error = true;
                self.diags.push(
                    Diagnostic::error(
                        "E0324",
                        format!(
                            "expression exceeds {} nested operations",
                            Self::MAX_EXPR_DEPTH
                        ),
                    )
                    .with_label(e.span, "split this expression up"),
                );
            }
            self.expr_depth -= 1;
            let t = self.uni.fresh();
            return self.record(e.id, t);
        }
        let ty = self.check_expr_inner(e, expected);
        self.expr_depth -= 1;
        self.record(e.id, ty)
    }

    fn check_expr_inner(&mut self, e: &Expr, expected: Option<&Type>) -> Type {
        match &e.kind {
            ExprKind::Int(_) => Type::Int,
            ExprKind::Float(_) => Type::Float,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Str(_) => Type::Str,
            ExprKind::Unit => Type::Unit,
            ExprKind::StringInterp { exprs, .. } => {
                self.check_literal_len(exprs.len(), "string interpolation", e.span);
                for ex in exprs {
                    self.check_expr(ex, None);
                }
                Type::Str
            }
            ExprKind::Var(name) => self.check_var(e, name, expected, false),
            ExprKind::Field { base, field } => self.check_field(e, base, field),
            ExprKind::MethodCall { recv, method, args } => {
                self.check_method_call(e, recv, method, args, expected)
            }
            ExprKind::Call { callee, args } => self.check_call(e, callee, args, expected),
            ExprKind::Unary { op, expr } => {
                let t = self.check_expr(expr, None);
                match op {
                    UnOp::Neg => {
                        let r = self.uni.shallow_resolve(&t);
                        // `-x` on a user type dispatches to its `neg` method.
                        if let Type::Named(def, _) = &r {
                            if let Some(&idx) = self.methods.get(&(*def, "neg".to_string())) {
                                self.check_method_visible(*def, idx, "neg", e.span);
                                let info = self.fns[idx as usize].clone();
                                if info.params.len() != 1 {
                                    self.diags.push(
                                        Diagnostic::error(
                                            "E0317",
                                            "`neg` overloads unary `-`, so it must take \
                                             only `self`"
                                                .to_string(),
                                        )
                                        .with_label(e.span, "")
                                        .with_secondary(info.span, "method declared here"),
                                    );
                                    return self.fresh(e.span, "negation");
                                }
                                self.res.insert(e.id, Res::Fn(idx));
                                let inst: Vec<Type> = (0..info.generics.len())
                                    .map(|_| self.fresh(e.span, "type argument"))
                                    .collect();
                                let p0 = substitute(&info.params[0], &inst);
                                self.expect_type(&p0, &t, expr.span, None);
                                return substitute(&info.ret, &inst);
                            }
                        }
                        match r {
                            Type::Int | Type::Float => r,
                            Type::Var(_) => {
                                self.cannot_infer_here(expr.span, "operand of `-`");
                                t
                            }
                            other => {
                                self.diags.push(
                                    Diagnostic::error(
                                        "E0310",
                                        format!(
                                            "cannot negate a value of type `{}`",
                                            self.show(&other)
                                        ),
                                    )
                                    .with_label(expr.span, "expected `Int` or `Float`"),
                                );
                                self.fresh(e.span, "negation")
                            }
                        }
                    }
                    UnOp::Not => {
                        self.expect_type(&Type::Bool, &t, expr.span, None);
                        Type::Bool
                    }
                }
            }
            ExprKind::Try(inner) => self.check_try(e, inner),
            ExprKind::Binary { op, op_span, lhs, rhs } => {
                self.check_binary(e, *op, *op_span, lhs, rhs)
            }
            ExprKind::Index { base, index } => {
                let bt = self.check_expr(base, None);
                match self.uni.shallow_resolve(&bt) {
                    Type::List(elem) => {
                        let it = self.check_expr(index, Some(&Type::Int));
                        self.expect_type(&Type::Int, &it, index.span, None);
                        *elem
                    }
                    Type::Map(k, v) => {
                        let it = self.check_expr(index, Some(&k));
                        self.expect_type(&k, &it, index.span, None);
                        *v
                    }
                    Type::Str => {
                        self.diags.push(
                            Diagnostic::error("E0313", "strings cannot be indexed with `[]`")
                                .with_label(base.span, "")
                                .with_note("use `.chars()`, `.char_at(i)`, or `.slice(a, b)`"),
                        );
                        self.check_expr(index, None);
                        Type::Str
                    }
                    Type::Var(_) => {
                        self.cannot_infer_here(base.span, "indexed value");
                        self.check_expr(index, None);
                        self.fresh(e.span, "index result")
                    }
                    other => {
                        self.diags.push(
                            Diagnostic::error(
                                "E0314",
                                format!("cannot index a value of type `{}`", self.show(&other)),
                            )
                            .with_label(base.span, ""),
                        );
                        self.check_expr(index, None);
                        self.fresh(e.span, "index result")
                    }
                }
            }
            ExprKind::List(items) => {
                self.check_literal_len(items.len(), "list literal", e.span);
                let elem = match expected.map(|t| self.uni.shallow_resolve(t)) {
                    Some(Type::List(t)) => *t,
                    _ => self.fresh(e.span, "list element"),
                };
                for item in items {
                    let t = self.check_expr(item, Some(&elem));
                    self.expect_type(&elem, &t, item.span, None);
                }
                Type::List(Box::new(elem))
            }
            ExprKind::MapLit(entries) => {
                self.check_literal_len(entries.len(), "map literal", e.span);
                let (k, v) = match expected.map(|t| self.uni.shallow_resolve(t)) {
                    Some(Type::Map(k, v)) => (*k, *v),
                    _ => (
                        self.fresh(e.span, "map key"),
                        self.fresh(e.span, "map value"),
                    ),
                };
                for (ke, ve) in entries {
                    let kt = self.check_expr(ke, Some(&k));
                    self.expect_type(&k, &kt, ke.span, None);
                    let vt = self.check_expr(ve, Some(&v));
                    self.expect_type(&v, &vt, ve.span, None);
                }
                let zk = self.uni.zonk(&k);
                if zk.contains_fn(&self.defs) {
                    self.diags.push(
                        Diagnostic::error("E0312", "functions cannot be used as map keys")
                            .with_label(e.span, ""),
                    );
                }
                Type::Map(Box::new(k), Box::new(v))
            }
            ExprKind::Tuple(items) => {
                self.check_literal_len(items.len(), "tuple", e.span);
                let expected_items: Vec<Option<Type>> =
                    match expected.map(|t| self.uni.shallow_resolve(t)) {
                        Some(Type::Tuple(ts)) if ts.len() == items.len() => {
                            ts.into_iter().map(Some).collect()
                        }
                        _ => vec![None; items.len()],
                    };
                let types: Vec<Type> = items
                    .iter()
                    .zip(&expected_items)
                    .map(|(item, exp)| self.check_expr(item, exp.as_ref()))
                    .collect();
                Type::Tuple(types)
            }
            ExprKind::Range { lo, hi, .. } => {
                let lt = self.check_expr(lo, Some(&Type::Int));
                self.expect_type(&Type::Int, &lt, lo.span, None);
                let ht = self.check_expr(hi, Some(&Type::Int));
                self.expect_type(&Type::Int, &ht, hi.span, None);
                Type::Range
            }
            ExprKind::StructLit { name, fields } => self.check_struct_lit(e, name, fields),
            ExprKind::Lambda { params, ret, body } => {
                self.check_lambda(e, params, ret.as_ref(), body, expected)
            }
            ExprKind::If { cond, then, els } => {
                let ct = self.check_expr(cond, Some(&Type::Bool));
                self.expect_type(&Type::Bool, &ct, cond.span, None);
                match els {
                    Some(els) => {
                        let tt = self.check_block(then, expected);
                        let et = self.check_expr(els, expected.or(Some(&tt)));
                        if self.uni.unify(&tt, &et).is_err() {
                            let (tt_s, et_s) = (self.show(&tt), self.show(&et));
                            self.diags.push(
                                Diagnostic::error(
                                    "E0315",
                                    "`if` and `else` branches have incompatible types",
                                )
                                .with_label(
                                    els.span,
                                    format!("this branch has type `{et_s}`"),
                                )
                                .with_secondary(
                                    then.span,
                                    format!("this branch has type `{tt_s}`"),
                                ),
                            );
                        }
                        tt
                    }
                    None => {
                        let tt = self.check_block(then, Some(&Type::Unit));
                        let r = self.uni.shallow_resolve(&tt);
                        if matches!(r, Type::Var(_)) {
                            let _ = self.uni.unify(&tt, &Type::Unit);
                        } else if r != Type::Unit {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0316",
                                    format!(
                                        "`if` without `else` must have type `Unit`, found `{}`",
                                        self.show(&r)
                                    ),
                                )
                                .with_label(then.span, "help: add an `else` branch or a `;`"),
                            );
                        }
                        Type::Unit
                    }
                }
            }
            ExprKind::Block(block) => self.check_block(block, expected),
            ExprKind::Match { scrutinee, arms, sugar } => {
                self.check_match(e, scrutinee, arms, *sugar, expected)
            }
        }
    }

    fn check_var(
        &mut self,
        e: &Expr,
        name: &str,
        _expected: Option<&Type>,
        as_callee: bool,
    ) -> Type {
        // Locals (innermost scope first).
        for scope in self.scopes.iter().rev() {
            if let Some(&id) = scope.get(name) {
                self.res.insert(e.id, Res::Local(id));
                return self.locals[id as usize].ty.clone();
            }
        }
        // Globals.
        if let Some(slot) = self.lookup_global(name) {
            self.res.insert(e.id, Res::Global(slot));
            return self.globals[slot as usize].ty.clone();
        }
        // Declared functions.
        if let Some(idx) = self.lookup_fn(name) {
            self.res.insert(e.id, Res::Fn(idx));
            let info = self.fns[idx as usize].clone();
            let inst: Vec<Type> = (0..info.generics.len())
                .map(|_| self.fresh(e.span, "type argument"))
                .collect();
            let params: Vec<Type> = info.params.iter().map(|p| substitute(p, &inst)).collect();
            let ret = substitute(&info.ret, &inst);
            return Type::Fn(params, Box::new(ret));
        }
        // Native free functions.
        if let Some(native) = Native::free_fn(name) {
            self.res.insert(e.id, Res::NativeFn(native));
            return self.instantiate_native(native, &[], e.span);
        }
        // Prelude variants.
        if let Some((def, variant)) = prelude_variant(name) {
            self.res.insert(e.id, Res::Variant { def, variant });
            let TypeDef::Enum(ed) = self.defs.get(def) else { unreachable!() };
            let nfields = ed.variants[variant as usize].fields.len();
            let ngen = ed.generics.len();
            if nfields > 0 && !as_callee {
                self.diags.push(
                    Diagnostic::error(
                        "E0409",
                        format!("the constructor `{name}` must be called"),
                    )
                    .with_label(e.span, "")
                    .with_note(format!("to pass it as a function, wrap it: `|x| {name}(x)`")),
                );
            }
            let inst: Vec<Type> =
                (0..ngen).map(|_| self.fresh(e.span, "type argument")).collect();
            return Type::Named(def, inst);
        }
        if Native::is_namespace(name) {
            let hint = match name {
                "math" => "use `math.sin(..)`, `math.pi`, ...",
                "fs" => "use `fs.read(..)`, `fs.write(..)`, ...",
                "gpu" => "use `gpu.available()`, `gpu.run(..)`, ...",
                "window" => "use `window.create(..)`, ...",
                "gfx" => "use `gfx.compile_program(..)`, `gfx.draw_arrays(..)`, ...",
                _ => "use `os.args()`, `os.env(..)`, ...",
            };
            self.diags.push(
                Diagnostic::error("E0410", format!("`{name}` is a namespace, not a value"))
                    .with_label(e.span, hint),
            );
            return self.fresh(e.span, "namespace");
        }
        // An imported module used as a value.
        if self.imports.contains_key(name) {
            self.diags.push(
                Diagnostic::error(
                    "E0410",
                    format!("`{name}` is an imported module, not a value"),
                )
                .with_label(e.span, format!("use `{name}.member`")),
            );
            return self.fresh(e.span, "namespace");
        }
        // Enum/struct type used as a value.
        if let Some(def) = self.lookup_def_local(name) {
            let kind = match self.defs.get(def) {
                TypeDef::Enum(_) => "enum",
                TypeDef::Struct(_) => "struct",
            };
            self.diags.push(
                Diagnostic::error(
                    "E0411",
                    format!("`{name}` is a {kind} type, not a value"),
                )
                .with_label(e.span, ""),
            );
            return self.fresh(e.span, "type as value");
        }
        // Known global declared later in the file.
        if self.pending_globals.contains(&self.qualify(name)) && self.scopes.is_empty() {
            self.diags.push(
                Diagnostic::error(
                    "E0412",
                    format!("`{name}` is used before its `let` declaration"),
                )
                .with_label(e.span, "used here")
                .with_note("top-level code runs in order; move the `let` above this line"),
            );
            return self.fresh(e.span, "forward global");
        }
        let mut d = Diagnostic::error("E0400", format!("undefined name `{name}`"))
            .with_label(e.span, "not found in this scope");
        if let Some(s) = self.suggest_value(name) {
            d = d.with_note(format!("did you mean `{s}`?"));
        }
        self.diags.push(d);
        self.fresh(e.span, "undefined name")
    }

    /// A bare enum variant path used as a value: `Shape.Empty` (or
    /// module-qualified `geo.Shape.Empty`). Nullary variants construct; a
    /// payload-taking constructor must be called (E0409).
    fn check_variant_path(&mut self, e: &Expr, def: DefId, variant: &Ident) -> Type {
        let TypeDef::Enum(ed) = self.defs.get(def) else { unreachable!() };
        let ename = ed.name.clone();
        let Some(vidx) = ed.variants.iter().position(|v| v.name == variant.name) else {
            let variants: Vec<&str> = ed.variants.iter().map(|v| v.name.as_str()).collect();
            self.diags.push(
                Diagnostic::error(
                    "E0414",
                    format!("no variant `{}` on enum `{ename}`", variant.name),
                )
                .with_label(variant.span, "")
                .with_note(format!("variants: {}", variants.join(", "))),
            );
            return self.fresh(e.span, "unknown variant");
        };
        let TypeDef::Enum(ed) = self.defs.get(def) else { unreachable!() };
        let nfields = ed.variants[vidx].fields.len();
        let ngen = ed.generics.len();
        self.res.insert(e.id, Res::Variant { def, variant: vidx as u32 });
        if nfields > 0 {
            self.diags.push(
                Diagnostic::error(
                    "E0409",
                    format!("the constructor `{ename}.{}` must be called", variant.name),
                )
                .with_label(e.span, "")
                .with_note(format!(
                    "to pass it as a function, wrap it: `|x| {ename}.{}(x)`",
                    variant.name
                )),
            );
        }
        let inst: Vec<Type> = (0..ngen).map(|_| self.fresh(e.span, "type argument")).collect();
        Type::Named(def, inst)
    }

    /// `alias.member` where `alias` is an imported module: a function
    /// reference, a global, or an error.
    fn check_module_member(&mut self, e: &Expr, alias: &str, key: &str, field: &Ident) -> Type {
        let qname = format!("{key}.{}", field.name);
        if let Some(&idx) = self.fn_by_name.get(&qname) {
            if !self.fns[idx as usize].is_pub {
                self.private_item(
                    field.span,
                    "function",
                    &format!("{alias}.{}", field.name),
                    &field.name,
                );
            }
            self.res.insert(e.id, Res::Fn(idx));
            let info = self.fns[idx as usize].clone();
            let inst: Vec<Type> = (0..info.generics.len())
                .map(|_| self.fresh(e.span, "type argument"))
                .collect();
            let params: Vec<Type> = info.params.iter().map(|p| substitute(p, &inst)).collect();
            let ret = substitute(&info.ret, &inst);
            return Type::Fn(params, Box::new(ret));
        }
        if let Some(&slot) = self.global_by_name.get(&qname) {
            if !self.globals[slot as usize].is_pub {
                self.private_item(
                    field.span,
                    "binding",
                    &format!("{alias}.{}", field.name),
                    &field.name,
                );
            }
            self.res.insert(e.id, Res::Global(slot));
            return self.globals[slot as usize].ty.clone();
        }
        if let Some(def) = self.defs.lookup(&qname) {
            let kind = match self.defs.get(def) {
                TypeDef::Enum(_) => "enum",
                TypeDef::Struct(_) => "struct",
            };
            self.diags.push(
                Diagnostic::error(
                    "E0411",
                    format!("`{alias}.{}` is a {kind} type, not a value", field.name),
                )
                .with_label(e.span, ""),
            );
            return self.fresh(e.span, "type as value");
        }
        self.diags.push(
            Diagnostic::error(
                "E0413",
                format!("no such member `{alias}.{}`", field.name),
            )
            .with_label(field.span, "not found in the imported module"),
        );
        self.fresh(e.span, "unknown member")
    }

    fn check_field(&mut self, e: &Expr, base: &Expr, field: &Ident) -> Type {
        // Special bases: namespaces and enum paths.
        if let ExprKind::Var(name) = &base.kind {
            if !self.name_is_value(name) {
                if let Some(key) = self.imports.get(name).cloned() {
                    return self.check_module_member(e, name, &key, field);
                }
                if Native::is_namespace(name) {
                    match Native::namespace_member(name, &field.name) {
                        Some(MathMember::Const(v)) => {
                            self.res.insert(e.id, Res::FloatConst(v));
                            return Type::Float;
                        }
                        Some(MathMember::Fn(n)) => {
                            self.res.insert(e.id, Res::NativeFn(n));
                            return self.instantiate_native(n, &[], e.span);
                        }
                        None => {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0413",
                                    format!("no such member `{name}.{}`", field.name),
                                )
                                .with_label(field.span, ""),
                            );
                            return self.fresh(e.span, "unknown member");
                        }
                    }
                }
                if let Some(def) = self.lookup_def_local(name) {
                    if matches!(self.defs.get(def), TypeDef::Enum(_)) {
                        return self.check_variant_path(e, def, field);
                    }
                }
            }
        }

        // Module-qualified enum path: `alias.Enum.Variant` (nullary).
        // `alias.member.field` where `member` is a module VALUE falls
        // through to ordinary field access on the member (v0.7 fix, same
        // shape as the method-call case below).
        if let ExprKind::Field { base: inner, field: tyname } = &base.kind {
            if let ExprKind::Var(alias) = &inner.kind {
                if !self.name_is_value(alias) {
                    if let Some(key) = self.imports.get(alias.as_str()).cloned() {
                        if let Some(def) =
                            self.resolve_def_name(&format!("{alias}.{}", tyname.name), tyname.span)
                        {
                            if matches!(self.defs.get(def), TypeDef::Enum(_)) {
                                return self.check_variant_path(e, def, field);
                            }
                        }
                        let qname = format!("{key}.{}", tyname.name);
                        let is_value_member = self.fn_by_name.contains_key(&qname)
                            || self.global_by_name.contains_key(&qname);
                        if !is_value_member {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0413",
                                    format!(
                                        "no enum or member `{}` in module `{alias}`",
                                        tyname.name
                                    ),
                                )
                                .with_label(tyname.span, ""),
                            );
                            return self.fresh(e.span, "unknown member");
                        }
                    }
                }
            }
        }

        let bt = self.check_expr(base, None);
        match self.uni.shallow_resolve(&bt) {
            Type::Named(def, args) => match self.defs.get(def) {
                TypeDef::Struct(s) => {
                    match s.fields.iter().position(|(n, _)| *n == field.name) {
                        Some(idx) => {
                            let fty = substitute(&s.fields[idx].1, &args);
                            self.res
                                .insert(e.id, Res::Field { def, index: idx as u32 });
                            fty
                        }
                        None => {
                            let sname = s.name.clone();
                            let names: Vec<String> =
                                s.fields.iter().map(|(n, _)| n.clone()).collect();
                            let mut d = Diagnostic::error(
                                "E0415",
                                format!("no field `{}` on struct `{sname}`", field.name),
                            )
                            .with_label(field.span, "unknown field");
                            if let Some(sugg) = closest(&field.name, names.iter().map(|s| s.as_str()))
                            {
                                d = d.with_note(format!("did you mean `{sugg}`?"));
                            }
                            self.diags.push(d);
                            self.fresh(e.span, "unknown field")
                        }
                    }
                }
                TypeDef::Enum(en) => {
                    let ename = en.name.clone();
                    self.diags.push(
                        Diagnostic::error(
                            "E0416",
                            format!("`{ename}` is an enum; it has no fields"),
                        )
                        .with_label(e.span, "")
                        .with_note("use `match` to inspect enum values"),
                    );
                    self.fresh(e.span, "enum field")
                }
            },
            Type::Tuple(ts) => match field.name.parse::<usize>() {
                Ok(i) if i < ts.len() => {
                    self.res.insert(e.id, Res::TupleIndex(i as u32));
                    ts[i].clone()
                }
                Ok(i) => {
                    self.diags.push(
                        Diagnostic::error(
                            "E0417",
                            format!(
                                "tuple index `{i}` out of bounds (this tuple has {} elements)",
                                ts.len()
                            ),
                        )
                        .with_label(field.span, ""),
                    );
                    self.fresh(e.span, "tuple index")
                }
                Err(_) => {
                    self.diags.push(
                        Diagnostic::error(
                            "E0418",
                            format!(
                                "no field `{}` on a tuple; use `.0`, `.1`, ...",
                                field.name
                            ),
                        )
                        .with_label(field.span, ""),
                    );
                    self.fresh(e.span, "tuple field")
                }
            },
            Type::Var(_) => {
                self.cannot_infer_here(base.span, "field access base");
                self.fresh(e.span, "field")
            }
            other => {
                self.diags.push(
                    Diagnostic::error(
                        "E0419",
                        format!(
                            "no field `{}` on a value of type `{}`",
                            field.name,
                            self.show(&other)
                        ),
                    )
                    .with_label(e.span, ""),
                );
                self.fresh(e.span, "field")
            }
        }
    }

    fn name_is_value(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains_key(name))
            || self.global_by_name.contains_key(&self.qualify(name))
            || self.fn_by_name.contains_key(&self.qualify(name))
    }

    fn check_method_call(
        &mut self,
        e: &Expr,
        recv: &Expr,
        method: &Ident,
        args: &[Expr],
        _expected: Option<&Type>,
    ) -> Type {
        // Enum variant construction: `Shape.Circle(1.0)`, `Option.Some(x)`.
        if let ExprKind::Var(name) = &recv.kind {
            if !self.name_is_value(name) {
                if let Some(key) = self.imports.get(name).cloned() {
                    return self.check_module_call(e, name, &key, method, args);
                }
                if let Some(def) = self.lookup_def_local(name) {
                    if let TypeDef::Enum(_) = self.defs.get(def) {
                        return self.check_variant_ctor(e, def, name, method, args);
                    }
                }
                if Native::is_namespace(name) {
                    match Native::namespace_member(name, &method.name) {
                        Some(MathMember::Fn(n)) => {
                            // NativeFn (not Method): the receiver is a
                            // namespace, so the compiler must not push it.
                            self.res.insert(e.id, Res::NativeFn(n));
                            return self.check_native_call(n, &[], args, e.span, method.span);
                        }
                        Some(MathMember::Const(_)) => {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0420",
                                    format!("`{name}.{}` is a constant, not a function", method.name),
                                )
                                .with_label(method.span, ""),
                            );
                            return Type::Float;
                        }
                        None => {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0413",
                                    format!("no such member `{name}.{}`", method.name),
                                )
                                .with_label(method.span, ""),
                            );
                            for a in args {
                                self.check_expr(a, None);
                            }
                            return self.fresh(e.span, "unknown member");
                        }
                    }
                }
            }
        }

        // Module-qualified variant construction: `alias.Enum.Variant(args)`.
        // `alias.member.method(args)` where `member` is a module VALUE
        // (pub let / fn) is NOT this case — it falls through to ordinary
        // method dispatch on the member's value (fixed in the v0.7 demo
        // round: this arm used to hijack every alias.x.y(..) shape).
        if let ExprKind::Field { base: inner, field: tyname } = &recv.kind {
            if let ExprKind::Var(alias) = &inner.kind {
                if !self.name_is_value(alias) {
                    if let Some(key) = self.imports.get(alias.as_str()).cloned() {
                        if let Some(def) =
                            self.resolve_def_name(&format!("{alias}.{}", tyname.name), tyname.span)
                        {
                            if matches!(self.defs.get(def), TypeDef::Enum(_)) {
                                let ename = self.defs.get(def).name().to_string();
                                return self.check_variant_ctor(e, def, &ename, method, args);
                            }
                        }
                        let qname = format!("{key}.{}", tyname.name);
                        let is_value_member = self.fn_by_name.contains_key(&qname)
                            || self.global_by_name.contains_key(&qname);
                        if !is_value_member {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0413",
                                    format!(
                                        "no enum or member `{}` in module `{alias}`",
                                        tyname.name
                                    ),
                                )
                                .with_label(tyname.span, ""),
                            );
                            for a in args {
                                self.check_expr(a, None);
                            }
                            return self.fresh(e.span, "unknown member");
                        }
                    }
                }
            }
        }

        let rt = self.check_expr(recv, None);
        let resolved = self.uni.shallow_resolve(&rt);

        // User-defined methods (impl blocks) on structs and enums. Prelude
        // Option/Result defs never appear in the map (impls on them are
        // rejected), so builtin method dispatch below is unaffected.
        if let Type::Named(def, _) = &resolved {
            if let Some(&idx) = self.methods.get(&(*def, method.name.clone())) {
                self.check_method_visible(*def, idx, &method.name, method.span);
                return self.check_user_method_call(e, recv, &rt, method, args, idx);
            }
        }

        let (kind, recv_args): (Recv, Vec<Type>) = match &resolved {
            Type::Int => (Recv::Int, vec![]),
            Type::Float => (Recv::Float, vec![]),
            Type::Str => (Recv::Str, vec![]),
            Type::Range => (Recv::Range, vec![]),
            Type::Bytes => (Recv::Bytes, vec![]),
            Type::Worker => (Recv::Worker, vec![]),
            Type::Window => (Recv::Window, vec![]),
            Type::List(t) => (Recv::List, vec![(**t).clone()]),
            Type::Map(k, v) => (Recv::Map, vec![(**k).clone(), (**v).clone()]),
            Type::Named(d, args) if *d == OPTION_DEF => (Recv::Option_, args.clone()),
            Type::Named(d, args) if *d == RESULT_DEF => (Recv::Result_, args.clone()),
            Type::Var(_) => {
                self.cannot_infer_here(recv.span, "method receiver");
                for a in args {
                    self.check_expr(a, None);
                }
                return self.fresh(e.span, "method result");
            }
            other => {
                self.diags.push(
                    Diagnostic::error(
                        "E0421",
                        format!(
                            "no method `{}` on a value of type `{}`",
                            method.name,
                            self.show(other)
                        ),
                    )
                    .with_label(method.span, ""),
                );
                for a in args {
                    self.check_expr(a, None);
                }
                return self.fresh(e.span, "method result");
            }
        };

        let Some(native) = Native::method(kind, &method.name) else {
            self.diags.push(
                Diagnostic::error(
                    "E0422",
                    format!(
                        "no method `{}` on `{}` values",
                        method.name,
                        kind.describe()
                    ),
                )
                .with_label(method.span, "unknown method"),
            );
            for a in args {
                self.check_expr(a, None);
            }
            return self.fresh(e.span, "method result");
        };
        self.res.insert(e.id, Res::Method(native));

        // Extra constraints not expressible in the scheme.
        match native {
            Native::ListJoin => {
                if let Type::List(t) = &resolved {
                    let tt = self.uni.zonk(t);
                    if !matches!(tt, Type::Str | Type::Var(_)) {
                        self.diags.push(
                            Diagnostic::error(
                                "E0423",
                                format!(
                                    "`join` requires `List[String]`, found `List[{}]`",
                                    self.show(&tt)
                                ),
                            )
                            .with_label(recv.span, "")
                            .with_note("map the elements first: `.map(|x| str(x)).join(...)`"),
                        );
                    } else {
                        let _ = self.uni.unify(t, &Type::Str);
                    }
                }
            }
            Native::ListSort => {
                if let Type::List(t) = &resolved {
                    // Concrete non-sortable element types are compile errors;
                    // generic element types are checked at runtime (generics
                    // are erased, so a List[Int] through a generic fn sorts).
                    match self.uni.shallow_resolve(t) {
                        Type::Int | Type::Float | Type::Str | Type::Param(_) | Type::Var(_) => {}
                        other => {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0322",
                                    format!(
                                        "cannot sort elements of type `{}`",
                                        self.show(&other)
                                    ),
                                )
                                .with_label(recv.span, "sort() needs Int, Float, or String elements")
                                .with_note("use sort_by(|a, b| ...) with a custom comparator"),
                            );
                        }
                    }
                }
            }
            _ => {}
        }

        self.check_native_call(native, &recv_args, args, e.span, method.span)
    }

    /// Instantiate a native scheme and check a call against it.
    fn check_native_call(
        &mut self,
        native: Native,
        recv_args: &[Type],
        args: &[Expr],
        call_span: Span,
        name_span: Span,
    ) -> Type {
        let sig = native.sig();
        let n = sig.max_param.max(recv_args.len() as u32);
        let inst: Vec<Type> = (0..n as usize)
            .map(|i| {
                recv_args
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| self.fresh(call_span, "type argument"))
            })
            .collect();
        let params: Vec<Type> = sig.params.iter().map(|p| substitute(p, &inst)).collect();
        let ret = substitute(&sig.ret, &inst);

        if args.len() != params.len() {
            self.diags.push(
                Diagnostic::error(
                    "E0317",
                    format!(
                        "`{}` takes {} argument{}, found {}",
                        native.name(),
                        params.len(),
                        if params.len() == 1 { "" } else { "s" },
                        args.len()
                    ),
                )
                .with_label(name_span, ""),
            );
            for a in args {
                self.check_expr(a, None);
            }
            return ret;
        }
        for (a, p) in args.iter().zip(&params) {
            let at = self.check_expr(a, Some(p));
            self.expect_type(p, &at, a.span, None);
        }

        // Special: assert_eq / print-family on function-containing types.
        if matches!(native, Native::AssertEq) {
            if let Some(a0) = args.first() {
                let t = self.uni.zonk(&params[0]);
                if t.contains_fn(&self.defs) {
                    self.diags.push(
                        Diagnostic::error("E0311", "cannot compare functions")
                            .with_label(a0.span, ""),
                    );
                }
            }
        }
        // `panic(..)` and `os.exit(..)` have polymorphic results that default
        // to Unit when nothing constrains them (e.g. a bare `panic("boom");`
        // statement, including the REPL's hidden result binding).
        if matches!(native, Native::Panic | Native::OsExit) {
            if let Type::Var(v) = self.uni.shallow_resolve(&ret) {
                if let Some(o) = self.var_origins.iter_mut().find(|o| o.var == v) {
                    o.default_unit = true;
                } else {
                    self.var_origins.push(VarOrigin {
                        var: v,
                        span: call_span,
                        what: "exit result",
                        default_unit: true,
                    });
                }
            }
        }
        ret
    }

    fn instantiate_native(&mut self, native: Native, recv_args: &[Type], span: Span) -> Type {
        let sig = native.sig();
        let n = sig.max_param.max(recv_args.len() as u32);
        let inst: Vec<Type> = (0..n as usize)
            .map(|i| {
                recv_args
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| self.fresh(span, "type argument"))
            })
            .collect();
        let params: Vec<Type> = sig.params.iter().map(|p| substitute(p, &inst)).collect();
        let ret = substitute(&sig.ret, &inst);
        Type::Fn(params, Box::new(ret))
    }

    fn check_variant_ctor(
        &mut self,
        e: &Expr,
        def: DefId,
        enum_name: &str,
        variant: &Ident,
        args: &[Expr],
    ) -> Type {
        let TypeDef::Enum(ed) = self.defs.get(def) else { unreachable!() };
        let Some(vidx) = ed.variants.iter().position(|v| v.name == variant.name) else {
            let variants: Vec<&str> = ed.variants.iter().map(|v| v.name.as_str()).collect();
            self.diags.push(
                Diagnostic::error(
                    "E0414",
                    format!("no variant `{}` on enum `{enum_name}`", variant.name),
                )
                .with_label(variant.span, "")
                .with_note(format!("variants: {}", variants.join(", "))),
            );
            for a in args {
                self.check_expr(a, None);
            }
            return self.fresh(e.span, "unknown variant");
        };
        let ngen = ed.generics.len();
        let field_tys = ed.variants[vidx].fields.clone();
        self.res.insert(e.id, Res::Variant { def, variant: vidx as u32 });

        if field_tys.is_empty() {
            self.diags.push(
                Diagnostic::error(
                    "E0424",
                    format!(
                        "variant `{}` takes no arguments; write it without `(..)`",
                        variant.name
                    ),
                )
                .with_label(e.span, ""),
            );
        } else if args.len() != field_tys.len() {
            self.diags.push(
                Diagnostic::error(
                    "E0317",
                    format!(
                        "variant `{}` takes {} argument{}, found {}",
                        variant.name,
                        field_tys.len(),
                        if field_tys.len() == 1 { "" } else { "s" },
                        args.len()
                    ),
                )
                .with_label(e.span, ""),
            );
        }
        let inst: Vec<Type> =
            (0..ngen).map(|_| self.fresh(e.span, "type argument")).collect();
        for (a, f) in args.iter().zip(&field_tys) {
            let want = substitute(f, &inst);
            let at = self.check_expr(a, Some(&want));
            self.expect_type(&want, &at, a.span, None);
        }
        for a in args.iter().skip(field_tys.len()) {
            self.check_expr(a, None);
        }
        Type::Named(def, inst)
    }

    /// `alias.f(args)` where `alias` is an imported module: a direct call of
    /// a module function, or a call through a module global holding a
    /// function value.
    fn check_module_call(
        &mut self,
        e: &Expr,
        alias: &str,
        key: &str,
        method: &Ident,
        args: &[Expr],
    ) -> Type {
        let qname = format!("{key}.{}", method.name);
        if let Some(&idx) = self.fn_by_name.get(&qname) {
            if !self.fns[idx as usize].is_pub {
                self.private_item(
                    method.span,
                    "function",
                    &format!("{alias}.{}", method.name),
                    &method.name,
                );
            }
            self.res.insert(e.id, Res::ModuleFn(idx));
            let info = self.fns[idx as usize].clone();
            let inst: Vec<Type> = (0..info.generics.len())
                .map(|_| self.fresh(e.span, "type argument"))
                .collect();
            let params: Vec<Type> = info.params.iter().map(|p| substitute(p, &inst)).collect();
            let ret = substitute(&info.ret, &inst);
            if args.len() != params.len() {
                self.diags.push(
                    Diagnostic::error(
                        "E0317",
                        format!(
                            "`{alias}.{}` takes {} argument{}, found {}",
                            method.name,
                            params.len(),
                            if params.len() == 1 { "" } else { "s" },
                            args.len()
                        ),
                    )
                    .with_label(e.span, "")
                    .with_secondary(info.span, "function declared here"),
                );
                for a in args {
                    self.check_expr(a, None);
                }
                return ret;
            }
            for (a, p) in args.iter().zip(&params) {
                let at = self.check_expr(a, Some(p));
                self.expect_type(p, &at, a.span, None);
            }
            return ret;
        }
        if let Some(&slot) = self.global_by_name.get(&qname) {
            if !self.globals[slot as usize].is_pub {
                self.private_item(
                    method.span,
                    "binding",
                    &format!("{alias}.{}", method.name),
                    &method.name,
                );
            }
            self.res.insert(e.id, Res::Global(slot));
            let gty = self.globals[slot as usize].ty.clone();
            return self.check_callable_value(e, &gty, args, method.span);
        }
        self.diags.push(
            Diagnostic::error(
                "E0413",
                format!("no such member `{alias}.{}`", method.name),
            )
            .with_label(method.span, "not found in the imported module"),
        );
        for a in args {
            self.check_expr(a, None);
        }
        self.fresh(e.span, "unknown member")
    }

    /// Check a call of an arbitrary value of (hopefully) function type.
    fn check_callable_value(
        &mut self,
        e: &Expr,
        callee_ty: &Type,
        args: &[Expr],
        callee_span: Span,
    ) -> Type {
        match self.uni.shallow_resolve(callee_ty) {
            Type::Fn(params, ret) => {
                if args.len() != params.len() {
                    self.diags.push(
                        Diagnostic::error(
                            "E0317",
                            format!(
                                "this function takes {} argument{}, found {}",
                                params.len(),
                                if params.len() == 1 { "" } else { "s" },
                                args.len()
                            ),
                        )
                        .with_label(e.span, ""),
                    );
                    for a in args {
                        self.check_expr(a, None);
                    }
                    return *ret;
                }
                for (a, p) in args.iter().zip(&params) {
                    let at = self.check_expr(a, Some(p));
                    self.expect_type(p, &at, a.span, None);
                }
                *ret
            }
            Type::Var(_) => {
                self.cannot_infer_here(callee_span, "called value");
                for a in args {
                    self.check_expr(a, None);
                }
                self.fresh(e.span, "call result")
            }
            other => {
                self.diags.push(
                    Diagnostic::error(
                        "E0318",
                        format!("cannot call a value of type `{}`", self.show(&other)),
                    )
                    .with_label(callee_span, "not a function"),
                );
                for a in args {
                    self.check_expr(a, None);
                }
                self.fresh(e.span, "call result")
            }
        }
    }

    /// A call of a user-defined method: instantiate the registered fn's
    /// scheme and unify parameter 0 with the receiver.
    fn check_user_method_call(
        &mut self,
        e: &Expr,
        recv: &Expr,
        recv_ty: &Type,
        method: &Ident,
        args: &[Expr],
        idx: u32,
    ) -> Type {
        self.res.insert(e.id, Res::Fn(idx));
        let info = self.fns[idx as usize].clone();
        let inst: Vec<Type> = (0..info.generics.len())
            .map(|_| self.fresh(e.span, "type argument"))
            .collect();
        let params: Vec<Type> = info.params.iter().map(|p| substitute(p, &inst)).collect();
        let ret = substitute(&info.ret, &inst);
        self.expect_type(&params[0], recv_ty, recv.span, None);
        if args.len() != params.len() - 1 {
            self.diags.push(
                Diagnostic::error(
                    "E0317",
                    format!(
                        "`{}` takes {} argument{}, found {}",
                        method.name,
                        params.len() - 1,
                        if params.len() == 2 { "" } else { "s" },
                        args.len()
                    ),
                )
                .with_label(e.span, "")
                .with_secondary(info.span, "method declared here"),
            );
            for a in args {
                self.check_expr(a, None);
            }
            return ret;
        }
        for (a, p) in args.iter().zip(&params[1..]) {
            let at = self.check_expr(a, Some(p));
            self.expect_type(p, &at, a.span, None);
        }
        ret
    }

    fn check_call(
        &mut self,
        e: &Expr,
        callee: &Expr,
        args: &[Expr],
        _expected: Option<&Type>,
    ) -> Type {
        // Calls with special callees: declared fns, natives, prelude variants.
        if let ExprKind::Var(name) = &callee.kind {
            let is_local = self.scopes.iter().any(|s| s.contains_key(name.as_str()))
                || self.global_by_name.contains_key(&self.qualify(name));
            if !is_local {
                if let Some(idx) = self.lookup_fn(name) {
                    self.res.insert(callee.id, Res::Fn(idx));
                    let info = self.fns[idx as usize].clone();
                    let inst: Vec<Type> = (0..info.generics.len())
                        .map(|_| self.fresh(e.span, "type argument"))
                        .collect();
                    let params: Vec<Type> =
                        info.params.iter().map(|p| substitute(p, &inst)).collect();
                    let ret = substitute(&info.ret, &inst);
                    self.record(callee.id, Type::Fn(params.clone(), Box::new(ret.clone())));
                    if args.len() != params.len() {
                        self.diags.push(
                            Diagnostic::error(
                                "E0317",
                                format!(
                                    "`{name}` takes {} argument{}, found {}",
                                    params.len(),
                                    if params.len() == 1 { "" } else { "s" },
                                    args.len()
                                ),
                            )
                            .with_label(e.span, "")
                            .with_secondary(info.span, "function declared here"),
                        );
                        for a in args {
                            self.check_expr(a, None);
                        }
                        return ret;
                    }
                    for (a, p) in args.iter().zip(&params) {
                        let at = self.check_expr(a, Some(p));
                        self.expect_type(p, &at, a.span, None);
                    }
                    return ret;
                }
                if let Some(native) = Native::free_fn(name) {
                    self.res.insert(callee.id, Res::NativeFn(native));
                    return self.check_native_call(native, &[], args, e.span, callee.span);
                }
                if let Some((def, variant)) = prelude_variant(name) {
                    self.res.insert(callee.id, Res::Variant { def, variant });
                    let ename = self.defs.get(def).name().to_string();
                    // Route through the shared ctor checker (it re-records res
                    // on the Call node, which the compiler prefers).
                    let vident = Ident { name: name.clone(), span: callee.span };
                    return self.check_variant_ctor(e, def, &ename, &vident, args);
                }
            }
        }

        let ct = self.check_expr(callee, None);
        match self.uni.shallow_resolve(&ct) {
            Type::Fn(params, ret) => {
                if args.len() != params.len() {
                    self.diags.push(
                        Diagnostic::error(
                            "E0317",
                            format!(
                                "this function takes {} argument{}, found {}",
                                params.len(),
                                if params.len() == 1 { "" } else { "s" },
                                args.len()
                            ),
                        )
                        .with_label(e.span, ""),
                    );
                    for a in args {
                        self.check_expr(a, None);
                    }
                    return *ret;
                }
                for (a, p) in args.iter().zip(&params) {
                    let at = self.check_expr(a, Some(p));
                    self.expect_type(p, &at, a.span, None);
                }
                *ret
            }
            Type::Var(_) => {
                self.cannot_infer_here(callee.span, "called value");
                for a in args {
                    self.check_expr(a, None);
                }
                self.fresh(e.span, "call result")
            }
            other => {
                self.diags.push(
                    Diagnostic::error(
                        "E0318",
                        format!("cannot call a value of type `{}`", self.show(&other)),
                    )
                    .with_label(callee.span, "not a function"),
                );
                for a in args {
                    self.check_expr(a, None);
                }
                self.fresh(e.span, "call result")
            }
        }
    }

    /// `expr?` — unwrap `Some`/`Ok`, or return the `None`/`Err` from the
    /// enclosing function. The enclosing return type must be an `Option` (for
    /// an Option operand) or a `Result` with the same error type (for a Result
    /// operand); its success type is unconstrained by the `?` itself.
    fn check_try(&mut self, e: &Expr, inner: &Expr) -> Type {
        let t = self.check_expr(inner, None);
        let ret = match self.fn_stack.last() {
            Some(FnCtx::Fn { ret } | FnCtx::Lambda { ret }) => Some(ret.clone()),
            None => {
                self.diags.push(
                    Diagnostic::error("E0328", "`?` outside of a function")
                        .with_label(e.span, "`?` propagates failure by returning early")
                        .with_note(
                            "on `None`/`Err`, `?` returns from the enclosing function; \
                             there is none here",
                        ),
                );
                None
            }
        };
        match self.uni.shallow_resolve(&t) {
            Type::Named(d, args) if d == OPTION_DEF => {
                if let Some(ret) = ret {
                    let some = self.fresh(e.span, "try result");
                    let want = Type::Named(OPTION_DEF, vec![some]);
                    if self.uni.unify(&want, &ret).is_err() {
                        self.diags.push(
                            Diagnostic::error(
                                "E0329",
                                "`?` on an `Option` requires the enclosing function to \
                                 return `Option`",
                            )
                            .with_label(
                                e.span,
                                format!(
                                    "the `None` case would return `None`, but the function \
                                     returns `{}`",
                                    self.show(&ret)
                                ),
                            ),
                        );
                    }
                }
                args[0].clone()
            }
            Type::Named(d, args) if d == RESULT_DEF => {
                if let Some(ret) = ret {
                    let ok = self.fresh(e.span, "try result");
                    let want = Type::Named(RESULT_DEF, vec![ok, args[1].clone()]);
                    if self.uni.unify(&want, &ret).is_err() {
                        self.diags.push(
                            Diagnostic::error(
                                "E0329",
                                format!(
                                    "`?` on a `{}` requires the enclosing function to \
                                     return `Result` with error type `{}`",
                                    self.show(&Type::Named(RESULT_DEF, args.clone())),
                                    self.show(&args[1])
                                ),
                            )
                            .with_label(
                                e.span,
                                format!(
                                    "the `Err` case would return the error, but the \
                                     function returns `{}`",
                                    self.show(&ret)
                                ),
                            ),
                        );
                    }
                }
                args[0].clone()
            }
            Type::Var(_) => {
                self.cannot_infer_here(inner.span, "operand of `?`");
                self.fresh(e.span, "try result")
            }
            other => {
                self.diags.push(
                    Diagnostic::error(
                        "E0330",
                        format!(
                            "`?` requires an `Option` or `Result`, found `{}`",
                            self.show(&other)
                        ),
                    )
                    .with_label(inner.span, "only `Option` and `Result` can be unwrapped with `?`"),
                );
                self.fresh(e.span, "try result")
            }
        }
    }

    fn check_binary(
        &mut self,
        e: &Expr,
        op: BinOp,
        op_span: Span,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Type {
        use BinOp::*;
        match op {
            And | Or => {
                let lt = self.check_expr(lhs, Some(&Type::Bool));
                self.expect_type(&Type::Bool, &lt, lhs.span, None);
                let rt = self.check_expr(rhs, Some(&Type::Bool));
                self.expect_type(&Type::Bool, &rt, rhs.span, None);
                Type::Bool
            }
            Eq | Ne => {
                let lt = self.check_expr(lhs, None);
                let rt = self.check_expr(rhs, Some(&lt));
                if self.uni.unify(&lt, &rt).is_err() {
                    let (ls, rs) = (self.show(&lt), self.show(&rt));
                    self.diags.push(
                        Diagnostic::error(
                            "E0319",
                            format!("cannot compare `{ls}` with `{rs}`"),
                        )
                        .with_label(op_span, "operands must have the same type")
                        .with_secondary(lhs.span, format!("this is `{ls}`"))
                        .with_secondary(rhs.span, format!("this is `{rs}`")),
                    );
                }
                let z = self.uni.zonk(&lt);
                if z.contains_fn(&self.defs) {
                    self.diags.push(
                        Diagnostic::error("E0311", "cannot compare functions")
                            .with_label(op_span, ""),
                    );
                }
                Type::Bool
            }
            Lt | Le | Gt | Ge => {
                let lt = self.check_expr(lhs, None);
                let rt = self.check_expr(rhs, Some(&lt));
                if self.uni.unify(&lt, &rt).is_err() {
                    let (ls, rs) = (self.show(&lt), self.show(&rt));
                    self.diags.push(
                        Diagnostic::error(
                            "E0319",
                            format!("cannot compare `{ls}` with `{rs}`"),
                        )
                        .with_label(op_span, "operands must have the same type")
                        .with_secondary(lhs.span, format!("this is `{ls}`"))
                        .with_secondary(rhs.span, format!("this is `{rs}`")),
                    );
                }
                self.require_comparable(&lt, op_span, "order");
                Type::Bool
            }
            BitAnd | BitOr | BitXor | Shl | Shr => {
                // Bitwise (v0.7): Int only, no operator-method dispatch.
                let lt = self.check_expr(lhs, Some(&Type::Int));
                self.expect_type(&Type::Int, &lt, lhs.span, None);
                let rt = self.check_expr(rhs, Some(&Type::Int));
                self.expect_type(&Type::Int, &rt, rhs.span, None);
                Type::Int
            }
            Add | Sub | Mul | Div | Rem => {
                let lt = self.check_expr(lhs, None);
                // Operator methods (v0.3): a user-typed left operand
                // dispatches `a + b` to `a.add(b)` and so on. Dispatch is on
                // the LEFT type only; the right operand's type is whatever
                // the method declares (so `vec * 2.0` works).
                if let Type::Named(def, _) = self.uni.shallow_resolve(&lt) {
                    let mname = op_method_name(op);
                    if let Some(&idx) = self.methods.get(&(def, mname.to_string())) {
                        return self.check_operator_method(e, op, op_span, idx, lhs, rhs);
                    }
                }
                let rt = self.check_expr(rhs, Some(&lt));
                if self.uni.unify(&lt, &rt).is_err() {
                    let (ls, rs) = (self.show(&lt), self.show(&rt));
                    let mut d = Diagnostic::error(
                        "E0320",
                        format!("mismatched operand types `{ls}` and `{rs}`"),
                    )
                    .with_label(op_span, "")
                    .with_secondary(lhs.span, format!("this is `{ls}`"))
                    .with_secondary(rhs.span, format!("this is `{rs}`"));
                    if (ls == "Int" && rs == "Float") || (ls == "Float" && rs == "Int") {
                        d = d.with_note(
                            "Socrates has no implicit numeric conversion; use `.to_float()` or `.to_int()`",
                        );
                    }
                    self.diags.push(d);
                    return self.fresh(op_span, "arithmetic");
                }
                self.check_arith_operand(op, &lt, op_span);
                self.uni.shallow_resolve(&lt)
            }
        }
    }

    /// Foreign (cross-module) use of a method requires `pub`.
    fn check_method_visible(&mut self, def: DefId, idx: u32, mname: &str, span: Span) {
        let type_name = self.defs.get(def).name().to_string();
        if Self::name_module(&type_name) != self.module_prefix
            && !self.fns[idx as usize].is_pub
        {
            self.diags.push(
                Diagnostic::error(
                    "E0339",
                    format!("method `{mname}` on `{type_name}` is private"),
                )
                .with_label(span, "not exported by its module")
                .with_note(format!("add `pub` to `fn {mname}` in its impl block")),
            );
        }
    }

    /// `a + b` dispatching to a user type's `add` method (etc.). The left
    /// operand was checked by the caller; its type comes from the side table.
    fn check_operator_method(
        &mut self,
        e: &Expr,
        op: BinOp,
        op_span: Span,
        idx: u32,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Type {
        let mname = op_method_name(op);
        let info = self.fns[idx as usize].clone();
        let recv_ty = self.types.get(&lhs.id).cloned().expect("lhs was just checked");
        let recv_ty = &recv_ty;
        let def = match self.uni.shallow_resolve(recv_ty) {
            Type::Named(d, _) => d,
            _ => unreachable!("operator dispatch requires a Named receiver"),
        };
        self.check_method_visible(def, idx, mname, op_span);
        if info.params.len() != 2 {
            self.diags.push(
                Diagnostic::error(
                    "E0317",
                    format!(
                        "`{mname}` overloads `{}`, so it must take exactly one \
                         parameter besides `self`; it takes {}",
                        op.symbol(),
                        info.params.len() - 1
                    ),
                )
                .with_label(op_span, "")
                .with_secondary(info.span, "method declared here"),
            );
            self.check_expr(rhs, None);
            return self.fresh(op_span, "operator result");
        }
        self.res.insert(e.id, Res::Fn(idx));
        let inst: Vec<Type> = (0..info.generics.len())
            .map(|_| self.fresh(e.span, "type argument"))
            .collect();
        let params: Vec<Type> = info.params.iter().map(|p| substitute(p, &inst)).collect();
        let ret = substitute(&info.ret, &inst);
        self.expect_type(&params[0], recv_ty, lhs.span, None);
        let rt = self.check_expr(rhs, Some(&params[1]));
        self.expect_type(&params[1], &rt, rhs.span, None);
        ret
    }

    fn check_arith_operand(&mut self, op: BinOp, t: &Type, span: Span) {
        use BinOp::*;
        let r = self.uni.shallow_resolve(t);
        let ok = match op {
            Add => matches!(r, Type::Int | Type::Float | Type::Str),
            Sub | Mul | Div => matches!(r, Type::Int | Type::Float),
            // Bitwise compound assignment (v0.8) mirrors plain `&`/`|`/`^`/
            // `<<`/`>>`: Int-only, no operator-method dispatch.
            Rem | BitAnd | BitOr | BitXor | Shl | Shr => matches!(r, Type::Int),
            _ => true,
        };
        if matches!(r, Type::Var(_)) {
            self.cannot_infer_here(span, "operand");
            return;
        }
        if !ok {
            let allowed = match op {
                Add => "`Int`, `Float`, or `String`",
                Rem | BitAnd | BitOr | BitXor | Shl | Shr => "`Int`",
                _ => "`Int` or `Float`",
            };
            let mut d = Diagnostic::error(
                "E0321",
                format!(
                    "operator `{}` is not defined for `{}`",
                    op.symbol(),
                    self.show(&r)
                ),
            )
            .with_label(span, format!("expected {allowed}"));
            if let Type::Named(def, _) = &r {
                let mname = op_method_name(op);
                if self.methods.contains_key(&(*def, mname.to_string())) {
                    // Reachable only from compound assignment (plain binary
                    // expressions dispatch before this check runs).
                    d = d.with_note(format!(
                        "compound assignment does not dispatch to operator \
                         methods; write `x = x {} ...`",
                        op.symbol()
                    ));
                } else {
                    d = d.with_note(format!(
                        "define `fn {mname}(self, other)` in an impl block to \
                         overload `{}` for this type",
                        op.symbol()
                    ));
                }
            }
            self.diags.push(d);
        }
    }

    fn require_comparable(&mut self, t: &Type, span: Span, what: &str) {
        let r = self.uni.shallow_resolve(t);
        match r {
            Type::Int | Type::Float | Type::Str => {}
            Type::Var(_) => self.cannot_infer_here(span, "compared value"),
            Type::Param(i) => {
                let name = self
                    .generic_scope
                    .get(i as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("T{i}"));
                self.diags.push(
                    Diagnostic::error(
                        "E0322",
                        format!("cannot {what} values of generic type `{name}`"),
                    )
                    .with_label(span, "ordering needs `Int`, `Float`, or `String`")
                    .with_note("Socrates generics have no constraints; use a concrete type here"),
                );
            }
            other => {
                self.diags.push(
                    Diagnostic::error(
                        "E0322",
                        format!("cannot {what} values of type `{}`", self.show(&other)),
                    )
                    .with_label(span, "ordering needs `Int`, `Float`, or `String`"),
                );
            }
        }
    }

    fn check_struct_lit(&mut self, e: &Expr, name: &Ident, fields: &[(Ident, Expr)]) -> Type {
        let Some(def) = self.resolve_def_name(&name.name, name.span) else {
            let mut d = Diagnostic::error(
                "E0401",
                format!("unknown struct `{}`", name.name),
            )
            .with_label(name.span, "");
            if let Some(s) = self.suggest_type(&name.name) {
                d = d.with_note(format!("did you mean `{s}`?"));
            }
            self.diags.push(d);
            for (_, v) in fields {
                self.check_expr(v, None);
            }
            return self.fresh(e.span, "struct literal");
        };
        let TypeDef::Struct(sd) = self.defs.get(def) else {
            self.diags.push(
                Diagnostic::error(
                    "E0425",
                    format!("`{}` is an enum, not a struct", name.name),
                )
                .with_label(name.span, "")
                .with_note(format!("construct variants with `{}.Variant(..)`", name.name)),
            );
            for (_, v) in fields {
                self.check_expr(v, None);
            }
            return self.fresh(e.span, "struct literal");
        };
        let def_fields: Vec<(String, Type)> = sd.fields.clone();
        let ngen = sd.generics.len();
        let inst: Vec<Type> =
            (0..ngen).map(|_| self.fresh(e.span, "type argument")).collect();

        let mut field_order = Vec::new();
        let mut written = HashSet::new();
        for (fname, value) in fields {
            match def_fields.iter().position(|(n, _)| *n == fname.name) {
                Some(idx) => {
                    if !written.insert(idx) {
                        self.diags.push(
                            Diagnostic::error(
                                "E0426",
                                format!("field `{}` written twice", fname.name),
                            )
                            .with_label(fname.span, ""),
                        );
                    }
                    field_order.push(idx as u32);
                    let want = substitute(&def_fields[idx].1, &inst);
                    let vt = self.check_expr(value, Some(&want));
                    self.expect_type(&want, &vt, value.span, None);
                }
                None => {
                    field_order.push(u32::MAX);
                    let mut d = Diagnostic::error(
                        "E0415",
                        format!("no field `{}` on struct `{}`", fname.name, name.name),
                    )
                    .with_label(fname.span, "unknown field");
                    if let Some(s) =
                        closest(&fname.name, def_fields.iter().map(|(n, _)| n.as_str()))
                    {
                        d = d.with_note(format!("did you mean `{s}`?"));
                    }
                    self.diags.push(d);
                    self.check_expr(value, None);
                }
            }
        }
        let missing: Vec<&str> = def_fields
            .iter()
            .enumerate()
            .filter(|(i, _)| !written.contains(i))
            .map(|(_, (n, _))| n.as_str())
            .collect();
        if !missing.is_empty() {
            self.diags.push(
                Diagnostic::error(
                    "E0427",
                    format!(
                        "missing field{} {} in struct literal",
                        if missing.len() == 1 { "" } else { "s" },
                        missing
                            .iter()
                            .map(|m| format!("`{m}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
                .with_label(e.span, ""),
            );
        }
        self.res.insert(e.id, Res::StructLit { def, field_order });
        Type::Named(def, inst)
    }

    fn check_lambda(
        &mut self,
        e: &Expr,
        params: &[LambdaParam],
        ret: Option<&TypeExpr>,
        body: &Expr,
        expected: Option<&Type>,
    ) -> Type {
        // Pull parameter/return types from the expected function type if present.
        let expected_fn = expected.map(|t| self.uni.shallow_resolve(t)).and_then(|t| {
            if let Type::Fn(ps, r) = t {
                if ps.len() == params.len() {
                    return Some((ps, *r));
                }
            }
            None
        });

        if params.len() > 255 {
            self.diags.push(
                Diagnostic::error(
                    "E0325",
                    format!("lambda has {} parameters; the limit is 255", params.len()),
                )
                .with_label(e.span, ""),
            );
        }
        let mut param_tys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let ty = match &p.ty {
                Some(t) => {
                    let t = self.resolve_type_expr(t);
                    if let Some((eps, _)) = &expected_fn {
                        let _ = self.uni.unify(&t, &eps[i]);
                    }
                    t
                }
                None => match &expected_fn {
                    Some((eps, _)) => eps[i].clone(),
                    None => self.fresh_named(p.name.span, "lambda parameter"),
                },
            };
            param_tys.push(ty);
        }
        let ret_ty = match ret {
            Some(t) => self.resolve_type_expr(t),
            None => match &expected_fn {
                Some((_, r)) => r.clone(),
                None => self.fresh(e.span, "lambda return"),
            },
        };

        let saved_fn_locals = self.cur_fn_locals;
        self.cur_fn_locals = 0;
        self.scopes.push(HashMap::new());
        for (p, ty) in params.iter().zip(&param_tys) {
            let id = self.alloc_local(&p.name.name, false, ty.clone(), p.name.span);
            self.res.insert(p.id, Res::Local(id));
            self.types.insert(p.id, ty.clone());
        }
        self.fn_stack.push(FnCtx::Lambda { ret: ret_ty.clone() });
        self.loop_depth.push(0);
        let body_ty = self.check_expr(body, Some(&ret_ty));
        self.expect_type(&ret_ty, &body_ty, body.span, None);
        self.loop_depth.pop();
        self.fn_stack.pop();
        self.scopes.pop();
        self.cur_fn_locals = saved_fn_locals;

        Type::Fn(param_tys, Box::new(ret_ty))
    }

    fn check_match(
        &mut self,
        e: &Expr,
        scrutinee: &Expr,
        arms: &[MatchArm],
        sugar: MatchSugar,
        expected: Option<&Type>,
    ) -> Type {
        let st = self.check_expr(scrutinee, None);
        let result = self.fresh(e.span, "match result");
        if let Some(exp) = expected {
            let _ = self.uni.unify(&result, exp);
        }

        for arm in arms {
            self.scopes.push(HashMap::new());
            let mut binds = PatBinds::default();
            let mut seen = HashSet::new();
            self.check_pattern(&arm.pattern, &st, &mut binds, &mut seen);
            self.materialize_binds(binds, false);
            if let Some(guard) = &arm.guard {
                let gt = self.check_expr(guard, Some(&Type::Bool));
                self.expect_type(&Type::Bool, &gt, guard.span, None);
            }
            let bt = self.check_expr(&arm.body, Some(&result));
            if self.uni.unify(&result, &bt).is_err() {
                let (want, got) = (self.show(&result), self.show(&bt));
                self.diags.push(
                    Diagnostic::error(
                        "E0323",
                        "match arms have incompatible types",
                    )
                    .with_label(arm.body.span, format!("this arm has type `{got}`"))
                    .with_note(format!("earlier arms have type `{want}`")),
                );
            }
            self.scopes.pop();
        }

        // Exhaustiveness & reachability are DEFERRED to finalize(): the
        // scrutinee's type may only resolve after this match is checked
        // (e.g. a lambda parameter pinned by a later call). Patterns are
        // lowered now, while their resolutions are fresh.
        let mut lowered = Vec::with_capacity(arms.len());
        for arm in arms {
            let mut truncated = false;
            let rows = self.lower_pattern(&arm.pattern, &mut truncated);
            if truncated {
                self.diags.push(
                    Diagnostic::warning(
                        "W0103",
                        "pattern is too large for exhaustiveness analysis",
                    )
                    .with_label(
                        arm.pattern.span,
                        "treated as covering everything; missing cases panic at runtime",
                    ),
                );
            }
            lowered.push(DeferredArm {
                rows,
                guarded: arm.guard.is_some(),
                pattern_span: arm.pattern.span,
            });
        }
        self.deferred_matches.push(DeferredMatch {
            scrut_ty: st.clone(),
            scrut_span: scrutinee.span,
            arms: lowered,
            sugar,
        });
        result
    }

    /// Run the deferred per-match exhaustiveness/reachability analyses, now
    /// that inference has fixed every scrutinee type it is going to fix.
    fn analyze_matches(&mut self) {
        let deferred = std::mem::take(&mut self.deferred_matches);
        for dm in deferred {
            if !self.uni.is_fully_resolved(&dm.scrut_ty) {
                // A cannot-infer error is reported elsewhere.
                continue;
            }
            let scrut_ty = self.uni.zonk(&dm.scrut_ty);
            let mut matrix: Vec<Vec<DPat>> = Vec::new();
            for arm in &dm.arms {
                let reachable = arm.rows.iter().any(|row| {
                    patterns::usefulness(
                        &matrix,
                        std::slice::from_ref(row),
                        std::slice::from_ref(&scrut_ty),
                        &self.defs,
                    )
                    .is_some()
                });
                // `if let`/`while let` (v0.8) always append a synthetic
                // wildcard fallback arm; when the user's own pattern is
                // already irrefutable, that fallback is unreachable, but the
                // user never wrote it — don't warn about compiler-generated
                // code they can't see or edit.
                if !reachable && dm.sugar == MatchSugar::None {
                    self.diags.push(
                        Diagnostic::warning("W0101", "unreachable match arm")
                            .with_label(arm.pattern_span, "this pattern is covered by earlier arms"),
                    );
                }
                if !arm.guarded {
                    matrix.extend(arm.rows.iter().cloned().map(|p| vec![p]));
                }
            }
            if let Some(witness) = patterns::usefulness(
                &matrix,
                &[DPat::wild()],
                std::slice::from_ref(&scrut_ty),
                &self.defs,
            ) {
                let shown = patterns::display_pattern(&witness[0], &self.defs);
                self.diags.push(
                    Diagnostic::error(
                        "E0501",
                        format!("non-exhaustive match: the value `{shown}` is not covered"),
                    )
                    .with_label(dm.scrut_span, format!("`{shown}` is not covered"))
                    .with_note("add an arm for it, or a catch-all `_ ->` arm"),
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // Patterns
    // ------------------------------------------------------------------

    fn check_pattern(
        &mut self,
        pat: &Pattern,
        ty: &Type,
        binds: &mut PatBinds,
        seen: &mut HashSet<String>,
    ) {
        self.types.insert(pat.id, ty.clone());
        match &pat.kind {
            PatternKind::Wildcard => {}
            PatternKind::Binding(name) => {
                // Reinterpret as a nullary variant pattern when it names one.
                let resolved = self.uni.shallow_resolve(ty);
                let variant_hit = match &resolved {
                    Type::Named(def, _) => {
                        if let TypeDef::Enum(ed) = self.defs.get(*def) {
                            ed.variants
                                .iter()
                                .position(|v| v.name == *name)
                                .map(|i| (*def, i as u32, ed.variants[i].fields.len()))
                        } else {
                            None
                        }
                    }
                    Type::Var(_) => prelude_variant(name).map(|(d, v)| {
                        let TypeDef::Enum(ed) = self.defs.get(d) else { unreachable!() };
                        (d, v, ed.variants[v as usize].fields.len())
                    }),
                    _ => None,
                };
                if let Some((def, vidx, nfields)) = variant_hit {
                    if nfields > 0 {
                        self.diags.push(
                            Diagnostic::error(
                                "E0502",
                                format!(
                                    "variant `{name}` has {nfields} field{}; the pattern needs `({})`",
                                    if nfields == 1 { "" } else { "s" },
                                    vec!["_"; nfields].join(", ")
                                ),
                            )
                            .with_label(pat.span, ""),
                        );
                        return;
                    }
                    self.res.insert(pat.id, Res::Variant { def, variant: vidx });
                    let ngen = self.defs.get(def).generics().len();
                    let inst: Vec<Type> =
                        (0..ngen).map(|_| self.fresh(pat.span, "type argument")).collect();
                    self.unify_pattern_type(ty, &Type::Named(def, inst), pat.span);
                    return;
                }
                if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    self.diags.push(
                        Diagnostic::warning(
                            "W0102",
                            format!("pattern `{name}` binds a new variable"),
                        )
                        .with_label(pat.span, "this is not a variant of the matched type")
                        .with_note(
                            "uppercase names in patterns usually refer to enum variants; \
                             if a binding is intended, use a lowercase name",
                        ),
                    );
                }
                if seen.contains(name) {
                    self.diags.push(
                        Diagnostic::error(
                            "E0504",
                            format!("`{name}` is bound more than once in this pattern"),
                        )
                        .with_label(pat.span, ""),
                    );
                    return;
                }
                seen.insert(name.clone());
                binds.bind(name, ty.clone(), pat.span, pat.id, &mut self.uni, &mut self.diags);
            }
            PatternKind::Int(_) => self.unify_pattern_type(ty, &Type::Int, pat.span),
            PatternKind::Float(_) => self.unify_pattern_type(ty, &Type::Float, pat.span),
            PatternKind::Bool(_) => self.unify_pattern_type(ty, &Type::Bool, pat.span),
            PatternKind::Str(_) => self.unify_pattern_type(ty, &Type::Str, pat.span),
            PatternKind::Unit => self.unify_pattern_type(ty, &Type::Unit, pat.span),
            PatternKind::Tuple(items) => {
                let item_tys: Vec<Type> =
                    items.iter().map(|p| self.fresh(p.span, "tuple element")).collect();
                self.unify_pattern_type(ty, &Type::Tuple(item_tys.clone()), pat.span);
                for (p, t) in items.iter().zip(&item_tys) {
                    self.check_pattern(p, t, binds, seen);
                }
            }
            PatternKind::Variant { enum_name, variant, fields, has_parens } => {
                let def = match enum_name {
                    Some(en) => match self.resolve_def_name(&en.name, en.span) {
                        Some(d) if matches!(self.defs.get(d), TypeDef::Enum(_)) => d,
                        Some(_) => {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0425",
                                    format!("`{}` is a struct, not an enum", en.name),
                                )
                                .with_label(en.span, ""),
                            );
                            return;
                        }
                        None => {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0401",
                                    format!("unknown enum `{}`", en.name),
                                )
                                .with_label(en.span, ""),
                            );
                            return;
                        }
                    },
                    None => {
                        // Unqualified: from the scrutinee type, else prelude.
                        match self.uni.shallow_resolve(ty) {
                            Type::Named(d, _)
                                if matches!(self.defs.get(d), TypeDef::Enum(_)) =>
                            {
                                d
                            }
                            _ => match prelude_variant(&variant.name) {
                                Some((d, _)) => d,
                                None => {
                                    self.diags.push(
                                        Diagnostic::error(
                                            "E0400",
                                            format!(
                                                "unknown variant `{}`; qualify it as `Enum.{}`",
                                                variant.name, variant.name
                                            ),
                                        )
                                        .with_label(variant.span, ""),
                                    );
                                    return;
                                }
                            },
                        }
                    }
                };
                let TypeDef::Enum(ed) = self.defs.get(def) else { unreachable!() };
                let ename = ed.name.clone();
                let Some(vidx) = ed.variants.iter().position(|v| v.name == variant.name) else {
                    let variants: Vec<&str> =
                        ed.variants.iter().map(|v| v.name.as_str()).collect();
                    self.diags.push(
                        Diagnostic::error(
                            "E0414",
                            format!("no variant `{}` on enum `{ename}`", variant.name),
                        )
                        .with_label(variant.span, "")
                        .with_note(format!("variants: {}", variants.join(", "))),
                    );
                    return;
                };
                let vfields = ed.variants[vidx].fields.clone();
                let ngen = ed.generics.len();
                self.res.insert(pat.id, Res::Variant { def, variant: vidx as u32 });

                if !has_parens && !vfields.is_empty() {
                    self.diags.push(
                        Diagnostic::error(
                            "E0502",
                            format!(
                                "variant `{}` has {} field{}; the pattern needs `({})`",
                                variant.name,
                                vfields.len(),
                                if vfields.len() == 1 { "" } else { "s" },
                                vec!["_"; vfields.len()].join(", ")
                            ),
                        )
                        .with_label(pat.span, ""),
                    );
                    return;
                }
                if *has_parens && fields.len() != vfields.len() {
                    self.diags.push(
                        Diagnostic::error(
                            "E0505",
                            format!(
                                "variant `{}` has {} field{}, but the pattern has {}",
                                variant.name,
                                vfields.len(),
                                if vfields.len() == 1 { "" } else { "s" },
                                fields.len()
                            ),
                        )
                        .with_label(pat.span, ""),
                    );
                }
                let inst: Vec<Type> =
                    (0..ngen).map(|_| self.fresh(pat.span, "type argument")).collect();
                self.unify_pattern_type(ty, &Type::Named(def, inst.clone()), pat.span);
                for (p, f) in fields.iter().zip(&vfields) {
                    let want = substitute(f, &inst);
                    self.check_pattern(p, &want, binds, seen);
                }
                for p in fields.iter().skip(vfields.len()) {
                    let t = self.fresh(p.span, "extra pattern");
                    self.check_pattern(p, &t, binds, seen);
                }
            }
            PatternKind::Struct { name, fields, rest } => {
                let Some(def) = self.resolve_def_name(&name.name, name.span) else {
                    self.diags.push(
                        Diagnostic::error(
                            "E0401",
                            format!("unknown struct `{}`", name.name),
                        )
                        .with_label(name.span, ""),
                    );
                    return;
                };
                let TypeDef::Struct(sd) = self.defs.get(def) else {
                    self.diags.push(
                        Diagnostic::error(
                            "E0425",
                            format!("`{}` is an enum, not a struct", name.name),
                        )
                        .with_label(name.span, "")
                        .with_note(format!(
                            "match variants with `{}.Variant(..)` patterns",
                            name.name
                        )),
                    );
                    return;
                };
                let def_fields = sd.fields.clone();
                let ngen = sd.generics.len();
                let inst: Vec<Type> =
                    (0..ngen).map(|_| self.fresh(pat.span, "type argument")).collect();
                self.unify_pattern_type(ty, &Type::Named(def, inst.clone()), pat.span);

                let mut field_order = Vec::new();
                let mut written = HashSet::new();
                for (fname, fpat) in fields {
                    match def_fields.iter().position(|(n, _)| *n == fname.name) {
                        Some(idx) => {
                            if !written.insert(idx) {
                                self.diags.push(
                                    Diagnostic::error(
                                        "E0426",
                                        format!("field `{}` matched twice", fname.name),
                                    )
                                    .with_label(fname.span, ""),
                                );
                            }
                            field_order.push(idx as u32);
                            let want = substitute(&def_fields[idx].1, &inst);
                            self.check_pattern(fpat, &want, binds, seen);
                        }
                        None => {
                            field_order.push(u32::MAX);
                            self.diags.push(
                                Diagnostic::error(
                                    "E0415",
                                    format!(
                                        "no field `{}` on struct `{}`",
                                        fname.name, name.name
                                    ),
                                )
                                .with_label(fname.span, ""),
                            );
                            let t = self.fresh(fpat.span, "unknown field");
                            self.check_pattern(fpat, &t, binds, seen);
                        }
                    }
                }
                if !rest {
                    let missing: Vec<&str> = def_fields
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| !written.contains(i))
                        .map(|(_, (n, _))| n.as_str())
                        .collect();
                    if !missing.is_empty() {
                        self.diags.push(
                            Diagnostic::error(
                                "E0506",
                                format!(
                                    "struct pattern is missing field{} {}",
                                    if missing.len() == 1 { "" } else { "s" },
                                    missing
                                        .iter()
                                        .map(|m| format!("`{m}`"))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            )
                            .with_label(pat.span, "add them, or `..` to ignore the rest"),
                        );
                    }
                }
                self.res.insert(pat.id, Res::StructPat { def, field_order });
            }
            PatternKind::Or(alts) => {
                let names_before: HashSet<String> = binds.names().collect();
                let mut alt_names: Vec<HashSet<String>> = Vec::new();
                for alt in alts {
                    let mut alt_seen = seen.clone();
                    self.check_pattern(alt, ty, binds, &mut alt_seen);
                    let new: HashSet<String> = binds
                        .names()
                        .filter(|n| !names_before.contains(n))
                        .collect();
                    alt_names.push(new);
                }
                if let Some(first) = alt_names.first() {
                    for (i, names) in alt_names.iter().enumerate().skip(1) {
                        if names != first {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0507",
                                    "or-pattern alternatives bind different variables",
                                )
                                .with_label(alts[i].span, "this alternative differs")
                                .with_note(
                                    "every `|` alternative must bind exactly the same names",
                                ),
                            );
                            break;
                        }
                    }
                    for n in first {
                        seen.insert(n.clone());
                    }
                }
            }
        }
    }

    fn unify_pattern_type(&mut self, expected: &Type, found: &Type, span: Span) {
        if self.uni.unify(expected, found).is_err() {
            let (e, f) = (self.show(expected), self.show(found));
            self.diags.push(
                Diagnostic::error(
                    "E0508",
                    format!("pattern of type `{f}` cannot match a value of type `{e}`"),
                )
                .with_label(span, format!("expected `{e}`, found `{f}` pattern")),
            );
        }
    }

    /// Turn collected pattern bindings into locals (or globals at top level).
    fn materialize_binds(&mut self, binds: PatBinds, mutable: bool) {
        for group in binds.groups {
            if self.scopes.is_empty() {
                // Top level: a global slot (bytecode operands are u16-wide).
                if self.globals.len() >= 65_000 {
                    self.diags.push(
                        Diagnostic::error("E0327", "too many global bindings (limit 65,000)")
                            .with_label(group.span, ""),
                    );
                    continue;
                }
                let slot = self.globals.len() as u32;
                let stored = self.qualify(&group.name);
                self.globals.push(GlobalInfo {
                    is_pub: self.cur_let_is_pub,
                    name: stored.clone(),
                    mutable,
                    ty: group.ty.clone(),
                    span: group.span,
                });
                self.global_by_name.insert(stored, slot);
                for node in group.nodes {
                    self.res.insert(node, Res::Global(slot));
                }
            } else {
                let id = self.alloc_local(&group.name, mutable, group.ty.clone(), group.span);
                for node in group.nodes {
                    self.res.insert(node, Res::Local(id));
                }
            }
        }
    }

    fn assert_irrefutable(&mut self, pat: &Pattern, what: &str) {
        let refutable_span = self.find_refutable(pat);
        if let Some((span, why)) = refutable_span {
            self.diags.push(
                Diagnostic::error("E0503", format!("refutable pattern in {what}"))
                    .with_label(span, why)
                    .with_note("the pattern must always match here; use `match` instead"),
            );
        }
    }

    fn find_refutable(&self, pat: &Pattern) -> Option<(Span, String)> {
        match &pat.kind {
            PatternKind::Wildcard | PatternKind::Unit => None,
            PatternKind::Binding(_) => {
                if let Some(Res::Variant { def, .. }) = self.res.get(&pat.id) {
                    if let TypeDef::Enum(e) = self.defs.get(*def) {
                        if e.variants.len() > 1 {
                            return Some((pat.span, "this variant pattern can fail".into()));
                        }
                    }
                }
                None
            }
            PatternKind::Int(_)
            | PatternKind::Float(_)
            | PatternKind::Bool(_)
            | PatternKind::Str(_) => Some((pat.span, "a literal pattern can fail".into())),
            PatternKind::Tuple(items) => items.iter().find_map(|p| self.find_refutable(p)),
            PatternKind::Variant { .. } => {
                if let Some(Res::Variant { def, .. }) = self.res.get(&pat.id) {
                    if let TypeDef::Enum(e) = self.defs.get(*def) {
                        if e.variants.len() == 1 {
                            if let PatternKind::Variant { fields, .. } = &pat.kind {
                                return fields.iter().find_map(|p| self.find_refutable(p));
                            }
                        }
                    }
                }
                Some((pat.span, "this variant pattern can fail".into()))
            }
            PatternKind::Struct { fields, .. } => {
                fields.iter().find_map(|(_, p)| self.find_refutable(p))
            }
            PatternKind::Or(_) => Some((pat.span, "or-patterns can fail".into())),
        }
    }

    /// Lower a checked pattern into decision rows (or-patterns expanded).
    /// Sets `truncated` when the expansion exceeded the cap and fell back to
    /// a wildcard (over-approximating coverage — reported as W0103).
    fn lower_pattern(&self, pat: &Pattern, truncated: &mut bool) -> Vec<DPat> {
        const CAP: usize = 4096;
        match &pat.kind {
            PatternKind::Wildcard => vec![DPat::wild()],
            PatternKind::Binding(_) => match self.res.get(&pat.id) {
                Some(Res::Variant { def, variant }) => {
                    vec![DPat::ctor(Ctor::Variant(*def, *variant), Vec::new())]
                }
                _ => vec![DPat::wild()],
            },
            PatternKind::Int(i) => vec![DPat::ctor(Ctor::Int(*i), Vec::new())],
            PatternKind::Float(f) => {
                // Canonicalize -0.0 to +0.0 so analysis agrees with the
                // runtime's IEEE `==` (0.0 == -0.0).
                let f = if *f == 0.0 { 0.0 } else { *f };
                vec![DPat::ctor(Ctor::FloatBits(f.to_bits()), Vec::new())]
            }
            PatternKind::Bool(b) => vec![DPat::ctor(Ctor::Bool(*b), Vec::new())],
            PatternKind::Str(s) => vec![DPat::ctor(Ctor::Str(s.clone()), Vec::new())],
            PatternKind::Unit => vec![DPat::ctor(Ctor::Unit, Vec::new())],
            PatternKind::Tuple(items) => {
                let parts: Vec<Vec<DPat>> = items.iter().map(|p| self.lower_pattern(p, truncated)).collect();
                cartesian(&parts, CAP, truncated)
                    .into_iter()
                    .map(|args| DPat::ctor(Ctor::Tuple(items.len()), args))
                    .collect()
            }
            PatternKind::Variant { fields, .. } => match self.res.get(&pat.id) {
                Some(Res::Variant { def, variant }) => {
                    let parts: Vec<Vec<DPat>> =
                        fields.iter().map(|p| self.lower_pattern(p, truncated)).collect();
                    cartesian(&parts, CAP, truncated)
                        .into_iter()
                        .map(|args| DPat::ctor(Ctor::Variant(*def, *variant), args))
                        .collect()
                }
                _ => vec![DPat::wild()],
            },
            PatternKind::Struct { fields, .. } => match self.res.get(&pat.id) {
                Some(Res::StructPat { def, field_order }) => {
                    let nfields = match self.defs.get(*def) {
                        TypeDef::Struct(s) => s.fields.len(),
                        _ => 0,
                    };
                    // Normalize to all fields in definition order.
                    let mut slot_pats: Vec<Vec<DPat>> = vec![vec![DPat::wild()]; nfields];
                    for ((_, fpat), &idx) in fields.iter().zip(field_order) {
                        if (idx as usize) < nfields {
                            slot_pats[idx as usize] = self.lower_pattern(fpat, truncated);
                        }
                    }
                    cartesian(&slot_pats, CAP, truncated)
                        .into_iter()
                        .map(|args| DPat::ctor(Ctor::Struct(*def), args))
                        .collect()
                }
                _ => vec![DPat::wild()],
            },
            PatternKind::Or(alts) => {
                let mut out = Vec::new();
                for alt in alts {
                    out.extend(self.lower_pattern(alt, truncated));
                    if out.len() > CAP {
                        *truncated = true;
                        return vec![DPat::wild()];
                    }
                }
                out
            }
        }
    }

    // ------------------------------------------------------------------
    // Function bodies
    // ------------------------------------------------------------------

    fn check_fn_body(&mut self, f: &FnDecl) {
        let Some(Res::Fn(idx)) = self.res.get(&f.id).cloned() else { return };
        let info = self.fns[idx as usize].clone();
        self.generic_scope = info.generics.clone();

        self.cur_fn_locals = 0;
        self.scopes.push(HashMap::new());
        for (p, ty) in f.params.iter().zip(&info.params) {
            let id = self.alloc_local(&p.name.name, false, ty.clone(), p.name.span);
            self.res.insert(p.id, Res::Local(id));
            self.types.insert(p.id, ty.clone());
        }
        self.fn_stack.push(FnCtx::Fn { ret: info.ret.clone() });
        self.loop_depth.push(0);

        let body_ty = self.check_block(&f.body, Some(&info.ret));
        if self.uni.unify(&info.ret, &body_ty).is_err() {
            let (want, got) = (self.show(&info.ret), self.show(&body_ty));
            let label_span = f
                .body
                .stmts
                .last()
                .map(|s| s.span)
                .unwrap_or(f.body.span);
            let mut d = Diagnostic::error(
                "E0301",
                format!("function `{}` should return `{want}`, but its body has type `{got}`", info.name),
            )
            .with_label(label_span, format!("this has type `{got}`"));
            if let Some(rt) = &f.ret {
                d = d.with_secondary(rt.span, "return type declared here");
            } else if got != "Unit" {
                d = d.with_note("without a `-> Type` annotation, functions return `Unit`; did you forget the annotation, or a trailing `;`?");
            }
            self.diags.push(d);
        }

        self.loop_depth.pop();
        self.fn_stack.pop();
        self.scopes.pop();
        self.generic_scope.clear();
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn alloc_local(&mut self, name: &str, mutable: bool, ty: Type, span: Span) -> u32 {
        self.cur_fn_locals += 1;
        if self.cur_fn_locals == 60_001 {
            self.diags.push(
                Diagnostic::error(
                    "E0326",
                    "too many local variables in one function (limit 60,000)",
                )
                .with_label(span, ""),
            );
        }
        let id = self.locals.len() as u32;
        self.locals.push(LocalInfo { name: name.to_string(), mutable, ty, span });
        self.scopes
            .last_mut()
            .expect("alloc_local outside scope")
            .insert(name.to_string(), id);
        id
    }

    /// Bytecode aggregate-length operands are u16-wide; enforce a limit well
    /// under 65,535 with headroom for the compiler's virtual-depth tracking.
    fn check_literal_len(&mut self, len: usize, what: &str, span: Span) {
        if len > 60_000 {
            self.diags.push(
                Diagnostic::error(
                    "E0325",
                    format!("{what} has {len} elements; the limit is 60,000"),
                )
                .with_label(span, ""),
            );
        }
    }

    fn fresh(&mut self, _span: Span, _what: &'static str) -> Type {
        self.uni.fresh()
    }

    /// A fresh variable that reports "cannot infer" at `span` if never solved.
    fn fresh_named(&mut self, span: Span, what: &'static str) -> Type {
        let v = self.uni.fresh_id();
        self.var_origins.push(VarOrigin { var: v, span, what, default_unit: false });
        Type::Var(v)
    }

    /// A fresh variable that silently becomes Unit if never solved.
    fn fresh_defaulting(&mut self, span: Span, what: &'static str) -> Type {
        let v = self.uni.fresh_id();
        self.var_origins.push(VarOrigin { var: v, span, what, default_unit: true });
        Type::Var(v)
    }

    /// The methods defined on a type, as (name, is_pub) pairs (for tooling —
    /// the language server's completion).
    pub fn methods_on(&self, def: DefId) -> Vec<(String, bool)> {
        self.methods
            .iter()
            .filter(|((d, _), _)| *d == def)
            .map(|((_, name), &idx)| (name.clone(), self.fns[idx as usize].is_pub))
            .collect()
    }

    /// Zonked display of a type against this checker's defs (for tooling —
    /// the language server's hover).
    pub fn display_type_public(&self, t: &Type) -> String {
        display_type(&self.uni.zonk(t), &self.defs, &[])
    }

    fn show(&self, t: &Type) -> String {
        display_type(&self.uni.zonk(t), &self.defs, &self.generic_scope)
    }

    fn expect_type(&mut self, expected: &Type, found: &Type, span: Span, expected_from: Option<Span>) {
        if self.uni.unify(expected, found).is_ok() {
            return;
        }
        let (e, f) = (self.show(expected), self.show(found));
        let mut d = Diagnostic::error("E0301", "type mismatch")
            .with_label(span, format!("expected `{e}`, found `{f}`"));
        if let Some(from) = expected_from {
            d = d.with_secondary(from, "expected due to this");
        }
        if (e == "Int" && f == "Float") || (e == "Float" && f == "Int") {
            d = d.with_note(
                "Socrates has no implicit numeric conversion; use `.to_float()` or `.to_int()`",
            );
        }
        self.diags.push(d);
    }

    fn cannot_infer_here(&mut self, span: Span, what: &str) {
        self.diags.push(
            Diagnostic::error(
                "E0302",
                format!("the type of this {what} must be known at this point"),
            )
            .with_label(span, "add a type annotation")
            .with_note("Socrates infers left to right; an annotation upstream fixes this"),
        );
    }

    /// Report leftover unsolved inference variables ("cannot infer"), and apply
    /// Unit defaulting where marked.
    fn finalize(&mut self) {
        self.analyze_matches();
        let origins = std::mem::take(&mut self.var_origins);
        for origin in origins {
            let t = self.uni.shallow_resolve(&Type::Var(origin.var));
            if let Type::Var(rep) = t {
                if origin.default_unit {
                    let _ = self.uni.unify(&Type::Var(rep), &Type::Unit);
                    continue;
                }
                if self.reported_vars.insert(rep) {
                    self.diags.push(
                        Diagnostic::error(
                            "E0302",
                            format!("cannot infer the type of this {}", origin.what),
                        )
                        .with_label(origin.span, "add a type annotation")
                        .with_note("Socrates's inference is local; annotate this or its context"),
                    );
                }
            }
        }
        // Additionally: globals must end up fully typed.
        for i in 0..self.globals.len() {
            let ty = self.globals[i].ty.clone();
            if !self.uni.is_fully_resolved(&ty) {
                let zonked = self.uni.zonk(&ty);
                if let Some(v) = first_var(&zonked) {
                    if self.reported_vars.insert(v) {
                        let g = &self.globals[i];
                        self.diags.push(
                            Diagnostic::error(
                                "E0302",
                                format!("cannot infer the type of `{}`", g.name),
                            )
                            .with_label(g.span, "add a type annotation")
                            .with_note(format!("the type so far is `{}`", self.show(&ty))),
                        );
                    }
                }
            }
        }
        // Zonk all recorded types so downstream consumers see concrete types.
        let keys: Vec<NodeId> = self.types.keys().copied().collect();
        for k in keys {
            let t = self.types[&k].clone();
            let z = self.uni.zonk(&t);
            self.types.insert(k, z);
        }
        for i in 0..self.locals.len() {
            let z = self.uni.zonk(&self.locals[i].ty);
            self.locals[i].ty = z;
        }
        for i in 0..self.globals.len() {
            let z = self.uni.zonk(&self.globals[i].ty);
            self.globals[i].ty = z;
        }
    }

    fn suggest_type(&self, name: &str) -> Option<String> {
        let builtin = [
            "Int", "Float", "Bool", "String", "Unit", "List", "Map", "Range", "Bytes", "Worker",
            "Window",
        ];
        let candidates = builtin
            .iter()
            .copied()
            .chain(self.defs.types.iter().map(|d| d.name()))
            .chain(self.generic_scope.iter().map(|s| s.as_str()));
        closest(name, candidates)
    }

    fn suggest_value(&self, name: &str) -> Option<String> {
        let mut cands: Vec<String> = Vec::new();
        for s in &self.scopes {
            cands.extend(s.keys().cloned());
        }
        cands.extend(self.global_by_name.keys().cloned());
        cands.extend(self.fn_by_name.keys().cloned());
        cands.extend(
            ["print", "println", "str", "panic", "assert", "assert_eq", "clock", "input", "math"]
                .iter()
                .map(|s| s.to_string()),
        );
        cands.extend(["Some", "None", "Ok", "Err"].iter().map(|s| s.to_string()));
        closest(name, cands.iter().map(|s| s.as_str()))
    }
}

// ---------------------------------------------------------------------------
// Pattern binding collection
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct DeferredArm {
    rows: Vec<DPat>,
    guarded: bool,
    pattern_span: Span,
}

#[derive(Clone)]
struct DeferredMatch {
    scrut_ty: Type,
    scrut_span: Span,
    arms: Vec<DeferredArm>,
    sugar: MatchSugar,
}

/// Does this block contain a `break` anywhere in its subtree?
///
/// Used to decide whether a trailing `while true { .. }` diverges. This
/// deliberately over-approximates — it descends into nested loops and
/// lambdas, where a `break` could never target the outer loop — because a
/// false "contains break" merely falls back to the old non-diverging
/// typing, while a false "no break" would type an escapable loop as
/// diverging (unsound).
fn block_contains_break(block: &Block) -> bool {
    block.stmts.iter().any(stmt_contains_break)
}

fn stmt_contains_break(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Break => true,
        StmtKind::Continue | StmtKind::Import { .. } => false,
        // Item declarations own their bodies; a `break` inside a nested `fn`
        // can never target this loop, and the checker rejects it there anyway.
        StmtKind::Fn(_) | StmtKind::Struct(_) | StmtKind::Enum(_) | StmtKind::Impl(_) => false,
        StmtKind::Let { init, .. } => expr_contains_break(init),
        StmtKind::Assign { target, value, .. } => {
            expr_contains_break(target) || expr_contains_break(value)
        }
        StmtKind::Expr { expr, .. } => expr_contains_break(expr),
        StmtKind::While { cond, body } => expr_contains_break(cond) || block_contains_break(body),
        StmtKind::For { iter, body, .. } => {
            expr_contains_break(iter) || block_contains_break(body)
        }
        StmtKind::Return(v) => v.as_ref().is_some_and(expr_contains_break),
    }
}

fn expr_contains_break(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Unit
        | ExprKind::Var(_) => false,
        ExprKind::StringInterp { exprs, .. } => exprs.iter().any(expr_contains_break),
        ExprKind::Field { base, .. } => expr_contains_break(base),
        ExprKind::Call { callee, args } => {
            expr_contains_break(callee) || args.iter().any(expr_contains_break)
        }
        ExprKind::MethodCall { recv, args, .. } => {
            expr_contains_break(recv) || args.iter().any(expr_contains_break)
        }
        ExprKind::Unary { expr, .. } | ExprKind::Try(expr) => expr_contains_break(expr),
        ExprKind::Binary { lhs, rhs, .. } => {
            expr_contains_break(lhs) || expr_contains_break(rhs)
        }
        ExprKind::Index { base, index } => {
            expr_contains_break(base) || expr_contains_break(index)
        }
        ExprKind::List(items) | ExprKind::Tuple(items) => items.iter().any(expr_contains_break),
        ExprKind::MapLit(entries) => entries
            .iter()
            .any(|(k, v)| expr_contains_break(k) || expr_contains_break(v)),
        ExprKind::Range { lo, hi, .. } => expr_contains_break(lo) || expr_contains_break(hi),
        ExprKind::StructLit { fields, .. } => {
            fields.iter().any(|(_, v)| expr_contains_break(v))
        }
        // Over-approximation: `break` in a lambda is an error anyway.
        ExprKind::Lambda { body, .. } => expr_contains_break(body),
        ExprKind::If { cond, then, els } => {
            expr_contains_break(cond)
                || block_contains_break(then)
                || els.as_deref().is_some_and(expr_contains_break)
        }
        ExprKind::Block(b) => block_contains_break(b),
        ExprKind::Match { scrutinee, arms, .. } => {
            expr_contains_break(scrutinee)
                || arms.iter().any(|a| {
                    a.guard.as_ref().is_some_and(expr_contains_break)
                        || expr_contains_break(&a.body)
                })
        }
    }
}

#[derive(Default)]
struct PatBinds {
    groups: Vec<BindGroup>,
}

struct BindGroup {
    name: String,
    ty: Type,
    span: Span,
    nodes: Vec<NodeId>,
}

impl PatBinds {
    fn names(&self) -> impl Iterator<Item = String> + '_ {
        self.groups.iter().map(|g| g.name.clone())
    }

    fn bind(
        &mut self,
        name: &str,
        ty: Type,
        span: Span,
        node: NodeId,
        uni: &mut Unifier,
        diags: &mut Vec<Diagnostic>,
    ) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.name == name) {
            // Re-binding from another or-pattern alternative: same variable.
            if uni.unify(&g.ty, &ty).is_err() {
                diags.push(
                    Diagnostic::error(
                        "E0509",
                        format!("`{name}` has different types in or-pattern alternatives"),
                    )
                    .with_label(span, "bound here with a different type")
                    .with_secondary(g.span, "first bound here"),
                );
            }
            g.nodes.push(node);
        } else {
            self.groups.push(BindGroup {
                name: name.to_string(),
                ty,
                span,
                nodes: vec![node],
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Small utilities
// ---------------------------------------------------------------------------

fn prelude_variant(name: &str) -> Option<(DefId, u32)> {
    match name {
        "Some" => Some((OPTION_DEF, 0)),
        "None" => Some((OPTION_DEF, 1)),
        "Ok" => Some((RESULT_DEF, 0)),
        "Err" => Some((RESULT_DEF, 1)),
        _ => None,
    }
}

/// The well-known method name an arithmetic operator dispatches to.
fn op_method_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::Div => "div",
        BinOp::Rem => "rem",
        _ => unreachable!("only arithmetic operators dispatch to methods"),
    }
}

fn collect_pattern_names(pat: &Pattern, out: &mut HashSet<String>) {
    match &pat.kind {
        PatternKind::Binding(n) => {
            out.insert(n.clone());
        }
        PatternKind::Tuple(items) => {
            for p in items {
                collect_pattern_names(p, out);
            }
        }
        PatternKind::Struct { fields, .. } => {
            for (_, p) in fields {
                collect_pattern_names(p, out);
            }
        }
        PatternKind::Variant { fields, .. } => {
            for p in fields {
                collect_pattern_names(p, out);
            }
        }
        PatternKind::Or(alts) => {
            for p in alts {
                collect_pattern_names(p, out);
            }
        }
        _ => {}
    }
}

fn cartesian(parts: &[Vec<DPat>], cap: usize, truncated: &mut bool) -> Vec<Vec<DPat>> {
    let mut rows: Vec<Vec<DPat>> = vec![Vec::new()];
    for part in parts {
        let mut next = Vec::new();
        for row in &rows {
            for p in part {
                let mut r = row.clone();
                r.push(p.clone());
                next.push(r);
                if next.len() > cap {
                    // Give up on precision: a single all-wildcards row
                    // (reported to the user as W0103 by the caller).
                    *truncated = true;
                    return vec![vec![DPat::wild(); parts.len()]];
                }
            }
        }
        rows = next;
    }
    rows
}

fn first_var(t: &Type) -> Option<u32> {
    match t {
        Type::Var(v) => Some(*v),
        Type::List(e) => first_var(e),
        Type::Map(k, v) => first_var(k).or_else(|| first_var(v)),
        Type::Tuple(ts) => ts.iter().find_map(first_var),
        Type::Fn(ps, r) => ps.iter().find_map(first_var).or_else(|| first_var(r)),
        Type::Named(_, ts) => ts.iter().find_map(first_var),
        _ => None,
    }
}

/// Closest candidate by edit distance, for "did you mean" notes.
fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        if c == name {
            continue;
        }
        let d = edit_distance(name, c);
        let max_ok = match name.len() {
            0..=3 => 1,
            4..=6 => 2,
            _ => 3,
        };
        if d <= max_ok && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c.to_string())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> (Checker, Vec<Diagnostic>) {
        let lexed = lex(src);
        assert!(lexed.diags.is_empty(), "lex errors: {:?}", lexed.diags);
        let parsed = parse(lexed.tokens, src);
        assert!(parsed.diags.is_empty(), "parse errors: {:?}", parsed.diags);
        let mut checker = Checker::new();
        checker.check_program(&parsed.program);
        let diags = checker.take_diags();
        (checker, diags)
    }

    fn ok(src: &str) {
        let (_, diags) = check_src(src);
        let errors: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        assert!(errors.is_empty(), "unexpected errors in {src:?}: {errors:#?}");
    }

    fn err_code(src: &str, code: &str) {
        let (_, diags) = check_src(src);
        assert!(
            diags.iter().any(|d| d.code == code && d.is_error()),
            "expected error {code} in {src:?}, got: {:?}",
            diags.iter().map(|d| (d.code, d.message.clone())).collect::<Vec<_>>()
        );
    }

    #[test]
    fn basics() {
        ok("let x = 1 + 2; let y = x * 3; println(y);");
        ok("let s = \"a\" + \"b\"; println(s.len());");
        err_code("let x = 1 + 2.0;", "E0320");
        err_code("let x: Int = \"hi\";", "E0301");
        err_code("println(nope);", "E0400");
    }

    #[test]
    fn functions() {
        ok("fn add(a: Int, b: Int) -> Int { a + b } println(add(1, 2));");
        ok("fn fib(n: Int) -> Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } } println(fib(10));");
        err_code("fn f(a: Int) -> Int { a } f(1, 2);", "E0317");
        err_code("fn f() -> Int { }", "E0301");
        err_code("fn f() -> Int { return \"no\"; }", "E0301");
    }

    #[test]
    fn generics() {
        ok(r#"
            fn first[T](xs: List[T]) -> Option[T] { xs.first() }
            let a = first([1, 2, 3]);
            let b = first(["x"]);
            println(a.unwrap() + 1);
            println(b.unwrap() + "!");
        "#);
        ok("fn id[T](x: T) -> T { x } let a: Int = id(5); let b: String = id(\"s\");");
        err_code("fn id[T](x: T) -> T { x } let a: Int = id(\"s\");", "E0301");
        err_code("fn bad[T](a: T, b: T) -> T { a } bad(1, \"s\");", "E0301");
    }

    #[test]
    fn lambdas_and_methods() {
        ok("let xs = [1, 2, 3]; let ys = xs.map(|n| n * 2); println(ys[0] + 1);");
        ok("let total = [1, 2, 3].fold(0, |acc, n| acc + n); println(total);");
        ok("let names = [\"a\", \"bb\"].filter(|s| s.len() > 1);");
        ok("let f = |x| x + 1; println(f(1));"); // inferable: `+ 1` pins x to Int
        err_code("let f = |x| x;", "E0302"); // genuinely ambiguous
        ok("let f = |x: Int| x + 1; println(f(1));");
        err_code("[1,2].map(|s| s.chars());", "E0422"); // Int has no chars()
    }

    #[test]
    fn structs() {
        ok(r#"
            struct Point { x: Float, y: Float }
            let p = Point { x: 1.0, y: 2.0 };
            p.x = p.x + 1.0;
            println(p.x);
        "#);
        err_code("struct P { x: Int } let p = P { };", "E0427");
        err_code("struct P { x: Int } let p = P { x: 1, y: 2 };", "E0415");
        err_code("struct P { x: Int } let p = P { x: 1 }; p.z;", "E0415");
        ok(r#"
            struct Pair[A, B] { first: A, second: B }
            let p = Pair { first: 1, second: "one" };
            let n: Int = p.first;
            let s: String = p.second;
        "#);
    }

    #[test]
    fn enums_and_match() {
        ok(r#"
            enum Shape { Circle(Float), Rect(Float, Float), Empty }
            fn area(s: Shape) -> Float {
                match s {
                    Shape.Circle(r) -> 3.14 * r * r,
                    Shape.Rect(w, h) -> w * h,
                    Shape.Empty -> 0.0,
                }
            }
            println(area(Shape.Circle(2.0)));
        "#);
        err_code(
            r#"
            enum Shape { Circle(Float), Empty }
            fn f(s: Shape) -> Int { match s { Shape.Circle(r) -> 1 } }
            "#,
            "E0501",
        );
        ok("fn f(o: Option[Int]) -> Int { match o { Some(v) -> v, None -> 0 } }");
        err_code("fn f(o: Option[Int]) -> Int { match o { Some(v) -> v } }", "E0501");
        err_code(
            "fn f(x: Int) -> Int { match x { 1 -> 1, 2 -> 2 } }",
            "E0501",
        );
        ok("fn f(x: Int) -> Int { match x { 1 -> 1, _ -> 0 } }");
        ok("fn f(b: Bool) -> Int { match b { true -> 1, false -> 0 } }");
    }

    #[test]
    fn or_patterns_and_guards() {
        ok("fn f(x: Int) -> String { match x { 0 | 1 -> \"low\", n if n < 100 -> \"mid\", _ -> \"hi\" } }");
        err_code(
            "fn f(t: (Int, Int)) -> Int { match t { (x, 0) | (0, y) -> 0, _ -> 1 } }",
            "E0507",
        );
        // Guarded arms don't count toward exhaustiveness.
        err_code("fn f(b: Bool) -> Int { match b { true -> 1, false if b -> 0 } }", "E0501");
    }

    #[test]
    fn mutability() {
        ok("let mut x = 1; x = 2; x += 3;");
        err_code("let x = 1; x = 2;", "E0307");
        err_code("let x = 1; x += 1;", "E0307");
        ok("struct P { x: Int } let p = P { x: 1 }; p.x = 2;"); // fields always mutable
        err_code("let t = (1, 2); t.0 = 5;", "E0309");
    }

    #[test]
    fn maps_and_lists() {
        ok(r#"
            let m = {"a": 1, "b": 2};
            let v: Option[Int] = m.get("a");
            m["c"] = 3;
            println(m.len());
        "#);
        ok("let m: Map[String, Int] = {:}; println(m.is_empty());");
        err_code("let m = {\"a\": 1, 2: 3};", "E0301");
        ok("let xs: List[Int] = []; println(xs.len());");
        err_code("let xs = [];", "E0302"); // cannot infer
        err_code("let xs = [1, \"two\"];", "E0301");
    }

    #[test]
    fn loops() {
        ok("for i in 0..10 { println(i); }");
        ok("let xs = [1, 2]; for x in xs { println(x); }");
        ok("for c in \"hi\" { println(c); }");
        err_code("for x in 5 { println(x); }", "E0303");
        err_code("break;", "E0305");
        ok("let mut i = 0; while i < 3 { i += 1; }");
        err_code("while 1 { }", "E0301");
    }

    #[test]
    fn strings_and_interp() {
        ok(r#"let name = "world"; println("hello {name}, {1 + 2}");"#);
        err_code(r#"let s = "x"; s[0];"#, "E0313");
    }

    #[test]
    fn options_results() {
        ok(r#"
            fn safe_div(a: Int, b: Int) -> Result[Int, String] {
                if b == 0 { Err("division by zero") } else { Ok(a / b) }
            }
            match safe_div(10, 2) {
                Ok(v) -> println(v),
                Err(e) -> println(e),
            }
        "#);
        ok("let x = Some(5); println(x.unwrap_or(0));");
        err_code("let x = None;", "E0302");
        ok("let x: Option[Int] = None; println(x.is_none());");
    }

    #[test]
    fn variant_paths() {
        ok(r#"
            enum Color { Red, Green, Blue }
            let c = Color.Red;
            match c { Color.Red -> println(1), Color.Green -> println(2), Color.Blue -> println(3) }
        "#);
        // Unqualified variant patterns work when the enum is known from the type.
        ok(r#"
            enum Color { Red, Green, Blue }
            fn f(c: Color) -> Int { match c { Red -> 1, Green -> 2, Blue -> 3 } }
        "#);
        err_code("enum E { A(Int) } let x = E.A;", "E0409");
        err_code("enum E { A } let x = E.B;", "E0414");
    }

    #[test]
    fn destructuring() {
        ok("let (a, b) = (1, \"two\"); println(a); println(b);");
        ok("struct P { x: Int, y: Int } let P { x, y } = P { x: 1, y: 2 }; println(x + y);");
        err_code("let (a, b) = (1, 2, 3);", "E0508");
        err_code("let Some(x) = Some(1);", "E0503"); // refutable
    }

    #[test]
    fn top_level_ordering() {
        ok("fn f() -> Int { g() } fn g() -> Int { 42 } println(f());"); // mutual visibility
        err_code("println(x); let x = 1;", "E0412"); // used before declaration
        ok("fn f() -> Int { later } let later = 5; println(f());"); // fns see all globals
    }

    #[test]
    fn math_namespace() {
        ok("println(math.sin(2.0)); println(math.pi); println(math.pow(2.0, 3.0));");
        err_code("println(math.nope(1.0));", "E0413");
        err_code("let m = math;", "E0410");
    }

    #[test]
    fn comparisons() {
        ok("println(1 < 2); println(\"a\" < \"b\"); println(1.5 >= 0.5);");
        err_code("println((1, 2) < (3, 4));", "E0322");
        err_code("fn f[T](a: T, b: T) -> Bool { a < b }", "E0322");
        ok("fn f[T](a: T, b: T) -> Bool { a == b }"); // equality is generic
        err_code("let f = |x: Int| x; let g = |x: Int| x; f == g;", "E0311");
    }

    #[test]
    fn unreachable_arm_warning() {
        let (_, diags) = check_src("fn f(x: Int) -> Int { match x { _ -> 0, 1 -> 1 } }");
        assert!(diags.iter().any(|d| d.code == "W0101"));
    }

    #[test]
    fn nested_generic_inference() {
        ok(r#"
            let pairs = [(1, "one"), (2, "two")];
            let names = pairs.map(|p| p.1);
            let joined = names.join(", ");
            println(joined);
        "#);
        ok(r#"
            let grid = [[1, 2], [3, 4]];
            let flat = grid.flat_map(|row| row.map(|v| v * 10));
            println(flat.len());
        "#);
    }

    #[test]
    fn returns_in_lambdas() {
        ok("let f = |x: Int| -> Int { if x > 0 { return x; } 0 }; println(f(5));");
        err_code("return 5;", "E0304");
    }
}
