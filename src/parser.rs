//! Recursive-descent parser: tokens → AST.
//!
//! Notable disambiguations:
//! - `{ ... }` in expression position may be a block or a map literal. We
//!   speculatively parse `expr :` after the `{`; on failure we roll back (token
//!   position and buffered diagnostics) and parse a block. `{:}` is the empty map.
//! - `Ident { ... }` is a struct literal except in "no-struct" contexts (the
//!   condition of `if`/`while`, `for` iterables, `match` scrutinees) where the
//!   `{` belongs to the following block.
//! - `x.0.1` lexes the `0.1` as a float; the parser splits it back into two
//!   tuple-index accesses using the token's source text.
//! - `|` at expression start begins a lambda; `||` there is an empty-param lambda.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::{NodeId, Span};
use crate::token::{Token, TokenKind};

pub struct ParseOutput {
    pub program: Program,
    pub diags: Vec<Diagnostic>,
    /// Number of NodeIds allocated (ids are `0..node_count`).
    pub node_count: u32,
}

pub fn parse(tokens: Vec<Token>, src: &str) -> ParseOutput {
    parse_with_ids(tokens, src, 0)
}

/// Parse with a starting NodeId offset (used by the REPL so ids never collide
/// across snippets sharing one session).
pub fn parse_with_ids(tokens: Vec<Token>, src: &str, first_id: u32) -> ParseOutput {
    let mut p = Parser {
        tokens,
        pos: 0,
        src,
        diags: Vec::new(),
        next_id: first_id,
        no_struct: false,
        depth: 0,
        depth_error: false,
        brace_memo: std::collections::HashMap::new(),
    };
    let program = p.program();
    ParseOutput { program, diags: p.diags, node_count: p.next_id }
}

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    src: &'a str,
    diags: Vec<Diagnostic>,
    next_id: u32,
    /// True while parsing a context where `Ident {` must not be a struct literal.
    no_struct: bool,
    /// Current expression/pattern/type nesting depth (recursion guard).
    depth: u32,
    depth_error: bool,
    /// `{` token index → "is a map literal", so the speculative map-vs-block
    /// parse is decided once per brace (otherwise nested blocks re-speculate
    /// exponentially).
    brace_memo: std::collections::HashMap<usize, bool>,
}

/// Nesting deeper than this gets a clean diagnostic instead of exhausting the
/// (large, but finite) parser stack.
const MAX_NESTING: u32 = 2000;

type PResult<T> = Result<T, ()>;

impl<'a> Parser<'a> {
    // ---- primitives ----

