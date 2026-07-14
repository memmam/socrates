//! The canonical source formatter (`fable fmt`).
//!
//! Prints the AST back out with normalized indentation (4 spaces), spacing,
//! and trailing commas. Literals (numbers, strings) are copied verbatim from
//! the source so radix, digit separators, and escapes survive. Comments are
//! preserved: they re-attach before the next statement at the right indent,
//! and a comment on the same line as a statement stays a trailing comment.
//! Blank-line runs between statements collapse to a single blank line.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::source::Source;
use crate::span::Span;
use crate::token::Comment;

pub fn format_source(name: &str, text: &str) -> Result<String, Vec<Diagnostic>> {
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
                self.expr(init, 0);
                self.out.push_str(";\n");
            }
            StmtKind::Assign { target, op, value } => {
                self.expr(target, 9);
                let sym = match op {
                    None => "=",
                    Some(BinOp::Add) => "+=",
                    Some(BinOp::Sub) => "-=",
                    Some(BinOp::Mul) => "*=",
                    Some(BinOp::Div) => "/=",
                    Some(BinOp::Rem) => "%=",
                    Some(_) => "=",
                };
                self.out.push(' ');
                self.out.push_str(sym);
                self.out.push(' ');
                self.expr(value, 0);
                self.out.push_str(";\n");
            }
            StmtKind::Expr { expr, tail } => {
                self.expr(expr, 0);
                let block_like = matches!(
                    expr.kind,
                    ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Block(_)
                );
                if !*tail && !block_like {
                    self.out.push(';');
                }
                self.out.push('\n');
            }
            StmtKind::While { cond, body } => {
                self.out.push_str("while ");
                self.expr(cond, 0);
                self.out.push(' ');
                self.block(body);
                self.out.push('\n');
            }
            StmtKind::For { var, iter, body, .. } => {
                self.out.push_str("for ");
                self.out.push_str(&var.name);
                self.out.push_str(" in ");
                self.expr(iter, 0);
                self.out.push(' ');
                self.block(body);
                self.out.push('\n');
            }
            StmtKind::Return(v) => {
                self.out.push_str("return");
                if let Some(v) = v {
                    self.out.push(' ');
                    self.expr(v, 0);
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
        if !f.generics.is_empty() {
            self.out.push('[');
            for (i, g) in f.generics.iter().enumerate() {
                if i > 0 {
                    self.out.push_str(", ");
                }
                self.out.push_str(&g.name);
            }
            self.out.push(']');
        }
        self.out.push('(');
        for (i, p) in f.params.iter().enumerate() {
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
        if let Some(r) = &f.ret {
            self.out.push_str(" -> ");
            self.type_expr(r);
        }
        self.out.push(' ');
        self.block(&f.body);
        self.out.push('\n');
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
    // Expressions
    // ------------------------------------------------------------------

    /// `prec` is the minimum precedence this context binds at; wrap the
    /// expression in parens if its own precedence is lower.
    fn expr(&mut self, e: &Expr, prec: u8) {
        let my_prec = expr_prec(e);
        let parens = my_prec < prec;
        if parens {
            self.out.push('(');
        }
        self.expr_inner(e);
        if parens {
            self.out.push(')');
        }
    }

    fn expr_inner(&mut self, e: &Expr) {
        match &e.kind {
            // Literals verbatim from the source.
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) => {
                let text = self.snippet(e.span).to_string();
                self.out.push_str(&text);
            }
            ExprKind::StringInterp { .. } => {
                let text = self.snippet(e.span).to_string();
                self.out.push_str(&text);
            }
            ExprKind::Bool(b) => self.out.push_str(if *b { "true" } else { "false" }),
            ExprKind::Unit => self.out.push_str("()"),
            ExprKind::Var(n) => self.out.push_str(n),
            ExprKind::Field { base, field } => {
                self.expr(base, 9);
                self.out.push('.');
                self.out.push_str(&field.name);
            }
            ExprKind::Call { callee, args } => {
                // `(s.field)(args)` must keep its parens: without them the
                // source re-parses as a method call `s.field(args)`.
                if matches!(callee.kind, ExprKind::Field { .. }) {
                    self.out.push('(');
                    self.expr_inner(callee);
                    self.out.push(')');
                } else {
                    self.expr(callee, 9);
                }
                self.call_args(args);
            }
            ExprKind::MethodCall { recv, method, args } => {
                self.expr(recv, 9);
                self.out.push('.');
                self.out.push_str(&method.name);
                self.call_args(args);
            }
            ExprKind::Unary { op, expr } => {
                self.out.push_str(match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                });
                self.expr(expr, 8);
            }
            ExprKind::Try(inner) => {
                self.expr(inner, 9);
                self.out.push('?');
            }
            ExprKind::Binary { op, lhs, rhs, .. } => {
                let p = bin_prec(*op);
                // Comparison/equality operators are NON-associative in the
                // grammar: `(a == b) == c` must keep its parens or it will
                // re-parse as a rejected chain.
                let non_assoc = matches!(
                    op,
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                );
                self.expr(lhs, if non_assoc { p + 1 } else { p });
                self.out.push(' ');
                self.out.push_str(op.symbol());
                self.out.push(' ');
                // Right operand binds one level tighter in both cases.
                self.expr(rhs, p + 1);
            }
            ExprKind::Index { base, index } => {
                self.expr(base, 9);
                self.out.push('[');
                self.expr(index, 0);
                self.out.push(']');
            }
            ExprKind::List(items) => {
                self.out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.expr(it, 0);
                }
                self.out.push(']');
            }
            ExprKind::MapLit(entries) => {
                if entries.is_empty() {
                    self.out.push_str("{:}");
                    return;
                }
                self.out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.expr(k, 0);
                    self.out.push_str(": ");
                    self.expr(v, 0);
                }
                self.out.push('}');
            }
            ExprKind::Tuple(items) => {
                self.out.push('(');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.expr(it, 0);
                }
                self.out.push(')');
            }
            ExprKind::Range { lo, hi, inclusive } => {
                self.expr(lo, 6);
                self.out.push_str(if *inclusive { "..=" } else { ".." });
                self.expr(hi, 6);
            }
            ExprKind::StructLit { name, fields } => {
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
                    self.expr(value, 0);
                }
                self.out.push_str(" }");
            }
            ExprKind::Lambda { params, ret, body } => {
                if params.is_empty() {
                    self.out.push_str("||");
                } else {
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
                if let Some(r) = ret {
                    self.out.push_str(" -> ");
                    self.type_expr(r);
                    self.out.push(' ');
                    if let ExprKind::Block(b) = &body.kind {
                        self.block(b);
                    } else {
                        self.expr(body, 0);
                    }
                } else {
                    self.out.push(' ');
                    self.expr(body, 1);
                }
            }
            ExprKind::If { cond, then, els } => {
                self.out.push_str("if ");
                self.expr(cond, 0);
                self.out.push(' ');
                // Inline single-expression branches, block otherwise.
                let inline_e = els.as_ref().and_then(|els_e| match &els_e.kind {
                    // Reuse inline_block so its comment guard applies: an
                    // else-block containing comments must stay multi-line.
                    ExprKind::Block(b) => self.inline_block(b),
                    _ => None,
                });
                match (self.inline_block(then), els) {
                    (Some(t), Some(_)) if inline_e.is_some() => {
                        let t = t.clone();
                        let e = inline_e.unwrap().clone();
                        self.out.push_str("{ ");
                        self.expr(&t, 0);
                        self.out.push_str(" } else { ");
                        self.expr(&e, 0);
                        self.out.push_str(" }");
                    }
                    _ => {
                        self.block(then);
                        if let Some(els_e) = els {
                            self.out.push_str(" else ");
                            match &els_e.kind {
                                ExprKind::Block(b) => self.block(b),
                                _ => self.expr_inner(els_e), // else-if chain
                            }
                        }
                    }
                }
            }
            ExprKind::Block(b) => self.block(b),
            ExprKind::Match { scrutinee, arms } => {
                self.out.push_str("match ");
                self.expr(scrutinee, 0);
                self.out.push_str(" {\n");
                self.indent += 1;
                self.last_line = self.line_of(scrutinee.span.end);
                for arm in arms {
                    self.flush_comments(arm.span.start);
                    self.pad();
                    self.pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.out.push_str(" if ");
                        self.expr(g, 0);
                    }
                    self.out.push_str(" -> ");
                    self.expr(&arm.body, 0);
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

    fn call_args(&mut self, args: &[Expr]) {
        self.out.push('(');
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.expr(a, 0);
        }
        self.out.push(')');
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

fn bin_prec(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Ne => 3,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 4,
        BinOp::Add | BinOp::Sub => 6,
        BinOp::Mul | BinOp::Div | BinOp::Rem => 7,
    }
}

fn expr_prec(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Binary { op, .. } => bin_prec(*op),
        ExprKind::Range { .. } => 5,
        ExprKind::Unary { .. } => 8,
        ExprKind::Lambda { .. } => 1,
        ExprKind::If { .. } | ExprKind::Match { .. } => 9,
        _ => 10,
    }
}
