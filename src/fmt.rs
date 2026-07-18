//! The canonical source formatter (`socrates fmt`).
//!
//! Prints the AST back out with normalized indentation (4 spaces), spacing,
//! and trailing commas. Literals (numbers, strings) are copied verbatim from
//! the source so radix, digit separators, and escapes survive. Comments are
//! preserved: they re-attach before the next statement at the right indent,
//! and a comment on the same line as a statement stays a trailing comment.
//! Blank-line runs between statements collapse to a single blank line.
//!
//! # Line width
//!
//! The formatter is width-aware (default [`DEFAULT_WIDTH`] columns) using a
//! hand-rolled measure-then-emit scheme in the spirit of Wadler/Prettier
//! groups. Every breakable construct is a *group*: [`Formatter::group`]
//! renders the group's one-line ("flat") layout directly into the output
//! buffer, keeps it when it fits in the remaining width, and otherwise rolls
//! the buffer back so the caller can emit the construct's broken (multi-line)
//! layout instead. Broken layouts format their children in auto mode again,
//! so breaking composes: the outermost group breaks first and inner groups
//! stay flat when they fit.
//!
//! Broken layouts:
//! - calls, lists, maps, tuples, struct literals: one element per line with a
//!   trailing comma and a hanging indent;
//! - a call whose *last* argument can never be one line (a block lambda, a
//!   `match`, ...) keeps the old "hugged" layout `f(a, |x| {` ... `})`;
//! - method/field chains: broken before each `.` element after the first —
//!   but only for pure width overflow; chains containing hard multi-line
//!   elements keep the attached `}).next(...)` layout;
//! - binary operators: the same-precedence spine breaks before each operator;
//! - `|x| expr` lambdas break to a block body;
//! - `if`/`else`(-`if` chains) inline only when the whole chain fits; a chain
//!   that breaks, breaks every branch (never `} else if c { 1 } else { 0 }`);
//! - `match` arms break in place.
//!
//! Flat rendering never consumes comments (constructs that would flush them
//! report "cannot flatten" instead), so a rolled-back attempt has no side
//! effects. A bracketed literal or argument list with *interior* comments
//! never flattens at all: it keeps the one-element-per-line layout, own-line
//! comments staying before their element and trailing comments on their
//! element's line — so a comment doubles as the author's escape hatch for
//! meaning-bearing multi-line layout. Layout decisions otherwise depend only
//! on the AST and the current column — never on source line numbers — which
//! keeps formatting idempotent.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::source::Source;
use crate::span::Span;
use crate::token::Comment;

/// The default maximum line width (`socrates fmt --width N` overrides it).
pub const DEFAULT_WIDTH: usize = 100;

pub fn format_source(name: &str, text: &str) -> Result<String, Vec<Diagnostic>> {
    format_source_width(name, text, DEFAULT_WIDTH)
}

pub fn format_source_width(
    name: &str,
    text: &str,
    width: usize,
) -> Result<String, Vec<Diagnostic>> {
    let lexed = crate::lexer::lex(text);
    let parsed = crate::parser::parse(lexed.tokens, text);
    let mut diags = lexed.diags;
    diags.extend(parsed.diags);
    if crate::diag::has_errors(&diags) {
        return Err(diags);
    }
    let source = Source::new(name, text);
    let mut f = Formatter {
        src: source,
        out: String::new(),
        indent: 0,
        comments: lexed.comments,
        next_comment: 0,
        last_line: 0,
        width,
        flat_cap: usize::MAX,
    };
    f.program(&parsed.program);
    // Trailing comments after the last statement.
    f.flush_comments(u32::MAX);
    let mut out = f.out;
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

struct Formatter {
    src: Source,
    out: String,
    indent: usize,
    comments: Vec<Comment>,
    next_comment: usize,
    /// Original source line of the last emitted item (for blank-line logic).
    last_line: u32,
    /// Maximum line width in columns.
    width: usize,
    /// Byte-length cap on `out` during a flat render; lets deeply nested flat
    /// attempts abort early once they cannot possibly fit (`chars >= bytes/4`,
    /// so exceeding `4 * budget` bytes proves the char budget is blown).
    flat_cap: usize,
}

/// One `.name` / `.name(args)` link of a postfix chain, plus the postfix
/// `?` / `[index]` operators applied to its result (in print order).
struct ChainLink<'a> {
    name: &'a Ident,
    args: Option<&'a [Expr]>,
    /// End of the call's own span (just past the `)`), bounding its argument
    /// region for interior-comment placement.
    close: u32,
    suffixes: Vec<ChainSuffix<'a>>,
}

enum ChainSuffix<'a> {
    Try,
    Index(&'a Expr),
}

/// Split a postfix expression into its base, the postfix operators applied
/// directly to the base, and the `.`-chain links in source order.
fn collect_chain(mut e: &Expr) -> (&Expr, Vec<ChainSuffix<'_>>, Vec<ChainLink<'_>>) {
    let mut links = Vec::new(); // gathered root-first, reversed below
    let mut pending: Vec<ChainSuffix> = Vec::new();
    loop {
        match &e.kind {
            ExprKind::MethodCall { recv, method, args } => {
                pending.reverse();
                links.push(ChainLink {
                    name: method,
                    args: Some(args),
                    close: e.span.end,
                    suffixes: std::mem::take(&mut pending),
                });
                e = recv;
            }
            ExprKind::Field { base, field } => {
                pending.reverse();
                links.push(ChainLink {
                    name: field,
                    args: None,
                    close: e.span.end,
                    suffixes: std::mem::take(&mut pending),
                });
                e = base;
            }
            ExprKind::Try(inner) => {
                pending.push(ChainSuffix::Try);
                e = inner;
            }
            ExprKind::Index { base, index } => {
                pending.push(ChainSuffix::Index(index));
                e = base;
            }
            _ => break,
        }
    }
    pending.reverse();
    links.reverse();
    (e, pending, links)
}

impl Formatter {
    fn snippet(&self, span: Span) -> &str {
        self.src.snippet(span)
    }

    fn line_of(&self, offset: u32) -> u32 {
        self.src.line_col(offset).line
    }

