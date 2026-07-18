//! The abstract syntax tree.
//!
//! Every expression, pattern, and type node carries a `Span` and a `NodeId`.
//! The type checker keys its side tables (types, resolutions) on `NodeId`; the
//! bytecode compiler consumes the AST together with those tables.

use crate::span::{NodeId, Span};

#[derive(Debug, Clone)]
pub struct Program {
    /// Top-level items and statements in source order.
    pub stmts: Vec<Stmt>,
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FnDecl {
    /// `pub fn ...` — visible to importing modules (v0.3). Meaningless (and
    /// harmless) in the root module.
    pub is_pub: bool,
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub fields: Vec<FieldDef>,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: Ident,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub variants: Vec<VariantDef>,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub struct VariantDef {
    pub name: Ident,
    pub fields: Vec<TypeExpr>,
    pub span: Span,
}

/// `impl TypeName[T, ...] { fn method(self, ...) { ... } ... }`
///
/// The bracketed names re-bind the type's declared generics for the whole
/// block (arity must match the declaration). Each method's first parameter is
/// `self`; the parser synthesizes its `TypeExpr` (`TypeName[T, ...]`) so that
/// methods flow through the ordinary function machinery.
#[derive(Debug, Clone)]
pub struct ImplDecl {
    pub ty_name: Ident,
    pub generics: Vec<Ident>,
    pub methods: Vec<FnDecl>,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Impl(ImplDecl),
    /// `import a.b;` / `import a.b as c;` — loads `a/b.soc` relative to the
    /// importing file. The module is referenced by its alias (default: the
    /// last path segment).
    Import {
        path: Vec<Ident>,
        alias: Option<Ident>,
    },
    /// `[pub] let [mut] pattern [: ty] = expr;`
    Let {
        is_pub: bool,
        mutable: bool,
        pattern: Pattern,
        ty: Option<TypeExpr>,
        init: Expr,
    },
    /// `lhs = rhs;` and compound forms. `op` is `None` for plain `=`.
    Assign {
        target: Expr,
        op: Option<BinOp>,
        value: Expr,
    },
    /// An expression used as a statement. `tail` marks the final expression of a
    /// block with no trailing `;` (it becomes the block's value).
    Expr {
        expr: Expr,
        tail: bool,
    },
    While {
        cond: Expr,
        body: Block,
    },
    /// `for pattern in iter { .. }` — any irrefutable pattern (a name, `_`,
    /// or nested tuple/struct patterns), enforced by the checker like `let`.
    For {
        pattern: Pattern,
        iter: Expr,
        body: Block,
    },
    Return(Option<Expr>),
    Break,
    Continue,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
    pub id: NodeId,
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    /// `"a {x} b"` — literal parts interleaved with expressions.
    /// `parts.len() == exprs.len() + 1`.
    StringInterp {
        parts: Vec<String>,
        exprs: Vec<Expr>,
    },
    /// A name reference: variable, function, or namespace head (`math`).
    Var(String),
    /// `expr.field` — struct field access, tuple index (`.0`), namespace member
    /// (`math.pi`), or enum variant path (`Shape.Circle`); disambiguated by the
    /// type checker.
    Field {
        base: Box<Expr>,
        field: Ident,
    },
    /// `f(a, b)`
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `expr.method(a, b)` — builtin method or variant construction like
    /// `Shape.Circle(1.0)`; disambiguated by the type checker.
    MethodCall {
        recv: Box<Expr>,
        method: Ident,
        args: Vec<Expr>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    /// `expr?` — unwrap `Some`/`Ok` or return the `None`/`Err` from the
    /// enclosing function.
    Try(Box<Expr>),
    Binary {
        op: BinOp,
        op_span: Span,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `a[i]`
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// `[a, b, c]`
    List(Vec<Expr>),
    /// `{"a": 1}` / `{:}`
    MapLit(Vec<(Expr, Expr)>),
    /// `(a, b)` — two or more elements.
    Tuple(Vec<Expr>),
    /// `a..b` / `a..=b`
    Range {
        lo: Box<Expr>,
        hi: Box<Expr>,
        inclusive: bool,
    },
    /// `Point { x: 1.0, y: 2.0 }`
    StructLit {
        name: Ident,
        fields: Vec<(Ident, Expr)>,
    },
    /// `|a, b| expr` or `|a: T| -> R { ... }`
    Lambda {
        params: Vec<LambdaParam>,
        ret: Option<TypeExpr>,
        body: Box<Expr>,
    },
    If {
        cond: Box<Expr>,
        then: Block,
        /// `else` branch: either another `If` expression or a block.
        els: Option<Box<Expr>>,
    },
    /// A block used as an expression.
    Block(Block),
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
        /// How this was spelled (v0.8): `if let`/`while let` desugar fully
        /// to `Match` at parse time (a two-arm match with a synthetic
        /// wildcard fallback), so the checker and compiler need no special
        /// cases; this only tells the formatter how to print it back and
        /// tells the checker to silence "unreachable arm" on the synthetic
        /// fallback (an irrefutable user pattern makes it unreachable, but
        /// the user never wrote it).
        sugar: MatchSugar,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchSugar {
    None,
    IfLet,
    WhileLet,
}

#[derive(Debug, Clone)]
pub struct LambdaParam {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
    /// The body was written as a bare `return`/`break`/`continue` and
    /// desugared to a one-statement block; the formatter prints it back
    /// in the sugar form.
    pub sugar: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    /// Bitwise (v0.7), Int-only.
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub enum PatternKind {
    Wildcard,
    /// Binds the matched value to a name. Note: a lone `UpperCamel` identifier
    /// that names a nullary enum variant (e.g. `None`) is parsed as `Binding`
    /// and reinterpreted as a variant pattern by the type checker.
    Binding(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    Tuple(Vec<Pattern>),
    /// `Some(x)`, `Shape.Circle(r)`, `None`, `Option.None`.
    /// `enum_name` is `None` for the unqualified form.
    Variant {
        enum_name: Option<Ident>,
        variant: Ident,
        fields: Vec<Pattern>,
        /// True when written with parentheses (`Some(x)`), false for bare (`None`).
        has_parens: bool,
    },
    /// `Point { x, y: 0.0, .. }`
    Struct {
        name: Ident,
        fields: Vec<(Ident, Pattern)>,
        rest: bool,
    },
    /// `p1 | p2 | p3`
    Or(Vec<Pattern>),
}

// ---------------------------------------------------------------------------
// Type expressions (syntax-level types)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TypeExpr {
    pub kind: TypeExprKind,
    pub span: Span,
    pub id: NodeId,
}

#[derive(Debug, Clone)]
pub enum TypeExprKind {
    /// `Int`, `Point`, `T`, `Option[Int]`, `List[T]`, ...
    Named {
        name: Ident,
        args: Vec<TypeExpr>,
    },
    /// `(T1, T2, ...)` — 2+ elements.
    Tuple(Vec<TypeExpr>),
    /// `fn(T1, T2) -> R`
    Fn {
        params: Vec<TypeExpr>,
        ret: Option<Box<TypeExpr>>,
    },
    /// `()` — the unit type.
    Unit,
}