    fn id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos.min(self.tokens.len() - 1)].kind
    }

    fn peek2(&self) -> &TokenKind {
        &self.tokens[(self.pos + 1).min(self.tokens.len() - 1)].kind
    }

    fn span(&self) -> Span {
        self.tokens[self.pos.min(self.tokens.len() - 1)].span
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos.min(self.tokens.len() - 1)].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> PResult<Token> {
        if self.at(kind) {
            Ok(self.advance())
        } else {
            self.error_here(format!("expected {what}, found {}", self.peek().describe()));
            Err(())
        }
    }

    fn error_here(&mut self, msg: impl Into<String>) {
        let span = self.span();
        self.diags.push(Diagnostic::error("E0200", msg).with_label(span, ""));
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    /// Panic-mode recovery: skip tokens until a likely statement boundary.
    fn synchronize(&mut self) {
        let mut depth = 0i32;
        while !self.at_eof() {
            match self.peek() {
                TokenKind::Semi if depth <= 0 => {
                    self.advance();
                    return;
                }
                TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => {
                    depth += 1;
                }
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                TokenKind::Let
                | TokenKind::Fn
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::While
                | TokenKind::For
                | TokenKind::Return
                    if depth <= 0 =>
                {
                    return;
                }
                _ => {}
            }
            self.advance();
        }
    }

    /// A bare identifier `and`/`or` directly after a complete expression is
    /// never valid; give the targeted hint instead of a generic parse error.
    fn word_operator_hint(&mut self, word: &str, op: &str) {
        if let TokenKind::Ident(name) = self.peek() {
            if name == word {
                let span = self.span();
                self.diags.push(
                    Diagnostic::error(
                        "E0106",
                        format!("`{word}` is not an operator; logical {word} is spelled `{op}`"),
                    )
                    .with_label(span, format!("write `{op}` here")),
                );
                self.advance();
            }
        }
    }

    fn ident(&mut self, what: &str) -> PResult<Ident> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                let t = self.advance();
                Ok(Ident { name, span: t.span })
            }
            _ => {
                self.error_here(format!("expected {what}, found {}", self.peek().describe()));
                Err(())
            }
        }
    }

    // ---- program & statements ----

    fn program(&mut self) -> Program {
        let mut stmts = Vec::new();
        while !self.at_eof() {
            let before = self.pos;
            match self.stmt(true, false) {
                Ok(Some(s)) => stmts.push(s),
                Ok(None) => {}
                Err(()) => self.synchronize(),
            }
            if self.pos == before && !self.at_eof() {
                // Ensure forward progress on unrecoverable input.
                self.advance();
            }
        }
        Program { stmts }
    }

    /// Parse one statement. `top_level` allows item declarations. `in_block`
    /// enables tail-expression semantics before a closing `}`.
    fn stmt(&mut self, top_level: bool, in_block: bool) -> PResult<Option<Stmt>> {
        if self.at(&TokenKind::Pub) {
            let pub_span = self.advance().span;
            if !top_level {
                self.diags.push(
                    Diagnostic::error("E0201", "`pub` is only allowed at the top level")
                        .with_label(pub_span, ""),
                );
            }
            return match self.peek() {
                TokenKind::Fn => {
                    let mut f = self.fn_decl()?;
                    f.is_pub = true;
                    let span = pub_span.to(f.span);
                    Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Fn(f) }))
                }
                TokenKind::Struct => {
                    let mut s = self.struct_decl()?;
                    s.is_pub = true;
                    let span = pub_span.to(s.span);
                    Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Struct(s) }))
                }
                TokenKind::Enum => {
                    let mut e = self.enum_decl()?;
                    e.is_pub = true;
                    let span = pub_span.to(e.span);
                    Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Enum(e) }))
                }
                TokenKind::Let => {
                    let mut stmt = self.let_stmt()?;
                    if let StmtKind::Let { is_pub, .. } = &mut stmt.kind {
                        *is_pub = true;
                    }
                    stmt.span = pub_span.to(stmt.span);
                    Ok(Some(stmt))
                }
                other => {
                    self.error_here(format!(
                        "expected `fn`, `struct`, `enum`, or `let` after `pub`, found {}",
                        other.describe()
                    ));
                    Err(())
                }
            };
        }
        match self.peek() {
            TokenKind::Semi => {
                self.advance(); // stray semicolon: harmless empty statement
                Ok(None)
            }
            TokenKind::Fn => {
                let f = self.fn_decl()?;
                if !top_level {
                    self.diags.push(
                        Diagnostic::error(
                            "E0201",
                            "function declarations are only allowed at the top level",
                        )
                        .with_label(f.span, "declared here")
                        .with_note("use a lambda instead: `let f = |x| ...;`"),
                    );
                }
                let span = f.span;
                Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Fn(f) }))
            }
            TokenKind::Struct => {
                let s = self.struct_decl()?;
                if !top_level {
                    self.diags.push(
                        Diagnostic::error(
                            "E0201",
                            "struct declarations are only allowed at the top level",
                        )
                        .with_label(s.span, "declared here"),
                    );
                }
                let span = s.span;
                Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Struct(s) }))
            }
            TokenKind::Enum => {
                let e = self.enum_decl()?;
                if !top_level {
                    self.diags.push(
                        Diagnostic::error(
                            "E0201",
                            "enum declarations are only allowed at the top level",
                        )
                        .with_label(e.span, "declared here"),
                    );
                }
                let span = e.span;
                Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Enum(e) }))
            }
            TokenKind::Impl => {
                let i = self.impl_decl()?;
                if !top_level {
                    self.diags.push(
                        Diagnostic::error(
                            "E0201",
                            "impl blocks are only allowed at the top level",
                        )
                        .with_label(i.span, "declared here"),
                    );
                }
                let span = i.span;
                Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Impl(i) }))
            }
            TokenKind::Import => {
                let s = self.import_stmt()?;
                if !top_level {
                    self.diags.push(
                        Diagnostic::error(
                            "E0201",
                            "imports are only allowed at the top level",
                        )
                        .with_label(s.span, ""),
                    );
                }
                Ok(Some(s))
            }
            TokenKind::Let => self.let_stmt().map(Some),
            TokenKind::While => self.while_stmt().map(Some),
            TokenKind::For => self.for_stmt().map(Some),
            TokenKind::Return => {
                let start = self.advance().span;
                let value = if self.at(&TokenKind::Semi) {
                    None
                } else {
                    Some(self.expr()?)
                };
                let end = self.span();
                self.expect(&TokenKind::Semi, "`;` after `return`")?;
                Ok(Some(Stmt {
                    span: start.to(end),
                    id: self.id(),
                    kind: StmtKind::Return(value),
                }))
            }
            TokenKind::Break => {
                let span = self.advance().span;
                self.expect(&TokenKind::Semi, "`;` after `break`")?;
                Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Break }))
            }
            TokenKind::Continue => {
                let span = self.advance().span;
                self.expect(&TokenKind::Semi, "`;` after `continue`")?;
                Ok(Some(Stmt { span, id: self.id(), kind: StmtKind::Continue }))
            }
            _ => self.expr_or_assign_stmt(in_block).map(Some),
        }
    }

    fn expr_or_assign_stmt(&mut self, in_block: bool) -> PResult<Stmt> {
        let expr = self.expr()?;
        let start = expr.span;

        // Assignment?
        let op = match self.peek() {
            TokenKind::Eq => Some(None),
            TokenKind::PlusEq => Some(Some(BinOp::Add)),
            TokenKind::MinusEq => Some(Some(BinOp::Sub)),
            TokenKind::StarEq => Some(Some(BinOp::Mul)),
            TokenKind::SlashEq => Some(Some(BinOp::Div)),
            TokenKind::PercentEq => Some(Some(BinOp::Rem)),
            TokenKind::AmpEq => Some(Some(BinOp::BitAnd)),
            TokenKind::PipeEq => Some(Some(BinOp::BitOr)),
            TokenKind::CaretEq => Some(Some(BinOp::BitXor)),
            TokenKind::ShlEq => Some(Some(BinOp::Shl)),
            TokenKind::ShrEq => Some(Some(BinOp::Shr)),
            _ => None,
        };
        if let Some(op) = op {
            let op_span = self.advance().span;
            if !matches!(
                expr.kind,
                ExprKind::Var(_) | ExprKind::Field { .. } | ExprKind::Index { .. }
            ) {
                self.diags.push(
                    Diagnostic::error("E0202", "invalid assignment target")
                        .with_label(expr.span, "cannot assign to this expression")
                        .with_secondary(op_span, ""),
                );
            }
            let value = self.expr()?;
            let end = self.span();
            self.expect(&TokenKind::Semi, "`;` after assignment")?;
            return Ok(Stmt {
                span: start.to(end),
                id: self.id(),
                kind: StmtKind::Assign { target: expr, op, value },
            });
        }

        // Expression statement / tail expression.
        if self.eat(&TokenKind::Semi) {
            return Ok(Stmt {
                span: start.to(self.prev_span()),
                id: self.id(),
                kind: StmtKind::Expr { expr, tail: false },
            });
        }
        let block_like = matches!(
            expr.kind,
            ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Block(_)
        );
        if in_block && self.at(&TokenKind::RBrace) {
            return Ok(Stmt { span: start, id: self.id(), kind: StmtKind::Expr { expr, tail: true } });
        }
        if block_like {
            return Ok(Stmt {
                span: start,
                id: self.id(),
                kind: StmtKind::Expr { expr, tail: false },
            });
        }
        if self.at_eof() && !in_block {
            // Allow a final expression without `;` at top level (script-friendly).
            return Ok(Stmt { span: start, id: self.id(), kind: StmtKind::Expr { expr, tail: false } });
        }
        self.error_here(format!(
            "expected `;` after expression, found {}",
            self.peek().describe()
        ));
        Err(())
    }

    fn let_stmt(&mut self) -> PResult<Stmt> {
        let start = self.advance().span; // `let`
        let mutable = self.eat(&TokenKind::Mut);
        let pattern = self.let_pattern()?;
        let ty = if self.eat(&TokenKind::Colon) { Some(self.type_expr()?) } else { None };
        self.expect(&TokenKind::Eq, "`=` in `let` binding")?;
        let init = self.expr()?;
        let end = self.span();
        self.expect(&TokenKind::Semi, "`;` after `let` binding")?;
        Ok(Stmt {
            span: start.to(end),
            id: self.id(),
            kind: StmtKind::Let { is_pub: false, mutable, pattern, ty, init },
        })
    }

    /// Patterns allowed on the left of `let`: binding, tuple, struct.
    fn let_pattern(&mut self) -> PResult<Pattern> {
        let pat = self.pattern()?;
        Ok(pat)
    }

    fn while_stmt(&mut self) -> PResult<Stmt> {
        let start = self.advance().span;
        if self.at(&TokenKind::Let) {
            return self.while_let_stmt(start);
        }
        let cond = self.no_struct_expr()?;
        let body = self.block()?;
        Ok(Stmt {
            span: start.to(body.span),
            id: self.id(),
            kind: StmtKind::While { cond, body },
        })
    }

    /// `while let PATTERN = EXPR { BODY }` (v0.8) — sugar for the
    /// recv/drain-loop dance (`while true { match EXPR { PATTERN -> BODY,
    /// _ -> break } }`, already a documented idiom, STYLE.md § 5). Desugars
    /// fully here: the outer `while true` and the fallback `_ -> break` arm
    /// are synthesized, so the checker's usual `expect_unit_body` on the
    /// while's block forces BODY to be Unit exactly as a hand-written loop
    /// would, and codegen is the ordinary While + Match paths, unchanged.
    fn while_let_stmt(&mut self, start: Span) -> PResult<Stmt> {
        self.advance(); // `let`
        let pattern = self.pattern()?;
        self.expect(&TokenKind::Eq, "`=` after `while let` pattern")?;
        let scrutinee = self.no_struct_expr()?;
        let body = self.block()?;
        let body_span = body.span;
        let match_span = start.to(body_span);

        let user_arm = MatchArm {
            pattern,
            guard: None,
            body: Expr { span: body_span, id: self.id(), kind: ExprKind::Block(body) },
            span: match_span,
            sugar: false,
        };
        let break_block = Block {
            stmts: vec![Stmt { span: body_span, id: self.id(), kind: StmtKind::Break }],
            span: body_span,
            id: self.id(),
        };
        let fallback_arm = MatchArm {
            pattern: Pattern { kind: PatternKind::Wildcard, span: body_span, id: self.id() },
            guard: None,
            body: Expr { span: body_span, id: self.id(), kind: ExprKind::Block(break_block) },
            span: body_span,
            sugar: true,
        };
        let match_expr = Expr {
            span: match_span,
            id: self.id(),
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: vec![user_arm, fallback_arm],
                sugar: MatchSugar::WhileLet,
            },
        };
        let loop_body = Block {
            stmts: vec![Stmt {
                span: match_span,
                id: self.id(),
                kind: StmtKind::Expr { expr: match_expr, tail: true },
            }],
            span: match_span,
            id: self.id(),
        };
        Ok(Stmt {
            span: match_span,
            id: self.id(),
            kind: StmtKind::While {
                cond: Expr { span: start, id: self.id(), kind: ExprKind::Bool(true) },
                body: loop_body,
            },
        })
    }

    fn for_stmt(&mut self) -> PResult<Stmt> {
        let start = self.advance().span;
        // A full pattern, so `for (i, x) in xs.enumerate()` and `for _ in r`
        // work; the checker enforces irrefutability exactly as for `let`.
        let pattern = self.pattern()?;
        self.expect(&TokenKind::In, "`in`")?;
        let iter = self.no_struct_expr()?;
        let body = self.block()?;
        Ok(Stmt {
            span: start.to(body.span),
            id: self.id(),
            kind: StmtKind::For { pattern, iter, body },
        })
    }

    // ---- items ----

    fn fn_decl(&mut self) -> PResult<FnDecl> {
        let start = self.advance().span; // `fn`
        let name = self.ident("function name")?;
        let generics = self.generics()?;
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut params = Vec::new();
        while !self.at(&TokenKind::RParen) {
            let pname = self.ident("parameter name")?;
            self.expect(&TokenKind::Colon, "`:` (parameter types are required)")?;
            let ty = self.type_expr()?;
            params.push(Param { name: pname, ty, id: self.id() });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        let ret = if self.eat(&TokenKind::Arrow) { Some(self.type_expr()?) } else { None };
        let body = self.block()?;
        Ok(FnDecl {
            is_pub: false,
            span: start.to(body.span),
            id: self.id(),
            name,
            generics,
            params,
            ret,
            body,
        })
    }

    /// `import a.b;` / `import a.b as c;`
    fn import_stmt(&mut self) -> PResult<Stmt> {
        let start = self.advance().span; // `import`
        let mut path = vec![self.ident("module name")?];
        while self.eat(&TokenKind::Dot) {
            path.push(self.ident("module name")?);
        }
        // `as` is contextual: an ordinary identifier here.
        let alias = if matches!(self.peek(), TokenKind::Ident(n) if n == "as") {
            self.advance();
            Some(self.ident("module alias")?)
        } else {
            None
        };
        let end = self.span();
        self.expect(&TokenKind::Semi, "`;` after import")?;
        Ok(Stmt {
            span: start.to(end),
            id: self.id(),
            kind: StmtKind::Import { path, alias },
        })
    }

    fn impl_decl(&mut self) -> PResult<ImplDecl> {
        let start = self.advance().span; // `impl`
        let ty_name = self.ident("type name")?;
        let generics = self.generics()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut methods = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            let is_pub = self.eat(&TokenKind::Pub);
            if !self.at(&TokenKind::Fn) {
                self.error_here(format!(
                    "expected {} or `}}` in an impl block, found {}",
                    if is_pub { "`fn`" } else { "`pub` or `fn`" },
                    self.peek().describe()
                ));
                return Err(());
            }
            let mut m = self.method_decl(&ty_name, &generics)?;
            m.is_pub = is_pub;
            methods.push(m);
        }
        let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
        Ok(ImplDecl { span: start.to(end), id: self.id(), ty_name, generics, methods })
    }

    /// A method inside an impl block: like `fn_decl`, but the first parameter
    /// must be a bare `self`, whose type (`TypeName[G, ...]`) is synthesized
    /// here so methods run through the ordinary function machinery.
    fn method_decl(&mut self, ty_name: &Ident, impl_generics: &[Ident]) -> PResult<FnDecl> {
        let start = self.advance().span; // `fn`
        let name = self.ident("method name")?;
        let generics = self.generics()?;
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut params = Vec::new();
        match self.peek() {
            TokenKind::Ident(n) if n == "self" => {
                let pname = self.ident("parameter name")?;
                if self.at(&TokenKind::Colon) {
                    self.error_here(
                        "`self` takes no type annotation; it always has the impl type",
                    );
                    return Err(());
                }
                let span = pname.span;
                let args = impl_generics
                    .iter()
                    .map(|g| TypeExpr {
                        span,
                        id: self.id(),
                        kind: TypeExprKind::Named {
                            name: Ident { name: g.name.clone(), span },
                            args: Vec::new(),
                        },
                    })
                    .collect();
                let ty = TypeExpr {
                    span,
                    id: self.id(),
                    kind: TypeExprKind::Named {
                        name: Ident { name: ty_name.name.clone(), span },
                        args,
                    },
                };
                params.push(Param { name: pname, ty, id: self.id() });
                self.eat(&TokenKind::Comma);
            }
            _ => {
                self.error_here(format!(
                    "the first parameter of a method must be `self`, found {}",
                    self.peek().describe()
                ));
                return Err(());
            }
        }
        while !self.at(&TokenKind::RParen) {
            let pname = self.ident("parameter name")?;
            self.expect(&TokenKind::Colon, "`:` (parameter types are required)")?;
            let ty = self.type_expr()?;
            params.push(Param { name: pname, ty, id: self.id() });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        let ret = if self.eat(&TokenKind::Arrow) { Some(self.type_expr()?) } else { None };
        let body = self.block()?;
        Ok(FnDecl {
            is_pub: false,
            span: start.to(body.span),
            id: self.id(),
            name,
            generics,
            params,
            ret,
            body,
        })
    }

    fn generics(&mut self) -> PResult<Vec<Ident>> {
        let mut out = Vec::new();
        if self.eat(&TokenKind::LBracket) {
            while !self.at(&TokenKind::RBracket) {
                out.push(self.ident("type parameter")?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBracket, "`]`")?;
        }
        Ok(out)
    }

    fn struct_decl(&mut self) -> PResult<StructDecl> {
        let start = self.advance().span;
        let name = self.ident("struct name")?;
        let generics = self.generics()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            let fname = self.ident("field name")?;
            self.expect(&TokenKind::Colon, "`:`")?;
            let ty = self.type_expr()?;
            fields.push(FieldDef { name: fname, ty });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
        Ok(StructDecl { is_pub: false, span: start.to(end), id: self.id(), name, generics, fields })
    }

    fn enum_decl(&mut self) -> PResult<EnumDecl> {
        let start = self.advance().span;
        let name = self.ident("enum name")?;
        let generics = self.generics()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            let vname = self.ident("variant name")?;
            let mut fields = Vec::new();
            let mut vspan = vname.span;
            if self.eat(&TokenKind::LParen) {
                while !self.at(&TokenKind::RParen) {
                    fields.push(self.type_expr()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                vspan = vspan.to(self.expect(&TokenKind::RParen, "`)`")?.span);
                if fields.is_empty() {
                    self.diags.push(
                        Diagnostic::error("E0203", "empty variant parentheses")
                            .with_label(vspan, "write the variant without `()`"),
                    );
                }
            }
            variants.push(VariantDef { name: vname, fields, span: vspan });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
        Ok(EnumDecl { is_pub: false, span: start.to(end), id: self.id(), name, generics, variants })
    }

    // ---- blocks ----

    fn block(&mut self) -> PResult<Block> {
        let start = self.expect(&TokenKind::LBrace, "`{`")?.span;
        let mut stmts = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            match self.stmt(false, true) {
                Ok(Some(s)) => stmts.push(s),
                Ok(None) => {}
                Err(()) => self.synchronize(),
            }
            if self.pos == before && !self.at(&TokenKind::RBrace) && !self.at_eof() {
                self.advance();
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
        Ok(Block { stmts, span: start.to(end), id: self.id() })
    }

    // ---- expressions ----

    fn expr(&mut self) -> PResult<Expr> {
        let saved = self.no_struct;
        self.no_struct = false;
        let r = self.or_expr();
        self.no_struct = saved;
        r
    }

    /// Parse an expression in a context where `Ident {` is NOT a struct literal
    /// (if/while conditions, for iterables, match scrutinees).
    fn no_struct_expr(&mut self) -> PResult<Expr> {
        let saved = self.no_struct;
        self.no_struct = true;
        let r = self.or_expr();
        self.no_struct = saved;
        r
    }

    fn or_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.and_expr()?;
        self.word_operator_hint("or", "||");
        while self.at(&TokenKind::PipePipe) {
            let op_span = self.advance().span;
            let rhs = self.and_expr()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                id: self.id(),
                kind: ExprKind::Binary { op: BinOp::Or, op_span, lhs: Box::new(lhs), rhs: Box::new(rhs) },
            };
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.equality_expr()?;
        self.word_operator_hint("and", "&&");
        while self.at(&TokenKind::AmpAmp) {
            let op_span = self.advance().span;
            let rhs = self.equality_expr()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                id: self.id(),
                kind: ExprKind::Binary { op: BinOp::And, op_span, lhs: Box::new(lhs), rhs: Box::new(rhs) },
            };
        }
        Ok(lhs)
    }

    fn equality_expr(&mut self) -> PResult<Expr> {
        let lhs = self.comparison_expr()?;
        let op = match self.peek() {
            TokenKind::EqEq => BinOp::Eq,
            TokenKind::BangEq => BinOp::Ne,
            _ => return Ok(lhs),
        };
        let op_span = self.advance().span;
        let rhs = self.comparison_expr()?;
        // Non-associative: `a == b == c` is an error.
        if matches!(self.peek(), TokenKind::EqEq | TokenKind::BangEq) {
            self.error_here("comparison operators cannot be chained; use parentheses");
            return Err(());
        }
        let span = lhs.span.to(rhs.span);
        Ok(Expr {
            span,
            id: self.id(),
            kind: ExprKind::Binary { op, op_span, lhs: Box::new(lhs), rhs: Box::new(rhs) },
        })
    }

    fn comparison_expr(&mut self) -> PResult<Expr> {
        let lhs = self.range_expr()?;
        let op = match self.peek() {
            TokenKind::Lt => BinOp::Lt,
            TokenKind::Le => BinOp::Le,
            TokenKind::Gt => BinOp::Gt,
            TokenKind::Ge => BinOp::Ge,
            _ => return Ok(lhs),
        };
        let op_span = self.advance().span;
        let rhs = self.range_expr()?;
        if matches!(self.peek(), TokenKind::Lt | TokenKind::Le | TokenKind::Gt | TokenKind::Ge) {
            self.error_here("comparison operators cannot be chained; use parentheses");
            return Err(());
        }
        let span = lhs.span.to(rhs.span);
        Ok(Expr {
            span,
            id: self.id(),
            kind: ExprKind::Binary { op, op_span, lhs: Box::new(lhs), rhs: Box::new(rhs) },
        })
    }

    fn range_expr(&mut self) -> PResult<Expr> {
        let lhs = self.bitor_expr()?;
        let inclusive = match self.peek() {
            TokenKind::DotDot => false,
            TokenKind::DotDotEq => true,
            _ => return Ok(lhs),
        };
        self.advance();
        let rhs = self.bitor_expr()?;
        let span = lhs.span.to(rhs.span);
        Ok(Expr {
            span,
            id: self.id(),
            kind: ExprKind::Range { lo: Box::new(lhs), hi: Box::new(rhs), inclusive },
        })
    }

    // Bitwise levels (v0.7), Rust's relative order: `|` < `^` < `&` < shifts,
    // all tighter than ranges/comparisons and looser than arithmetic — so
    // `x & 511 == 0` tests the mask and `1 << n - 1` shifts by `n - 1`.
    // Infix `|` is unambiguous with lambda syntax: lambdas only start in
    // prefix position (and `||` in prefix position is the empty param list).

    fn bitor_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.bitxor_expr()?;
        while self.at(&TokenKind::Pipe) {
            let op_span = self.advance().span;
            let rhs = self.bitxor_expr()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                id: self.id(),
                kind: ExprKind::Binary {
                    op: BinOp::BitOr,
                    op_span,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
        }
        Ok(lhs)
    }

    fn bitxor_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.bitand_expr()?;
        while self.at(&TokenKind::Caret) {
            let op_span = self.advance().span;
            let rhs = self.bitand_expr()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                id: self.id(),
                kind: ExprKind::Binary {
                    op: BinOp::BitXor,
                    op_span,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
        }
        Ok(lhs)
    }

    fn bitand_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.shift_expr()?;
        while self.at(&TokenKind::Amp) {
            let op_span = self.advance().span;
            let rhs = self.shift_expr()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                id: self.id(),
                kind: ExprKind::Binary {
                    op: BinOp::BitAnd,
                    op_span,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
        }
        Ok(lhs)
    }

    fn shift_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.additive_expr()?;
        loop {
            let op = match self.peek() {
                TokenKind::Shl => BinOp::Shl,
                TokenKind::Shr => BinOp::Shr,
                _ => break,
            };
            let op_span = self.advance().span;
            let rhs = self.additive_expr()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                id: self.id(),
                kind: ExprKind::Binary { op, op_span, lhs: Box::new(lhs), rhs: Box::new(rhs) },
            };
        }
        Ok(lhs)
    }

    fn additive_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.multiplicative_expr()?;
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            let op_span = self.advance().span;
            let rhs = self.multiplicative_expr()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                id: self.id(),
                kind: ExprKind::Binary { op, op_span, lhs: Box::new(lhs), rhs: Box::new(rhs) },
            };
        }
        Ok(lhs)
    }

    fn multiplicative_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.unary_expr()?;
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Rem,
                _ => break,
            };
            let op_span = self.advance().span;
            let rhs = self.unary_expr()?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                span,
                id: self.id(),
                kind: ExprKind::Binary { op, op_span, lhs: Box::new(lhs), rhs: Box::new(rhs) },
            };
        }
        Ok(lhs)
    }

    fn unary_expr(&mut self) -> PResult<Expr> {
        match self.peek() {
            TokenKind::Minus => {
                let start = self.advance().span;
                let expr = self.unary_expr()?;
                let span = start.to(expr.span);
                Ok(Expr {
                    span,
                    id: self.id(),
                    kind: ExprKind::Unary { op: UnOp::Neg, expr: Box::new(expr) },
                })
            }
            TokenKind::Bang => {
                let start = self.advance().span;
                let expr = self.unary_expr()?;
                let span = start.to(expr.span);
                Ok(Expr {
                    span,
                    id: self.id(),
                    kind: ExprKind::Unary { op: UnOp::Not, expr: Box::new(expr) },
                })
            }
            _ => self.postfix_expr(),
        }
    }

    fn postfix_expr(&mut self) -> PResult<Expr> {
        let mut expr = self.primary_expr()?;
        loop {
            match self.peek() {
                TokenKind::Dot => {
                    self.advance();
                    // Tuple index: `.0`, and the `.0.1` float-split case.
                    match self.peek().clone() {
                        TokenKind::Int(n) => {
                            let t = self.advance();
                            let span = expr.span.to(t.span);
                            expr = Expr {
                                span,
                                id: self.id(),
                                kind: ExprKind::Field {
                                    base: Box::new(expr),
                                    field: Ident { name: n.to_string(), span: t.span },
                                },
                            };
                        }
                        TokenKind::Float(_) => {
                            // `x.0.1` — the lexer produced Float("0.1"); split it.
                            let t = self.advance();
                            let raw = &self.src[t.span.start as usize..t.span.end as usize];
                            if let Some((a, b)) = raw.split_once('.') {
                                if a.chars().all(|c| c.is_ascii_digit())
                                    && b.chars().all(|c| c.is_ascii_digit())
                                {
                                    let mid = t.span.start + a.len() as u32;
                                    let s1 = Span::new(t.span.start, mid);
                                    let s2 = Span::new(mid + 1, t.span.end);
                                    let span1 = expr.span.to(s1);
                                    expr = Expr {
                                        span: span1,
                                        id: self.id(),
                                        kind: ExprKind::Field {
                                            base: Box::new(expr),
                                            field: Ident { name: a.to_string(), span: s1 },
                                        },
                                    };
                                    let span2 = expr.span.to(s2);
                                    expr = Expr {
                                        span: span2,
                                        id: self.id(),
                                        kind: ExprKind::Field {
                                            base: Box::new(expr),
                                            field: Ident { name: b.to_string(), span: s2 },
                                        },
                                    };
                                    continue;
                                }
                            }
                            self.diags.push(
                                Diagnostic::error("E0204", "invalid tuple index")
                                    .with_label(t.span, "expected a field or index after `.`"),
                            );
                            return Err(());
                        }
                        _ => {
                            let field = self.ident("field or method name")?;
                            if self.at(&TokenKind::LParen) {
                                let (args, end) = self.call_args()?;
                                let span = expr.span.to(end);
                                expr = Expr {
                                    span,
                                    id: self.id(),
                                    kind: ExprKind::MethodCall {
                                        recv: Box::new(expr),
                                        method: field,
                                        args,
                                    },
                                };
                            } else {
                                let span = expr.span.to(field.span);
                                expr = Expr {
                                    span,
                                    id: self.id(),
                                    kind: ExprKind::Field { base: Box::new(expr), field },
                                };
                            }
                        }
                    }
                }
                TokenKind::LParen => {
                    let (args, end) = self.call_args()?;
                    let span = expr.span.to(end);
                    expr = Expr {
                        span,
                        id: self.id(),
                        kind: ExprKind::Call { callee: Box::new(expr), args },
                    };
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.expr()?;
                    let end = self.expect(&TokenKind::RBracket, "`]`")?.span;
                    let span = expr.span.to(end);
                    expr = Expr {
                        span,
                        id: self.id(),
                        kind: ExprKind::Index { base: Box::new(expr), index: Box::new(index) },
                    };
                }
                TokenKind::Question => {
                    let q = self.advance().span;
                    let span = expr.span.to(q);
                    expr = Expr { span, id: self.id(), kind: ExprKind::Try(Box::new(expr)) };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn call_args(&mut self) -> PResult<(Vec<Expr>, Span)> {
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut args = Vec::new();
        while !self.at(&TokenKind::RParen) {
            args.push(self.expr()?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        let end = self.expect(&TokenKind::RParen, "`)`")?.span;
        Ok((args, end))
    }

    fn enter_nesting(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            if !self.depth_error {
                self.depth_error = true;
                let span = self.span();
                self.diags.push(
                    Diagnostic::error(
                        "E0207",
                        format!("program nesting exceeds {MAX_NESTING} levels"),
                    )
                    .with_label(span, "too deeply nested here"),
                );
            }
            self.depth -= 1;
            return Err(());
        }
        Ok(())
    }

    fn primary_expr(&mut self) -> PResult<Expr> {
        self.enter_nesting()?;
        let r = self.primary_expr_inner();
        self.depth -= 1;
        r
    }

    fn primary_expr_inner(&mut self) -> PResult<Expr> {
        let tok_span = self.span();
        match self.peek().clone() {
            TokenKind::Int(v) => {
                self.advance();
                Ok(Expr { span: tok_span, id: self.id(), kind: ExprKind::Int(v) })
            }
            TokenKind::Float(v) => {
                self.advance();
                Ok(Expr { span: tok_span, id: self.id(), kind: ExprKind::Float(v) })
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr { span: tok_span, id: self.id(), kind: ExprKind::Bool(true) })
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr { span: tok_span, id: self.id(), kind: ExprKind::Bool(false) })
            }
            TokenKind::Str(s) => {
                self.advance();
                Ok(Expr { span: tok_span, id: self.id(), kind: ExprKind::Str(s) })
            }
            TokenKind::StrInterpStart(first) => {
                self.advance();
                let mut parts = vec![first];
                let mut exprs = Vec::new();
                loop {
                    exprs.push(self.expr()?);
                    match self.peek().clone() {
                        TokenKind::StrInterpMid(s) => {
                            self.advance();
                            parts.push(s);
                        }
                        TokenKind::StrInterpEnd(s) => {
                            let end = self.advance().span;
                            parts.push(s);
                            return Ok(Expr {
                                span: tok_span.to(end),
                                id: self.id(),
                                kind: ExprKind::StringInterp { parts, exprs },
                            });
                        }
                        _ => {
                            self.error_here("expected end of string interpolation");
                            return Err(());
                        }
                    }
                }
            }
            TokenKind::Ident(name) => {
                self.advance();
                // Struct literal `Name { ... }` (allowed outside no-struct contexts).
                if self.at(&TokenKind::LBrace) && !self.no_struct && self.looks_like_struct_lit() {
                    let name = Ident { name, span: tok_span };
                    return self.struct_lit(name);
                }
                // Qualified struct literal `alias.Name { ... }`: the dotted
                // name is stored joined ("alias.Name"); the checker splits it.
                if self.at(&TokenKind::Dot)
                    && matches!(self.peek2(), TokenKind::Ident(_))
                    && matches!(
                        self.tokens[(self.pos + 2).min(self.tokens.len() - 1)].kind,
                        TokenKind::LBrace
                    )
                    && !self.no_struct
                {
                    let save = self.pos;
                    self.advance(); // `.`
                    let second = self.ident("type name")?;
                    if self.looks_like_struct_lit() {
                        let name = Ident {
                            name: format!("{name}.{}", second.name),
                            span: tok_span.to(second.span),
                        };
                        return self.struct_lit(name);
                    }
                    self.pos = save;
                }
                Ok(Expr { span: tok_span, id: self.id(), kind: ExprKind::Var(name) })
            }
            TokenKind::LParen => {
                self.advance();
                if self.eat(&TokenKind::RParen) {
                    let span = tok_span.to(self.prev_span());
                    return Ok(Expr { span, id: self.id(), kind: ExprKind::Unit });
                }
                let first = self.expr()?;
                if self.eat(&TokenKind::Comma) {
                    let mut items = vec![first];
                    while !self.at(&TokenKind::RParen) {
                        items.push(self.expr()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    let end = self.expect(&TokenKind::RParen, "`)`")?.span;
                    if items.len() == 1 {
                        // `(x,)` — not a tuple in Socrates.
                        self.diags.push(
                            Diagnostic::error("E0205", "tuples need at least two elements")
                                .with_label(tok_span.to(end), ""),
                        );
                        return Ok(items.pop().unwrap());
                    }
                    return Ok(Expr {
                        span: tok_span.to(end),
                        id: self.id(),
                        kind: ExprKind::Tuple(items),
                    });
                }
                self.expect(&TokenKind::RParen, "`)`")?;
                Ok(first)
            }
            TokenKind::LBracket => {
                self.advance();
                let mut items = Vec::new();
                while !self.at(&TokenKind::RBracket) {
                    items.push(self.expr()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                let end = self.expect(&TokenKind::RBracket, "`]`")?.span;
                Ok(Expr { span: tok_span.to(end), id: self.id(), kind: ExprKind::List(items) })
            }
            TokenKind::LBrace => self.brace_expr(),
            TokenKind::Pipe => self.lambda(false),
            TokenKind::PipePipe => self.lambda(true),
            TokenKind::If => self.if_expr(),
            TokenKind::Match => self.match_expr(),
            other => {
                self.error_here(format!("expected an expression, found {}", other.describe()));
                Err(())
            }
        }
    }

    /// After `Ident` with `{` next: decide struct literal vs. something else.
    /// `Name {}` and `Name { ident: ...` / `Name { ident,` / `Name { ident }`
    /// are struct literals. This keeps `match x { arm -> ... }`-style code
    /// working when `x {` would be ambiguous (we only call this when not in a
    /// no-struct context, so a plain heuristic suffices).
    fn looks_like_struct_lit(&self) -> bool {
        debug_assert!(self.at(&TokenKind::LBrace));
        match self.peek2() {
            TokenKind::RBrace => true,
            TokenKind::Ident(_) => {
                let third = &self.tokens[(self.pos + 2).min(self.tokens.len() - 1)].kind;
                matches!(third, TokenKind::Colon | TokenKind::Comma | TokenKind::RBrace)
            }
            _ => false,
        }
    }

    fn struct_lit(&mut self, name: Ident) -> PResult<Expr> {
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            let fname = self.ident("field name")?;
            let value = if self.eat(&TokenKind::Colon) {
                self.expr()?
            } else {
                // Shorthand `Point { x, y }` — field takes the variable of the same name.
                Expr {
                    span: fname.span,
                    id: self.id(),
                    kind: ExprKind::Var(fname.name.clone()),
                }
            };
            fields.push((fname, value));
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
        let span = name.span.to(end);
        Ok(Expr { span, id: self.id(), kind: ExprKind::StructLit { name, fields } })
    }

    /// `{` in expression position: block, map literal, or `{:}` (empty map).
    fn brace_expr(&mut self) -> PResult<Expr> {
        let start = self.span();
        // `{:}` — empty map.
        if matches!(self.peek2(), TokenKind::Colon) {
            let third = &self.tokens[(self.pos + 2).min(self.tokens.len() - 1)].kind;
            if matches!(third, TokenKind::RBrace) {
                self.advance(); // {
                self.advance(); // :
                let end = self.advance().span; // }
                return Ok(Expr {
                    span: start.to(end),
                    id: self.id(),
                    kind: ExprKind::MapLit(Vec::new()),
                });
            }
        }

        // Speculative map-literal parse: `{ expr :` commits to a map. The
        // decision is memoized per `{` token — without this, speculation over
        // nested blocks re-parses subtrees exponentially.
        let brace_pos = self.pos;
        let is_map = match self.brace_memo.get(&brace_pos) {
            Some(&v) => v,
            None => {
                let save_pos = self.pos;
                let save_diags = self.diags.len();
                let save_ids = self.next_id;
                let save_depth_err = self.depth_error;
                self.advance(); // `{`
                let is_map = match self.expr() {
                    Ok(_key) => self.at(&TokenKind::Colon),
                    Err(()) => false,
                };
                // Roll back regardless; re-parse cleanly on the chosen branch.
                self.pos = save_pos;
                self.diags.truncate(save_diags);
                self.next_id = save_ids;
                self.depth_error = save_depth_err;
                self.brace_memo.insert(brace_pos, is_map);
                is_map
            }
        };

        if is_map {
            self.advance(); // `{`
            let mut entries = Vec::new();
            while !self.at(&TokenKind::RBrace) {
                let key = self.expr()?;
                self.expect(&TokenKind::Colon, "`:` in map literal")?;
                let value = self.expr()?;
                entries.push((key, value));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
            Ok(Expr { span: start.to(end), id: self.id(), kind: ExprKind::MapLit(entries) })
        } else {
            let block = self.block()?;
            let span = block.span;
            Ok(Expr { span, id: self.id(), kind: ExprKind::Block(block) })
        }
    }

    fn lambda(&mut self, empty_params: bool) -> PResult<Expr> {
        let start = self.span();
        let mut params = Vec::new();
        if empty_params {
            self.advance(); // `||`
        } else {
            self.advance(); // `|`
            while !self.at(&TokenKind::Pipe) {
                // `_` discards the argument: it binds nothing and cannot be
                // referenced (the lexer never produces `_` as an expression).
                let name = if self.at(&TokenKind::Underscore) {
                    let span = self.advance().span;
                    Ident { name: "_".to_string(), span }
                } else {
                    self.ident("lambda parameter")?
                };
                let ty = if self.eat(&TokenKind::Colon) { Some(self.type_expr()?) } else { None };
                params.push(LambdaParam { name, ty, id: self.id() });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::Pipe, "`|` to close lambda parameters")?;
        }

        let ret = if self.eat(&TokenKind::Arrow) { Some(self.type_expr()?) } else { None };
        let body = if ret.is_some() {
            // With an explicit return type, a block body is required.
            let block = self.block()?;
            let span = block.span;
            Expr { span, id: self.id(), kind: ExprKind::Block(block) }
        } else {
            self.expr()?
        };
        let span = start.to(body.span);
        Ok(Expr {
            span,
            id: self.id(),
            kind: ExprKind::Lambda { params, ret, body: Box::new(body) },
        })
    }

    fn if_expr(&mut self) -> PResult<Expr> {
        let start = self.advance().span; // `if`
        if self.at(&TokenKind::Let) {
            return self.if_let_expr(start);
        }
        let cond = self.no_struct_expr()?;
        let then = self.block()?;
        let mut span = start.to(then.span);
        let els = if self.eat(&TokenKind::Else) {
            let e = if self.at(&TokenKind::If) {
                self.if_expr()?
            } else {
                let b = self.block()?;
                let bspan = b.span;
                Expr { span: bspan, id: self.id(), kind: ExprKind::Block(b) }
            };
            span = span.to(e.span);
            Some(Box::new(e))
        } else {
            None
        };
        Ok(Expr {
            span,
            id: self.id(),
            kind: ExprKind::If { cond: Box::new(cond), then, els },
        })
    }

    /// `if let PATTERN = EXPR { THEN } [else ...]` (v0.8) — sugar for
    /// `match EXPR { PATTERN -> THEN, _ -> ELSE-or-Unit }`. Desugars fully
    /// here into an ordinary two-arm `Match`: with no `else`, the fallback
    /// arm's body is a literal `Unit`, which — via the same arm-type
    /// unification every `Match` already does — forces THEN to be
    /// Unit-typed too, exactly like a plain `if` with no `else`. An
    /// `else if` chains through a recursive `if_expr` call, exactly as
    /// plain `if`/`else if` already does.
    fn if_let_expr(&mut self, start: Span) -> PResult<Expr> {
        self.advance(); // `let`
        let pattern = self.pattern()?;
        self.expect(&TokenKind::Eq, "`=` after `if let` pattern")?;
        let scrutinee = self.no_struct_expr()?;
        let then = self.block()?;
        let then_span = then.span;
        let mut span = start.to(then_span);
        let fallback_body = if self.eat(&TokenKind::Else) {
            let e = if self.at(&TokenKind::If) {
                self.if_expr()?
            } else {
                let b = self.block()?;
                let bspan = b.span;
                Expr { span: bspan, id: self.id(), kind: ExprKind::Block(b) }
            };
            span = span.to(e.span);
            e
        } else {
            Expr { span: then_span, id: self.id(), kind: ExprKind::Unit }
        };
        let user_arm = MatchArm {
            pattern,
            guard: None,
            body: Expr { span: then_span, id: self.id(), kind: ExprKind::Block(then) },
            span,
            sugar: false,
        };
        let fallback_span = fallback_body.span;
        let fallback_arm = MatchArm {
            pattern: Pattern { kind: PatternKind::Wildcard, span: fallback_span, id: self.id() },
            guard: None,
            body: fallback_body,
            span: fallback_span,
            sugar: false,
        };
        Ok(Expr {
            span,
            id: self.id(),
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: vec![user_arm, fallback_arm],
                sugar: MatchSugar::IfLet,
            },
        })
    }

    fn match_expr(&mut self) -> PResult<Expr> {
        let start = self.advance().span; // `match`
        let scrutinee = self.no_struct_expr()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let pat_start = self.span();
            let pattern = self.pattern()?;
            let guard = if self.eat(&TokenKind::If) { Some(self.no_struct_expr()?) } else { None };
            self.expect(&TokenKind::Arrow, "`->` after match pattern")?;
            let (body, sugar) = self.arm_body()?;
            let arm_span = pat_start.to(body.span);
            let body_is_block = matches!(
                body.kind,
                ExprKind::Block(_) | ExprKind::If { .. } | ExprKind::Match { .. }
            );
            arms.push(MatchArm { pattern, guard, body, span: arm_span, sugar });
            if !self.eat(&TokenKind::Comma) {
                // Allow omitting the comma after a block-bodied arm.
                if !body_is_block {
                    break;
                }
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}` to close `match`")?.span;
        if arms.is_empty() {
            self.diags.push(
                Diagnostic::error("E0206", "`match` must have at least one arm")
                    .with_label(start.to(end), ""),
            );
        }
        Ok(Expr {
            span: start.to(end),
            id: self.id(),
            kind: ExprKind::Match { scrutinee: Box::new(scrutinee), arms, sugar: MatchSugar::None },
        })
    }

    /// A match-arm body: an expression, or — as sugar for the block form —
    /// a bare `return [expr]`, `break`, or `continue`, which parses as if
    /// written `{ return expr; }`. The `bool` reports whether the sugar was
    /// used, so the formatter can print it back.
    fn arm_body(&mut self) -> PResult<(Expr, bool)> {
        let start = self.span();
        let stmt = match self.peek() {
            TokenKind::Return => {
                self.advance();
                let value = if self.at(&TokenKind::Comma) || self.at(&TokenKind::RBrace) {
                    None
                } else {
                    let v = self.expr()?;
                    // A valueless bare `return` with the comma omitted would
                    // have swallowed the NEXT arm's pattern as its value; the
                    // `->` right after the "value" gives it away.
                    if self.at(&TokenKind::Arrow) {
                        self.diags.push(
                            Diagnostic::error(
                                "E0200",
                                "this parsed as the `return` value, but it looks like the next arm",
                            )
                            .with_label(v.span, "consumed as the returned value")
                            .with_note(
                                "a bare `return` with no value needs a comma before the next arm: `-> return,`",
                            ),
                        );
                        return Err(());
                    }
                    Some(v)
                };
                let end = value.as_ref().map_or(start, |v| v.span);
                Some(Stmt { span: start.to(end), id: self.id(), kind: StmtKind::Return(value) })
            }
            TokenKind::Break => {
                self.advance();
                Some(Stmt { span: start, id: self.id(), kind: StmtKind::Break })
            }
            TokenKind::Continue => {
                self.advance();
                Some(Stmt { span: start, id: self.id(), kind: StmtKind::Continue })
            }
            _ => None,
        };
        if let Some(stmt) = stmt {
            let span = stmt.span;
            let block = Block { stmts: vec![stmt], span, id: self.id() };
            return Ok((Expr { span, id: self.id(), kind: ExprKind::Block(block) }, true));
        }
        let body = self.expr()?;
        // A statement-shaped arm (`Some(v) -> x = v`, `.. -> n += v`) is a
        // common trip-up; point at the fix instead of a bare "expected `,`".
        if matches!(
            self.peek(),
            TokenKind::Eq
                | TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::StarEq
                | TokenKind::SlashEq
                | TokenKind::PercentEq
                | TokenKind::AmpEq
                | TokenKind::PipeEq
                | TokenKind::CaretEq
                | TokenKind::ShlEq
                | TokenKind::ShrEq
        ) {
            let span = self.span();
            self.diags.push(
                Diagnostic::error("E0200", "assignment cannot be a match-arm body")
                    .with_label(span, "assignment is a statement, not an expression")
                    .with_note("wrap the arm body in a block: `pattern -> { place = value; }`"),
            );
            return Err(());
        }
        Ok((body, false))
    }

    // ---- patterns ----

    fn pattern(&mut self) -> PResult<Pattern> {
        let first = self.base_pattern()?;
        if !self.at(&TokenKind::Pipe) {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.eat(&TokenKind::Pipe) {
            alts.push(self.base_pattern()?);
        }
        let span = alts.first().unwrap().span.to(alts.last().unwrap().span);
        Ok(Pattern { span, id: self.id(), kind: PatternKind::Or(alts) })
    }

    fn base_pattern(&mut self) -> PResult<Pattern> {
        self.enter_nesting()?;
        let r = self.base_pattern_inner();
        self.depth -= 1;
        r
    }

    fn base_pattern_inner(&mut self) -> PResult<Pattern> {
        let tok_span = self.span();
        match self.peek().clone() {
            TokenKind::Underscore => {
                self.advance();
                Ok(Pattern { span: tok_span, id: self.id(), kind: PatternKind::Wildcard })
            }
            TokenKind::Int(v) => {
                self.advance();
                Ok(Pattern { span: tok_span, id: self.id(), kind: PatternKind::Int(v) })
            }
            TokenKind::Float(v) => {
                self.advance();
                Ok(Pattern { span: tok_span, id: self.id(), kind: PatternKind::Float(v) })
            }
            TokenKind::Minus => {
                self.advance();
                match self.peek().clone() {
                    TokenKind::Int(v) => {
                        let end = self.advance().span;
                        Ok(Pattern {
                            span: tok_span.to(end),
                            id: self.id(),
                            kind: PatternKind::Int(v.checked_neg().unwrap_or(i64::MIN)),
                        })
                    }
                    TokenKind::Float(v) => {
                        let end = self.advance().span;
                        Ok(Pattern {
                            span: tok_span.to(end),
                            id: self.id(),
                            kind: PatternKind::Float(-v),
                        })
                    }
                    _ => {
                        self.error_here("expected a number after `-` in pattern");
                        Err(())
                    }
                }
            }
            TokenKind::True => {
                self.advance();
                Ok(Pattern { span: tok_span, id: self.id(), kind: PatternKind::Bool(true) })
            }
            TokenKind::False => {
                self.advance();
                Ok(Pattern { span: tok_span, id: self.id(), kind: PatternKind::Bool(false) })
            }
            TokenKind::Str(s) => {
                self.advance();
                Ok(Pattern { span: tok_span, id: self.id(), kind: PatternKind::Str(s) })
            }
            TokenKind::StrInterpStart(_) => {
                self.error_here("interpolated strings cannot be used as patterns");
                Err(())
            }
            TokenKind::LParen => {
                self.advance();
                if self.eat(&TokenKind::RParen) {
                    let span = tok_span.to(self.prev_span());
                    return Ok(Pattern { span, id: self.id(), kind: PatternKind::Unit });
                }
                let mut items = vec![self.pattern()?];
                while self.eat(&TokenKind::Comma) {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    items.push(self.pattern()?);
                }
                let end = self.expect(&TokenKind::RParen, "`)`")?.span;
                if items.len() == 1 {
                    // Parenthesized pattern.
                    return Ok(items.pop().unwrap());
                }
                Ok(Pattern {
                    span: tok_span.to(end),
                    id: self.id(),
                    kind: PatternKind::Tuple(items),
                })
            }
            TokenKind::Ident(name) => {
                self.advance();
                let head = Ident { name, span: tok_span };

                // Qualified variant: `Enum.Variant` / `Enum.Variant(...)`,
                // or module-qualified `alias.Enum.Variant(...)` (the enum
                // name is stored joined: "alias.Enum"). A module-qualified
                // struct pattern `alias.Type { .. }` is also caught here.
                if self.at(&TokenKind::Dot) {
                    self.advance();
                    let mut enum_name = head;
                    let mut variant = self.ident("variant name")?;
                    if self.at(&TokenKind::Dot) {
                        self.advance();
                        enum_name = Ident {
                            name: format!("{}.{}", enum_name.name, variant.name),
                            span: enum_name.span.to(variant.span),
                        };
                        variant = self.ident("variant name")?;
                    } else if self.at(&TokenKind::LBrace) {
                        let name = Ident {
                            name: format!("{}.{}", enum_name.name, variant.name),
                            span: enum_name.span.to(variant.span),
                        };
                        return self.struct_pattern(name, tok_span);
                    }
                    let (fields, has_parens, end) = self.variant_fields(variant.span)?;
                    return Ok(Pattern {
                        span: tok_span.to(end),
                        id: self.id(),
                        kind: PatternKind::Variant {
                            enum_name: Some(enum_name),
                            variant,
                            fields,
                            has_parens,
                        },
                    });
                }
                // Unqualified variant with payload: `Some(x)`.
                if self.at(&TokenKind::LParen) {
                    let variant = head;
                    let (fields, has_parens, end) = self.variant_fields(variant.span)?;
                    return Ok(Pattern {
                        span: tok_span.to(end),
                        id: self.id(),
                        kind: PatternKind::Variant { enum_name: None, variant, fields, has_parens },
                    });
                }
                // Struct pattern: `Point { ... }`.
                if self.at(&TokenKind::LBrace) {
                    return self.struct_pattern(head, tok_span);
                }
                // Plain binding (the checker may reinterpret `None` etc. as a
                // nullary variant).
                Ok(Pattern {
                    span: tok_span,
                    id: self.id(),
                    kind: PatternKind::Binding(head.name),
                })
            }
            other => {
                self.error_here(format!("expected a pattern, found {}", other.describe()));
                Err(())
            }
        }
    }

    /// The body of a struct pattern, after its (possibly module-qualified)
    /// name; the cursor is at `{`.
    fn struct_pattern(&mut self, name: Ident, start: Span) -> PResult<Pattern> {
        self.advance(); // `{`
        let mut fields = Vec::new();
        let mut rest = false;
        while !self.at(&TokenKind::RBrace) {
            if self.eat(&TokenKind::DotDot) {
                rest = true;
                break;
            }
            let fname = self.ident("field name")?;
            let pat = if self.eat(&TokenKind::Colon) {
                self.pattern()?
            } else {
                Pattern {
                    span: fname.span,
                    id: self.id(),
                    kind: PatternKind::Binding(fname.name.clone()),
                }
            };
            fields.push((fname, pat));
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
        Ok(Pattern {
            span: start.to(end),
            id: self.id(),
            kind: PatternKind::Struct { name, fields, rest },
        })
    }

    fn variant_fields(&mut self, name_span: Span) -> PResult<(Vec<Pattern>, bool, Span)> {
        if !self.at(&TokenKind::LParen) {
            return Ok((Vec::new(), false, name_span));
        }
        self.advance();
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RParen) {
            fields.push(self.pattern()?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        let end = self.expect(&TokenKind::RParen, "`)`")?.span;
        Ok((fields, true, end))
    }

    // ---- types ----

    fn type_expr(&mut self) -> PResult<TypeExpr> {
        self.enter_nesting()?;
        let r = self.type_expr_inner();
        self.depth -= 1;
        r
    }

    fn type_expr_inner(&mut self) -> PResult<TypeExpr> {
        let tok_span = self.span();
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                self.advance();
                let mut name = Ident { name, span: tok_span };
                // Qualified type from an imported module: `alias.Type`. Stored
                // joined ("alias.Type"); the checker splits it.
                if self.at(&TokenKind::Dot) && matches!(self.peek2(), TokenKind::Ident(_)) {
                    self.advance();
                    let second = self.ident("type name")?;
                    name = Ident {
                        name: format!("{}.{}", name.name, second.name),
                        span: name.span.to(second.span),
                    };
                }
                let mut args = Vec::new();
                let mut span = tok_span;
                if self.eat(&TokenKind::LBracket) {
                    while !self.at(&TokenKind::RBracket) {
                        args.push(self.type_expr()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    span = span.to(self.expect(&TokenKind::RBracket, "`]`")?.span);
                }
                Ok(TypeExpr { span, id: self.id(), kind: TypeExprKind::Named { name, args } })
            }
            TokenKind::LParen => {
                self.advance();
                if self.eat(&TokenKind::RParen) {
                    let span = tok_span.to(self.prev_span());
                    return Ok(TypeExpr { span, id: self.id(), kind: TypeExprKind::Unit });
                }
                let mut items = vec![self.type_expr()?];
                while self.eat(&TokenKind::Comma) {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    items.push(self.type_expr()?);
                }
                let end = self.expect(&TokenKind::RParen, "`)`")?.span;
                if items.len() == 1 {
                    return Ok(items.pop().unwrap());
                }
                Ok(TypeExpr {
                    span: tok_span.to(end),
                    id: self.id(),
                    kind: TypeExprKind::Tuple(items),
                })
            }
            TokenKind::Fn => {
                self.advance();
                self.expect(&TokenKind::LParen, "`(`")?;
                let mut params = Vec::new();
                while !self.at(&TokenKind::RParen) {
                    params.push(self.type_expr()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                let mut span = tok_span.to(self.expect(&TokenKind::RParen, "`)`")?.span);
                let ret = if self.eat(&TokenKind::Arrow) {
                    let t = self.type_expr()?;
                    span = span.to(t.span);
                    Some(Box::new(t))
                } else {
                    None
                };
                Ok(TypeExpr { span, id: self.id(), kind: TypeExprKind::Fn { params, ret } })
            }
            other => {
                self.error_here(format!("expected a type, found {}", other.describe()));
                Err(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parse_ok(src: &str) -> Program {
        let lexed = lex(src);
        assert!(lexed.diags.is_empty(), "lex errors: {:?}", lexed.diags);
        let out = parse(lexed.tokens, src);
        assert!(out.diags.is_empty(), "parse errors in {src:?}: {:?}", out.diags);
        out.program
    }

    fn parse_err(src: &str) -> Vec<Diagnostic> {
        let lexed = lex(src);
        let out = parse(lexed.tokens, src);
        assert!(!out.diags.is_empty(), "expected parse errors for {src:?}");
        out.diags
    }

    #[test]
    fn simple_program() {
        let p = parse_ok("let x = 1 + 2 * 3;\nprintln(x);");
        assert_eq!(p.stmts.len(), 2);
    }

    #[test]
    fn precedence_shape() {
        // 1 + 2 * 3 parses as 1 + (2 * 3)
        let p = parse_ok("let x = 1 + 2 * 3;");
        let StmtKind::Let { init, .. } = &p.stmts[0].kind else { panic!() };
        let ExprKind::Binary { op: BinOp::Add, rhs, .. } = &init.kind else {
            panic!("expected Add at root, got {:?}", init.kind)
        };
        assert!(matches!(rhs.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
    }

    #[test]
    fn functions_and_generics() {
        parse_ok("fn map[T, U](xs: List[T], f: fn(T) -> U) -> List[U] { xs }");
        parse_ok("fn no_ret(x: Int) { println(x); }");
    }

    #[test]
    fn structs_and_enums() {
        parse_ok("struct Point { x: Float, y: Float }");
        parse_ok("enum Shape { Circle(Float), Rect(Float, Float), Empty }");
        parse_ok("struct Pair[A, B] { first: A, second: B }");
    }

    #[test]
    fn struct_literal_and_field_access() {
        parse_ok("let p = Point { x: 1.0, y: 2.0 }; let q = p.x;");
        parse_ok("let p = Point { x, y }; p.x = 3.0;");
    }

    #[test]
    fn no_struct_in_condition() {
        // `x { }` after `if` must not parse as struct literal.
        parse_ok("if x { println(1); } else { println(2); }");
        parse_ok("while running { tick(); }");
        parse_ok("for i in xs { println(i); }");
        // But parenthesized struct literals in conditions are fine.
        parse_ok("if (Point { x: 1.0, y: 2.0 }).x > 0.0 { println(1); }");
    }

    #[test]
    fn match_arms() {
        parse_ok(
            r#"
            match s {
                Shape.Circle(r) if r > 1.0 -> "big",
                Shape.Circle(r) -> "small",
                Shape.Rect(w, h) -> "rect",
                Shape.Empty -> "empty",
            }
            "#,
        );
        parse_ok("match x { 0 | 1 | 2 -> \"low\", _ -> \"high\" }");
        parse_ok("match p { Point { x, y } -> x + y }");
        parse_ok("match t { (a, b) -> a + b }");
        parse_ok("match o { Some(v) -> v, None -> 0 }");
    }

    #[test]
    fn lambdas() {
        parse_ok("let f = |x| x * 2;");
        parse_ok("let f = |x: Int, y: Int| x + y;");
        parse_ok("let f = || 42;");
        parse_ok("let f = |x: Int| -> Int { x + 1 };");
        parse_ok("xs.map(|n| n * n);");
    }

    #[test]
    fn map_literal_vs_block() {
        let p = parse_ok("let m = {\"a\": 1, \"b\": 2};");
        let StmtKind::Let { init, .. } = &p.stmts[0].kind else { panic!() };
        assert!(matches!(init.kind, ExprKind::MapLit(ref v) if v.len() == 2));

        let p = parse_ok("let m = {:};");
        let StmtKind::Let { init, .. } = &p.stmts[0].kind else { panic!() };
        assert!(matches!(init.kind, ExprKind::MapLit(ref v) if v.is_empty()));

        let p = parse_ok("let b = { let y = 1; y + 1 };");
        let StmtKind::Let { init, .. } = &p.stmts[0].kind else { panic!() };
        assert!(matches!(init.kind, ExprKind::Block(_)));
    }

    #[test]
    fn ranges() {
        parse_ok("for i in 0..10 { println(i); }");
        parse_ok("let r = 1..=5;");
    }

    #[test]
    fn tuple_index_chain() {
        // `.0.1` must split the float token.
        parse_ok("let x = pair.0;");
        parse_ok("let x = nested.0.1;");
    }

    #[test]
    fn string_interp_expr() {
        let p = parse_ok(r#"let s = "sum = {a + b}!";"#);
        let StmtKind::Let { init, .. } = &p.stmts[0].kind else { panic!() };
        let ExprKind::StringInterp { parts, exprs } = &init.kind else { panic!() };
        assert_eq!(parts.len(), 2);
        assert_eq!(exprs.len(), 1);
    }

    #[test]
    fn errors() {
        parse_err("let x = ;");
        parse_err("fn f(x) {}"); // missing param type
        parse_err("let x = 1 < 2 < 3;"); // chained comparison
        parse_err("match x { }"); // no arms
        parse_err("{ fn g() {} }"); // nested fn
    }

    #[test]
    fn destructuring_let() {
        parse_ok("let (a, b) = pair;");
        parse_ok("let Point { x, y } = p;");
    }

    #[test]
    fn compound_assign() {
        parse_ok("x += 1; y[0] *= 2; p.x -= 0.5;");
    }
}