    fn pad(&mut self) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
    }

    /// Current column: characters on the (unfinished) last output line.
    fn col(&self) -> usize {
        let start = self.out.rfind('\n').map_or(0, |i| i + 1);
        self.out[start..].chars().count()
    }

    /// Render a group's flat layout with `f` and keep it when it fits on the
    /// current line with `rider` more characters to follow; otherwise roll the
    /// buffer back and return false so the caller emits the broken layout.
    /// `f` must be side-effect-free apart from writing to `out` (flat
    /// renderers never flush comments or touch the indent).
    fn group<F: FnOnce(&mut Self) -> bool>(&mut self, rider: usize, f: F) -> bool {
        let mark = self.out.len();
        let budget = self.width.saturating_sub(self.col() + rider);
        let saved_cap = self.flat_cap;
        self.flat_cap = mark + budget.saturating_mul(4) + 16;
        let ok = f(self);
        self.flat_cap = saved_cap;
        if ok {
            let text = &self.out[mark..];
            if !text.contains('\n') && text.chars().count() <= budget {
                return true;
            }
        }
        self.out.truncate(mark);
        false
    }

    /// Whether `e` *can* be rendered on one line at all (no block bodies,
    /// `match`es, or comments forcing multiple lines) — ignoring width.
    fn can_flatten(&mut self, e: &Expr) -> bool {
        let mark = self.out.len();
        let saved_cap = self.flat_cap;
        self.flat_cap = usize::MAX;
        let ok = self.flat_expr(e, 0);
        self.flat_cap = saved_cap;
        self.out.truncate(mark);
        ok
    }

    /// Width in characters of a fragment rendered by `f` (rolled back).
    fn measure<F: FnOnce(&mut Self)>(&mut self, f: F) -> usize {
        let mark = self.out.len();
        f(self);
        let w = self.out[mark..].chars().count();
        self.out.truncate(mark);
        w
    }

    fn blank_line_if_gap(&mut self, start: u32) {
        let line = self.line_of(start);
        if self.last_line != 0 && line > self.last_line + 1 {
            self.out.push('\n');
        }
    }

    /// Emit comments that start before `upto` (byte offset), each on its own
    /// line (or attached to the previous line if it started there).
    fn flush_comments(&mut self, upto: u32) {
        while self.next_comment < self.comments.len()
            && self.comments[self.next_comment].span.start < upto
        {
            let c = self.comments[self.next_comment].clone();
            self.next_comment += 1;
            let cline = self.line_of(c.span.start);
            if cline == self.last_line && self.out.ends_with('\n') {
                // Trailing comment: attach to the previous line.
                self.out.pop();
                self.out.push(' ');
                self.out.push_str(c.text.trim_end());
                self.out.push('\n');
            } else {
                self.blank_line_if_gap(c.span.start);
                self.pad();
                self.out.push_str(c.text.trim_end());
                self.out.push('\n');
            }
            self.last_line = self.line_of(c.span.end.saturating_sub(1));
        }
    }

    fn program(&mut self, p: &Program) {
        for stmt in &p.stmts {
            self.stmt(stmt);
        }
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    fn stmt(&mut self, s: &Stmt) {
        self.flush_comments(s.span.start);
        self.blank_line_if_gap(s.span.start);
        self.pad();
        match &s.kind {
            StmtKind::Fn(f) => self.fn_decl(f, false),
            StmtKind::Import { path, alias } => {
                self.out.push_str("import ");
                for (i, seg) in path.iter().enumerate() {
                    if i > 0 {
                        self.out.push('.');
                    }
                    self.out.push_str(&seg.name);
                }
                if let Some(a) = alias {
                    self.out.push_str(" as ");
                    self.out.push_str(&a.name);
                }
                self.out.push_str(";\n");
            }
            StmtKind::Impl(d) => self.impl_decl(d),
            StmtKind::Struct(d) => self.struct_decl(d),
            StmtKind::Enum(d) => self.enum_decl(d),
            StmtKind::Let { is_pub, mutable, pattern, ty, init } => {
                if *is_pub {
                    self.out.push_str("pub ");
                }
                self.out.push_str("let ");
                if *mutable {
                    self.out.push_str("mut ");
                }
                self.pattern(pattern);
                if let Some(t) = ty {
                    self.out.push_str(": ");
                    self.type_expr(t);
                }
                self.out.push_str(" = ");
                self.expr(init, 0, 1);
                self.out.push_str(";\n");
            }
            StmtKind::Assign { target, op, value } => {
                self.expr(target, 13, 0);
                let sym = match op {
                    None => "=",
                    Some(BinOp::Add) => "+=",
                    Some(BinOp::Sub) => "-=",
                    Some(BinOp::Mul) => "*=",
                    Some(BinOp::Div) => "/=",
                    Some(BinOp::Rem) => "%=",
                    Some(BinOp::BitAnd) => "&=",
                    Some(BinOp::BitOr) => "|=",
                    Some(BinOp::BitXor) => "^=",
                    Some(BinOp::Shl) => "<<=",
                    Some(BinOp::Shr) => ">>=",
                    Some(_) => "=",
                };
                self.out.push(' ');
                self.out.push_str(sym);
                self.out.push(' ');
                self.expr(value, 0, 1);
                self.out.push_str(";\n");
            }
            StmtKind::Expr { expr, tail } => {
                let block_like = matches!(
                    expr.kind,
                    ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Block(_)
                );
                let semi = !*tail && !block_like;
                self.expr(expr, 0, usize::from(semi));
                if semi {
                    self.out.push(';');
                }
                self.out.push('\n');
            }
            StmtKind::While { cond, body } => {
                if let Some((pattern, scrutinee, user_body)) = while_let_sugar(cond, body) {
                    self.out.push_str("while let ");
                    self.pattern(pattern);
                    self.out.push_str(" = ");
                    self.expr(scrutinee, 0, 2);
                    self.out.push(' ');
                    self.block(user_body);
                } else {
                    self.out.push_str("while ");
                    self.expr(cond, 0, 2);
                    self.out.push(' ');
                    self.block(body);
                }
                self.out.push('\n');
            }
            StmtKind::For { pattern, iter, body } => {
                self.out.push_str("for ");
                self.pattern(pattern);
                self.out.push_str(" in ");
                self.expr(iter, 0, 2);
                self.out.push(' ');
                self.block(body);
                self.out.push('\n');
            }
            StmtKind::Return(v) => {
                self.out.push_str("return");
                if let Some(v) = v {
                    self.out.push(' ');
                    self.expr(v, 0, 1);
                }
                self.out.push_str(";\n");
            }
            StmtKind::Break => self.out.push_str("break;\n"),
            StmtKind::Continue => self.out.push_str("continue;\n"),
        }
        self.last_line = self.line_of(s.span.end.saturating_sub(1));
    }

    /// `method` marks an impl-block method: its first parameter prints as a
    /// bare `self` (the parser synthesized its type annotation).
    fn fn_decl(&mut self, f: &FnDecl, method: bool) {
        if f.is_pub {
            self.out.push_str("pub ");
        }
        self.out.push_str("fn ");
        self.out.push_str(&f.name.name);
        self.generics(&f.generics);
        // The parameter list is a group; its rider is everything that still
        // follows on the closing line: ` -> Ret` and the ` {` of the body.
        let ret_width = match &f.ret {
            Some(r) => self.measure(|s| {
                s.out.push_str(" -> ");
                s.type_expr(r);
            }),
            None => 0,
        };
        if f.params.is_empty() {
            self.out.push_str("()");
        } else if !self.group(ret_width + 2, |s| s.flat_params(&f.params, method)) {
            self.out.push_str("(\n");
            self.indent += 1;
            for (i, p) in f.params.iter().enumerate() {
                self.pad();
                self.out.push_str(&p.name.name);
                if !(method && i == 0) {
                    self.out.push_str(": ");
                    self.type_expr(&p.ty);
                }
                self.out.push_str(",\n");
            }
            self.indent -= 1;
            self.pad();
            self.out.push(')');
        }
        if let Some(r) = &f.ret {
            self.out.push_str(" -> ");
            self.type_expr(r);
        }
        self.out.push(' ');
        self.block(&f.body);
        self.out.push('\n');
    }

    fn flat_params(&mut self, params: &[Param], method: bool) -> bool {
        self.out.push('(');
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.out.push_str(&p.name.name);
            if method && i == 0 {
                continue;
            }
            self.out.push_str(": ");
            self.type_expr(&p.ty);
        }
        self.out.push(')');
        true
    }

    fn impl_decl(&mut self, d: &ImplDecl) {
        self.out.push_str("impl ");
        self.out.push_str(&d.ty_name.name);
        self.generics(&d.generics);
        if d.methods.is_empty() {
            self.out.push_str(" {}\n");
            return;
        }
        self.out.push_str(" {\n");
        self.indent += 1;
        self.last_line = self.line_of(d.ty_name.span.end);
        for m in &d.methods {
            self.flush_comments(m.span.start);
            self.blank_line_if_gap(m.span.start);
            self.pad();
            self.fn_decl(m, true);
            self.last_line = self.line_of(m.span.end.saturating_sub(1));
        }
        self.indent -= 1;
        self.flush_comments(d.span.end);
        self.pad();
        self.out.push_str("}\n");
    }

    fn struct_decl(&mut self, d: &StructDecl) {
        if d.is_pub {
            self.out.push_str("pub ");
        }
        self.out.push_str("struct ");
        self.out.push_str(&d.name.name);
        self.generics(&d.generics);
        if d.fields.is_empty() {
            self.out.push_str(" {}\n");
            return;
        }
        self.out.push_str(" {\n");
        self.indent += 1;
        self.last_line = self.line_of(d.name.span.end);
        for f in &d.fields {
            self.flush_comments(f.name.span.start);
            self.pad();
            self.out.push_str(&f.name.name);
            self.out.push_str(": ");
            self.type_expr(&f.ty);
            self.out.push_str(",\n");
            self.last_line = self.line_of(f.ty.span.end.saturating_sub(1));
        }
        self.indent -= 1;
        self.flush_comments(d.span.end);
        self.pad();
        self.out.push_str("}\n");
    }

    fn enum_decl(&mut self, d: &EnumDecl) {
        if d.is_pub {
            self.out.push_str("pub ");
        }
        self.out.push_str("enum ");
        self.out.push_str(&d.name.name);
        self.generics(&d.generics);
        if d.variants.is_empty() {
            self.out.push_str(" {}\n");
            return;
        }
        self.out.push_str(" {\n");
        self.indent += 1;
        self.last_line = self.line_of(d.name.span.end);
        for v in &d.variants {
            self.flush_comments(v.span.start);
            self.pad();
            self.out.push_str(&v.name.name);
            if !v.fields.is_empty() {
                self.out.push('(');
                for (i, t) in v.fields.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.type_expr(t);
                }
                self.out.push(')');
            }
            self.out.push_str(",\n");
            self.last_line = self.line_of(v.span.end.saturating_sub(1));
        }
        self.indent -= 1;
        self.flush_comments(d.span.end);
        self.pad();
        self.out.push_str("}\n");
    }

    fn generics(&mut self, gs: &[Ident]) {
        if gs.is_empty() {
            return;
        }
        self.out.push('[');
        for (i, g) in gs.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.out.push_str(&g.name);
        }
        self.out.push(']');
    }

    // ------------------------------------------------------------------
    // Blocks
    // ------------------------------------------------------------------

    fn block(&mut self, b: &Block) {
        if b.stmts.is_empty() {
            // Preserve any comments inside the braces.
            if self.comments_before(b.span.end) {
                self.out.push_str("{\n");
                self.indent += 1;
                self.flush_comments(b.span.end);
                self.indent -= 1;
                self.pad();
                self.out.push('}');
            } else {
                self.out.push_str("{}");
            }
            return;
        }
        self.out.push_str("{\n");
        self.indent += 1;
        // The first inner statement compares its gap against the block
        // opening, not against whatever preceded the enclosing statement.
        self.last_line = self.line_of(b.span.start);
        for s in &b.stmts {
            self.stmt(s);
        }
        self.flush_comments(b.span.end);
        self.indent -= 1;
        self.pad();
        self.out.push('}');
    }

    fn comments_before(&self, upto: u32) -> bool {
        self.next_comment < self.comments.len()
            && self.comments[self.next_comment].span.start < upto
    }

    /// Whether any not-yet-emitted line-ending comment starts in
    /// `[start, end)` — i.e. the region (a bracketed literal's interior, an
    /// argument list) has comments the flat layout would evict. Such a
    /// construct never flattens: it keeps the one-element-per-line layout
    /// with each comment attached where the author put it, which also makes
    /// interior comments the escape hatch for meaning-bearing multi-line
    /// layout. Only comments that end their source line count: those are the
    /// positions (own-line, end-of-line) the broken layout can reproduce. A
    /// mid-line `/* .. */` with code after it on the same line cannot be
    /// pinned and keeps the old evict-after-statement behavior.
    fn comments_within(&self, start: u32, end: u32) -> bool {
        self.comments[self.next_comment..]
            .iter()
            .take_while(|c| c.span.start < end)
            .any(|c| c.span.start >= start && self.ends_its_line(c.span.end))
    }

    /// Whether only whitespace follows `offset` on its source line.
    fn ends_its_line(&self, offset: u32) -> bool {
        self.src.text[offset as usize..]
            .chars()
            .take_while(|&ch| ch != '\n')
            .all(char::is_whitespace)
    }

    /// A block that is short enough to inline as `{ expr }` in expressions.
    fn inline_block<'b>(&self, b: &'b Block) -> Option<&'b Expr> {
        if b.stmts.len() != 1 || self.comments_before(b.span.end) {
            return None;
        }
        match &b.stmts[0].kind {
            StmtKind::Expr { expr, tail: true } => Some(expr),
            _ => None,
        }
    }

    // ------------------------------------------------------------------
    // Expressions — auto mode (flat when it fits, broken otherwise)
    // ------------------------------------------------------------------

    /// Format an expression: keep it on one line when it fits within the
    /// width (with `rider` more characters following on the same line), break
    /// it otherwise. `prec` is the minimum precedence this context binds at;
    /// the expression is wrapped in parens if its own precedence is lower.
    fn expr(&mut self, e: &Expr, prec: u8, rider: usize) {
        if self.group(rider, |s| s.flat_expr(e, prec)) {
            return;
        }
        let parens = expr_prec(e) < prec;
        if parens {
            self.out.push('(');
        }
        self.expr_broken(e, rider + usize::from(parens));
        if parens {
            self.out.push(')');
        }
    }

    // ------------------------------------------------------------------
    // Expressions — flat renderers
    //
    // These mirror the broken renderers exactly for everything that fits on
    // one line. They return false when the expression can never be a single
    // line (block bodies, `match`, comments) and must not flush comments or
    // touch the indent, so a failed attempt rolls back cleanly.
    // ------------------------------------------------------------------

    fn flat_expr(&mut self, e: &Expr, prec: u8) -> bool {
        if self.out.len() > self.flat_cap {
            return false; // cannot fit anyway; abort early
        }
        let parens = expr_prec(e) < prec;
        if parens {
            self.out.push('(');
        }
        if !self.flat_expr_inner(e) {
            return false;
        }
        if parens {
            self.out.push(')');
        }
        true
    }

    fn flat_expr_inner(&mut self, e: &Expr) -> bool {
        match &e.kind {
            // Literals verbatim from the source.
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::StringInterp { .. } => {
                let text = self.snippet(e.span).to_string();
                self.out.push_str(&text);
                true
            }
            ExprKind::Bool(b) => {
                self.out.push_str(if *b { "true" } else { "false" });
                true
            }
            ExprKind::Unit => {
                self.out.push_str("()");
                true
            }
            ExprKind::Var(n) => {
                self.out.push_str(n);
                true
            }
            ExprKind::Field { base, field } => {
                if !self.flat_expr(base, 13) {
                    return false;
                }
                self.out.push('.');
                self.out.push_str(&field.name);
                true
            }
            ExprKind::Call { callee, args } => {
                if !args.is_empty() && self.comments_within(callee.span.end, e.span.end) {
                    return false; // interior comments pin the broken layout
                }
                // `(s.field)(args)` must keep its parens: without them the
                // source re-parses as a method call `s.field(args)`.
                if matches!(callee.kind, ExprKind::Field { .. }) {
                    self.out.push('(');
                    if !self.flat_expr_inner(callee) {
                        return false;
                    }
                    self.out.push(')');
                } else if !self.flat_expr(callee, 13) {
                    return false;
                }
                self.flat_args(args)
            }
            ExprKind::MethodCall { recv, method, args } => {
                if !args.is_empty() && self.comments_within(method.span.end, e.span.end) {
                    return false; // interior comments pin the broken layout
                }
                if !self.flat_expr(recv, 13) {
                    return false;
                }
                self.out.push('.');
                self.out.push_str(&method.name);
                self.flat_args(args)
            }
            ExprKind::Unary { op, expr } => {
                self.out.push_str(match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                });
                self.flat_expr(expr, 12)
            }
            ExprKind::Try(inner) => {
                if !self.flat_expr(inner, 13) {
                    return false;
                }
                self.out.push('?');
                true
            }
            ExprKind::Binary { op, lhs, rhs, .. } => {
                let p = bin_prec(*op);
                // Comparison/equality operators are NON-associative in the
                // grammar: `(a == b) == c` must keep its parens or it will
                // re-parse as a rejected chain.
                let lhs_prec = if non_assoc(*op) { p + 1 } else { p };
                if !self.flat_expr(lhs, lhs_prec) {
                    return false;
                }
                self.out.push(' ');
                self.out.push_str(op.symbol());
                self.out.push(' ');
                // Right operand binds one level tighter in both cases.
                self.flat_expr(rhs, p + 1)
            }
            ExprKind::Index { base, index } => {
                if !self.flat_expr(base, 13) {
                    return false;
                }
                self.out.push('[');
                if !self.flat_expr(index, 0) {
                    return false;
                }
                self.out.push(']');
                true
            }
            ExprKind::List(items) => {
                if !items.is_empty() && self.comments_within(e.span.start, e.span.end) {
                    return false; // interior comments pin the broken layout
                }
                self.out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    if !self.flat_expr(it, 0) {
                        return false;
                    }
                }
                self.out.push(']');
                true
            }
            ExprKind::MapLit(entries) => {
                if entries.is_empty() {
                    self.out.push_str("{:}");
                    return true;
                }
                if self.comments_within(e.span.start, e.span.end) {
                    return false; // interior comments pin the broken layout
                }
                self.out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    if !self.flat_expr(k, 0) {
                        return false;
                    }
                    self.out.push_str(": ");
                    if !self.flat_expr(v, 0) {
                        return false;
                    }
                }
                self.out.push('}');
                true
            }
            ExprKind::Tuple(items) => {
                if !items.is_empty() && self.comments_within(e.span.start, e.span.end) {
                    return false; // interior comments pin the broken layout
                }
                self.out.push('(');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    if !self.flat_expr(it, 0) {
                        return false;
                    }
                }
                self.out.push(')');
                true
            }
            ExprKind::Range { lo, hi, inclusive } => {
                if !self.flat_expr(lo, 6) {
                    return false;
                }
                self.out.push_str(if *inclusive { "..=" } else { ".." });
                self.flat_expr(hi, 6)
            }
            ExprKind::StructLit { name, fields } => {
                if !fields.is_empty() && self.comments_within(name.span.end, e.span.end) {
                    return false; // interior comments pin the broken layout
                }
                self.out.push_str(&name.name);
                self.out.push_str(" { ");
                for (i, (fname, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    // Shorthand when the value is a variable of the same name.
                    if let ExprKind::Var(v) = &value.kind {
                        if *v == fname.name {
                            self.out.push_str(&fname.name);
                            continue;
                        }
                    }
                    self.out.push_str(&fname.name);
                    self.out.push_str(": ");
                    if !self.flat_expr(value, 0) {
                        return false;
                    }
                }
                self.out.push_str(" }");
                true
            }
            ExprKind::Lambda { params, ret, body } => {
                self.lambda_params(params);
                if let Some(r) = ret {
                    self.out.push_str(" -> ");
                    self.type_expr(r);
                    self.out.push(' ');
                    // A return type requires a block body; only an empty one
                    // is a single line.
                    if let ExprKind::Block(b) = &body.kind {
                        if b.stmts.is_empty() && !self.comments_before(b.span.end) {
                            self.out.push_str("{}");
                            return true;
                        }
                    }
                    return false;
                }
                self.out.push(' ');
                self.flat_expr(body, 1)
            }
            ExprKind::If { cond, then, els } => {
                // Only an if/else(-if chain) whose every branch is a single
                // expression can be one line; the whole chain flattens (or
                // none of it does — `expr_broken` breaks every branch).
                let Some(els_e) = els else { return false };
                let Some(t) = self.inline_block(then) else { return false };
                let t = t.clone();
                self.out.push_str("if ");
                if !self.flat_expr(cond, 0) {
                    return false;
                }
                self.out.push_str(" { ");
                if !self.flat_expr(&t, 0) {
                    return false;
                }
                self.out.push_str(" } else ");
                match &els_e.kind {
                    // `else if ...`: flatten the rest of the chain in place.
                    ExprKind::If { .. } => self.flat_expr_inner(els_e),
                    ExprKind::Block(eb) => {
                        let Some(x) = self.inline_block(eb) else { return false };
                        let x = x.clone();
                        self.out.push_str("{ ");
                        if !self.flat_expr(&x, 0) {
                            return false;
                        }
                        self.out.push_str(" }");
                        true
                    }
                    _ => false,
                }
            }
            ExprKind::Block(_) | ExprKind::Match { .. } => false,
        }
    }

    fn flat_args(&mut self, args: &[Expr]) -> bool {
        self.out.push('(');
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            if !self.flat_expr(a, 0) {
                return false;
            }
        }
        self.out.push(')');
        true
    }

    fn lambda_params(&mut self, params: &[LambdaParam]) {
        if params.is_empty() {
            self.out.push_str("||");
            return;
        }
        self.out.push('|');
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.out.push_str(&p.name.name);
            if let Some(t) = &p.ty {
                self.out.push_str(": ");
                self.type_expr(t);
            }
        }
        self.out.push('|');
    }

    // ------------------------------------------------------------------
    // Expressions — broken renderers
    // ------------------------------------------------------------------

    /// Multi-line layout for an expression that does not fit flat. Children
    /// are formatted in auto mode again, so inner groups stay flat when they
    /// fit and breaking composes outermost-first.
    fn expr_broken(&mut self, e: &Expr, rider: usize) {
        match &e.kind {
            // Atoms never break; a single long token may overflow the width.
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::StringInterp { .. } => {
                let text = self.snippet(e.span).to_string();
                self.out.push_str(&text);
            }
            ExprKind::Bool(b) => self.out.push_str(if *b { "true" } else { "false" }),
            ExprKind::Unit => self.out.push_str("()"),
            ExprKind::Var(n) => self.out.push_str(n),
            ExprKind::Field { .. }
            | ExprKind::MethodCall { .. }
            | ExprKind::Try(_)
            | ExprKind::Index { .. } => self.chain_broken(e, rider),
            ExprKind::Call { callee, args } => {
                if matches!(callee.kind, ExprKind::Field { .. }) {
                    self.out.push('(');
                    self.expr(callee, 0, 1);
                    self.out.push(')');
                } else {
                    self.expr(callee, 13, 0);
                }
                self.call_args_group(args, callee.span.end, e.span.end, rider);
            }
            ExprKind::Unary { op, expr } => {
                self.out.push_str(match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                });
                self.expr(expr, 12, rider);
            }
            ExprKind::Binary { .. } => self.binary_broken(e, rider),
            ExprKind::List(items) => {
                if items.is_empty() {
                    self.out.push_str("[]");
                    return;
                }
                self.out.push_str("[\n");
                self.indent += 1;
                self.last_line = self.line_of(e.span.start);
                for it in items {
                    self.flush_comments(it.span.start);
                    self.pad();
                    self.expr(it, 0, 1);
                    self.out.push_str(",\n");
                    self.last_line = self.line_of(it.span.end.saturating_sub(1));
                }
                self.flush_comments(e.span.end);
                self.indent -= 1;
                self.pad();
                self.out.push(']');
            }
            ExprKind::MapLit(entries) => {
                if entries.is_empty() {
                    self.out.push_str("{:}");
                    return;
                }
                self.out.push_str("{\n");
                self.indent += 1;
                self.last_line = self.line_of(e.span.start);
                for (k, v) in entries {
                    self.flush_comments(k.span.start);
                    self.pad();
                    self.expr(k, 0, 2);
                    self.out.push_str(": ");
                    self.expr(v, 0, 1);
                    self.out.push_str(",\n");
                    self.last_line = self.line_of(v.span.end.saturating_sub(1));
                }
                self.flush_comments(e.span.end);
                self.indent -= 1;
                self.pad();
                self.out.push('}');
            }
            ExprKind::Tuple(items) => {
                self.out.push_str("(\n");
                self.indent += 1;
                self.last_line = self.line_of(e.span.start);
                for it in items {
                    self.flush_comments(it.span.start);
                    self.pad();
                    self.expr(it, 0, 1);
                    self.out.push_str(",\n");
                    self.last_line = self.line_of(it.span.end.saturating_sub(1));
                }
                self.flush_comments(e.span.end);
                self.indent -= 1;
                self.pad();
                self.out.push(')');
            }
            ExprKind::Range { lo, hi, inclusive } => {
                self.expr(lo, 6, 2);
                self.out.push_str(if *inclusive { "..=" } else { ".." });
                self.expr(hi, 6, rider);
            }
            ExprKind::StructLit { name, fields } => {
                self.out.push_str(&name.name);
                self.out.push_str(" {\n");
                self.indent += 1;
                self.last_line = self.line_of(name.span.end.saturating_sub(1));
                for (fname, value) in fields {
                    self.flush_comments(fname.span.start);
                    self.pad();
                    self.out.push_str(&fname.name);
                    // Shorthand when the value is a variable of the same name.
                    let shorthand = matches!(&value.kind, ExprKind::Var(v) if *v == fname.name);
                    if !shorthand {
                        self.out.push_str(": ");
                        self.expr(value, 0, 1);
                    }
                    self.out.push_str(",\n");
                    self.last_line = self.line_of(value.span.end.saturating_sub(1));
                }
                self.flush_comments(e.span.end);
                self.indent -= 1;
                self.pad();
                self.out.push('}');
            }
            ExprKind::Lambda { params, ret, body } => {
                self.lambda_broken(params, ret, body, rider)
            }
            ExprKind::If { cond, then, els } => {
                self.out.push_str("if ");
                self.expr(cond, 0, 2);
                self.out.push(' ');
                self.block(then);
                if let Some(els_e) = els {
                    self.out.push_str(" else ");
                    match &els_e.kind {
                        ExprKind::Block(b) => self.block(b),
                        // An else-if: the chain as a whole did not fit flat,
                        // so every branch of it breaks — never a half-inline
                        // chain like `} else if c { 1 } else { 0 }`.
                        _ => self.expr_broken(els_e, rider),
                    }
                }
            }
            ExprKind::Block(b) => self.block(b),
            ExprKind::Match { scrutinee, arms, sugar: MatchSugar::IfLet } => {
                // `if let PATTERN = SCRUTINEE { THEN } [else ...]` — the
                // parser's desugared shape (arm[0] = user pattern/then,
                // arm[1] = synthetic wildcard/else-or-Unit), printed back in
                // its original sugar form.
                self.out.push_str("if let ");
                self.pattern(&arms[0].pattern);
                self.out.push_str(" = ");
                self.expr(scrutinee, 0, 2);
                self.out.push(' ');
                let ExprKind::Block(then) = &arms[0].body.kind else {
                    unreachable!("if-let sugar always wraps THEN in a block")
                };
                self.block(then);
                if !matches!(arms[1].body.kind, ExprKind::Unit) {
                    self.out.push_str(" else ");
                    match &arms[1].body.kind {
                        ExprKind::Block(b) => self.block(b),
                        _ => self.expr_broken(&arms[1].body, rider),
                    }
                }
            }
            ExprKind::Match { scrutinee, arms, sugar: _ } => {
                self.out.push_str("match ");
                self.expr(scrutinee, 0, 2);
                self.out.push_str(" {\n");
                self.indent += 1;
                self.last_line = self.line_of(scrutinee.span.end);
                for arm in arms {
                    self.flush_comments(arm.span.start);
                    self.pad();
                    self.pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.out.push_str(" if ");
                        self.expr(g, 0, 4);
                    }
                    self.out.push_str(" -> ");
                    if arm.sugar {
                        // Print the bare-statement sugar back instead of the
                        // desugared one-statement block.
                        self.sugar_arm_body(&arm.body);
                    } else {
                        self.expr(&arm.body, 0, 1);
                    }
                    self.out.push_str(",\n");
                    self.last_line = self.line_of(arm.span.end.saturating_sub(1));
                }
                self.flush_comments(e.span.end);
                self.indent -= 1;
                self.pad();
                self.out.push('}');
            }
        }
    }

    /// Broken layout for postfix chains (`.method(args)` / `.field` / `?` /
    /// `[i]`). A chain that could be one line but is too wide breaks before
    /// each `.` element after the first; a chain that can never be one line
    /// (a block lambda argument, a `match`, ...) keeps the attached layout —
    /// `xs.map(|x| {` ... `}).join("")` — and lets each element's argument
    /// list break on its own.
    fn chain_broken(&mut self, e: &Expr, rider: usize) {
        let (base, base_sfx, links) = collect_chain(e);
        let break_chain = links.len() >= 2 && self.can_flatten(e);
        let base_rider = if links.is_empty() { rider + base_sfx.len() } else { 0 };
        self.expr(base, 13, base_rider);
        self.chain_suffixes(&base_sfx);
        let Some((first, tail)) = links.split_first() else { return };
        self.chain_link(first, if tail.is_empty() { rider } else { 0 });
        if break_chain {
            self.indent += 1;
            for (i, link) in tail.iter().enumerate() {
                self.out.push('\n');
                self.pad();
                self.chain_link(link, if i + 1 == tail.len() { rider } else { 0 });
            }
            self.indent -= 1;
        } else {
            for (i, link) in tail.iter().enumerate() {
                self.chain_link(link, if i + 1 == tail.len() { rider } else { 0 });
            }
        }
    }

    fn chain_link(&mut self, link: &ChainLink, rider: usize) {
        self.out.push('.');
        self.out.push_str(&link.name.name);
        if let Some(args) = link.args {
            self.call_args_group(args, link.name.span.end, link.close, rider + link.suffixes.len());
        }
        self.chain_suffixes(&link.suffixes);
    }

    fn chain_suffixes(&mut self, sfx: &[ChainSuffix]) {
        for s in sfx {
            match s {
                ChainSuffix::Try => self.out.push('?'),
                ChainSuffix::Index(ix) => {
                    self.out.push('[');
                    self.expr(ix, 0, 1);
                    self.out.push(']');
                }
            }
        }
    }

    /// An argument list: flat when it fits; hugged when the last argument can
    /// never be one line (so `f(a, |x| {` ... `})` keeps its shape); one
    /// argument per line with a trailing comma otherwise. `open`/`close`
    /// bound the argument region (from just after the callee to just past the
    /// `)`): interior comments there rule out the flat layout and are emitted
    /// next to their argument in the broken one.
    fn call_args_group(&mut self, args: &[Expr], open: u32, close: u32, rider: usize) {
        if args.is_empty() {
            self.out.push_str("()");
            return;
        }
        let commented = self.comments_within(open, close);
        if !commented && self.group(rider, |s| s.flat_args(args)) {
            return;
        }
        let (last, init) = args.split_last().expect("non-empty");
        // A single unbreakable token (usually a long string) that would not
        // fit even on a line of its own: breaking cannot help, keep it flat
        // (unless a comment needs the broken layout to survive).
        if init.is_empty() && is_atom(last) && !commented {
            let arg_width = self.measure(|s| {
                let _ = s.flat_expr(last, 0);
            });
            if (self.indent + 1) * 4 + arg_width + 1 > self.width {
                self.out.push('(');
                let _ = self.flat_expr(last, 0);
                self.out.push(')');
                return;
            }
        }
        // Hug a last argument that can never be one line, and also container
        // literals (`f([` ... `])`), which break element-per-line in place.
        // Comments before the last argument need one-argument-per-line lines
        // of their own to attach to, so they rule the hug out.
        let container = matches!(
            last.kind,
            ExprKind::List(_) | ExprKind::MapLit(_) | ExprKind::StructLit { .. }
        );
        if (container || !self.can_flatten(last))
            && !self.comments_within(open, last.span.start)
            && self.try_hug(init, last, rider)
        {
            return;
        }
        self.out.push_str("(\n");
        self.indent += 1;
        self.last_line = self.line_of(open.saturating_sub(1));
        for a in args {
            self.flush_comments(a.span.start);
            self.pad();
            self.expr(a, 0, 1);
            self.out.push_str(",\n");
            self.last_line = self.line_of(a.span.end.saturating_sub(1));
        }
        self.flush_comments(close);
        self.indent -= 1;
        self.pad();
        self.out.push(')');
    }

    /// Attempt the hugged argument layout: all arguments but the last flat,
    /// the last expanding in place, the `)` attached to its final line.
    /// Rolls back (including any comments consumed by the speculative render)
    /// and returns false when the leading arguments cannot stay flat.
    fn try_hug(&mut self, init: &[Expr], last: &Expr, rider: usize) -> bool {
        let mark = self.out.len();
        let col_before = self.col();
        let saved_next = self.next_comment;
        let saved_last_line = self.last_line;
        let saved_cap = self.flat_cap;
        self.flat_cap = mark + self.width.saturating_mul(4);
        self.out.push('(');
        let mut flat_ok = true;
        for a in init {
            if !self.flat_expr(a, 0) {
                flat_ok = false;
                break;
            }
            self.out.push_str(", ");
        }
        self.flat_cap = saved_cap;
        if flat_ok {
            self.expr(last, 0, 1);
            self.out.push(')');
            let text = &self.out[mark..];
            let first_line = text.split('\n').next().unwrap_or("");
            if text.contains('\n')
                && col_before + first_line.chars().count() <= self.width
                && self.col() + rider <= self.width
            {
                return true;
            }
        }
        self.out.truncate(mark);
        self.next_comment = saved_next;
        self.last_line = saved_last_line;
        false
    }

    /// Broken layout for binary expressions: gather the left spine of
    /// operators at the root's precedence level (associative operators only —
    /// this stays correct for any operator the language grows, since it is
    /// keyed off `bin_prec`/`non_assoc`, not specific tokens) and break
    /// before each operator with a hanging indent.
    fn binary_broken(&mut self, e: &Expr, rider: usize) {
        let ExprKind::Binary { op, lhs, rhs, .. } = &e.kind else { unreachable!() };
        let p = bin_prec(*op);
        let root_na = non_assoc(*op);
        if !self.can_flatten(e) {
            // A hard multi-line operand (`1 + match n {` ...): keep the
            // operators attached and let the operand break in place.
            let lhs_prec = if root_na { p + 1 } else { p };
            self.expr(lhs, lhs_prec, 0);
            self.out.push(' ');
            self.out.push_str(op.symbol());
            self.out.push(' ');
            self.expr(rhs, p + 1, rider);
            return;
        }
        let mut tail: Vec<(BinOp, &Expr)> = Vec::new(); // gathered rightmost-first
        let mut cur = e;
        let head = loop {
            match &cur.kind {
                ExprKind::Binary { op: o, lhs, rhs, .. }
                    if std::ptr::eq(cur, e)
                        || (bin_prec(*o) == p && !non_assoc(*o) && !root_na) =>
                {
                    tail.push((*o, rhs));
                    cur = lhs;
                }
                _ => break cur,
            }
        };
        tail.reverse();
        self.expr(head, if root_na { p + 1 } else { p }, 0);
        self.indent += 1;
        for (i, (o, rhs)) in tail.iter().enumerate() {
            self.out.push('\n');
            self.pad();
            self.out.push_str(o.symbol());
            self.out.push(' ');
            self.expr(rhs, p + 1, if i + 1 == tail.len() { rider } else { 0 });
        }
        self.indent -= 1;
    }

    /// Broken layout for lambdas. Block/`match`/`if` bodies break naturally
    /// after the parameters; any other body breaks *to a block* so it gets a
    /// fresh full-width line.
    fn lambda_broken(
        &mut self,
        params: &[LambdaParam],
        ret: &Option<TypeExpr>,
        body: &Expr,
        rider: usize,
    ) {
        self.lambda_params(params);
        if let Some(r) = ret {
            self.out.push_str(" -> ");
            self.type_expr(r);
            self.out.push(' ');
            if let ExprKind::Block(b) = &body.kind {
                self.block(b);
            } else {
                self.expr(body, 0, rider);
            }
            return;
        }
        match &body.kind {
            ExprKind::Block(b) => {
                self.out.push(' ');
                self.block(b);
            }
            ExprKind::Match { .. } | ExprKind::If { .. } => {
                self.out.push(' ');
                self.expr(body, 1, rider);
            }
            _ if !self.can_flatten(body) => {
                // A body that can never be one line (`|r| xs.map(|c| {` ...)
                // keeps the attached layout and breaks in place.
                self.out.push(' ');
                self.expr(body, 1, rider);
            }
            _ => {
                // `|x| looong` breaks to `|x| {` body `}`; reparsing yields a
                // block body, which the arms above then keep stable.
                self.out.push_str(" {\n");
                self.indent += 1;
                self.pad();
                self.expr(body, 0, 0);
                self.out.push('\n');
                self.indent -= 1;
                self.pad();
                self.out.push('}');
            }
        }
    }

    /// The body of a `sugar` match arm is a synthesized one-statement block
    /// holding a `return`/`break`/`continue`; print the statement bare.
    fn sugar_arm_body(&mut self, body: &Expr) {
        let ExprKind::Block(b) = &body.kind else {
            self.expr(body, 0, 1);
            return;
        };
        match b.stmts.first().map(|s| &s.kind) {
            Some(StmtKind::Return(v)) => {
                self.out.push_str("return");
                if let Some(v) = v {
                    self.out.push(' ');
                    self.expr(v, 0, 1);
                }
            }
            Some(StmtKind::Break) => self.out.push_str("break"),
            Some(StmtKind::Continue) => self.out.push_str("continue"),
            _ => self.expr(body, 0, 1),
        }
    }

    // ------------------------------------------------------------------
    // Patterns & types
    // ------------------------------------------------------------------

    fn pattern(&mut self, p: &Pattern) {
        match &p.kind {
            PatternKind::Wildcard => self.out.push('_'),
            PatternKind::Binding(n) => self.out.push_str(n),
            PatternKind::Int(_) | PatternKind::Float(_) | PatternKind::Str(_) => {
                let text = self.snippet(p.span).to_string();
                self.out.push_str(&text);
            }
            PatternKind::Bool(b) => self.out.push_str(if *b { "true" } else { "false" }),
            PatternKind::Unit => self.out.push_str("()"),
            PatternKind::Tuple(items) => {
                self.out.push('(');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.pattern(it);
                }
                self.out.push(')');
            }
            PatternKind::Variant { enum_name, variant, fields, has_parens } => {
                if let Some(en) = enum_name {
                    self.out.push_str(&en.name);
                    self.out.push('.');
                }
                self.out.push_str(&variant.name);
                if *has_parens {
                    self.out.push('(');
                    for (i, f) in fields.iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(", ");
                        }
                        self.pattern(f);
                    }
                    self.out.push(')');
                }
            }
            PatternKind::Struct { name, fields, rest } => {
                self.out.push_str(&name.name);
                self.out.push_str(" { ");
                for (i, (fname, fpat)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    if let PatternKind::Binding(b) = &fpat.kind {
                        if *b == fname.name {
                            self.out.push_str(&fname.name);
                            continue;
                        }
                    }
                    self.out.push_str(&fname.name);
                    self.out.push_str(": ");
                    self.pattern(fpat);
                }
                if *rest {
                    if !fields.is_empty() {
                        self.out.push_str(", ");
                    }
                    self.out.push_str("..");
                }
                self.out.push_str(" }");
            }
            PatternKind::Or(alts) => {
                for (i, a) in alts.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(" | ");
                    }
                    self.pattern(a);
                }
            }
        }
    }

    fn type_expr(&mut self, t: &TypeExpr) {
        match &t.kind {
            TypeExprKind::Unit => self.out.push_str("()"),
            TypeExprKind::Named { name, args } => {
                self.out.push_str(&name.name);
                if !args.is_empty() {
                    self.out.push('[');
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(", ");
                        }
                        self.type_expr(a);
                    }
                    self.out.push(']');
                }
            }
            TypeExprKind::Tuple(items) => {
                self.out.push('(');
                for (i, a) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.type_expr(a);
                }
                self.out.push(')');
            }
            TypeExprKind::Fn { params, ret } => {
                self.out.push_str("fn(");
                for (i, a) in params.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.type_expr(a);
                }
                self.out.push(')');
                if let Some(r) = ret {
                    self.out.push_str(" -> ");
                    self.type_expr(r);
                }
            }
        }
    }
}

/// Recognizes the parser's `while let` desugaring — `while true { match
/// SCRUTINEE { PATTERN -> BODY, _ -> break } }`, tagged `MatchSugar::WhileLet`
/// — so the formatter can print it back in its original sugar form. Keyed
/// off the explicit tag rather than the shape, so a hand-written loop that
/// merely looks the same is never reinterpreted.
fn while_let_sugar<'a>(cond: &Expr, body: &'a Block) -> Option<(&'a Pattern, &'a Expr, &'a Block)> {
    if !matches!(cond.kind, ExprKind::Bool(true)) {
        return None;
    }
    let [stmt] = body.stmts.as_slice() else { return None };
    let StmtKind::Expr { expr, tail: true } = &stmt.kind else { return None };
    let ExprKind::Match { scrutinee, arms, sugar: MatchSugar::WhileLet } = &expr.kind else {
        return None;
    };
    let [user_arm, _fallback] = arms.as_slice() else { return None };
    let ExprKind::Block(user_body) = &user_arm.body.kind else { return None };
    Some((&user_arm.pattern, scrutinee, user_body))
}

/// A leaf the formatter can never split across lines.
fn is_atom(e: &Expr) -> bool {
    matches!(
        e.kind,
        ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::StringInterp { .. }
            | ExprKind::Bool(_)
            | ExprKind::Unit
            | ExprKind::Var(_)
    )
}

fn bin_prec(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Ne => 3,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
        // Range sits at 5; bitwise (v0.7) binds tighter than ranges and
        // looser than arithmetic, in Rust's relative order.
        BinOp::BitOr => 6,
        BinOp::BitXor => 7,
        BinOp::BitAnd => 8,
        BinOp::Shl | BinOp::Shr => 9,
        BinOp::Add | BinOp::Sub => 10,
        BinOp::Mul | BinOp::Div | BinOp::Rem => 11,
    }
}

/// Operators that are non-associative in the grammar: parenthesized operands
/// at the same precedence must keep their parens, and broken layouts must not
/// flatten through them.
fn non_assoc(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

fn expr_prec(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Binary { op, .. } => bin_prec(*op),
        ExprKind::Range { .. } => 5,
        ExprKind::Unary { .. } => 12,
        ExprKind::Lambda { .. } => 1,
        ExprKind::If { .. } | ExprKind::Match { .. } => 13,
        _ => 14,
    }
}
