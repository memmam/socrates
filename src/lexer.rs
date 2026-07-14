//! The lexer: source text → tokens.
//!
//! Interpolated strings are handled with a mode stack. `"a {x} b {y} c"` lexes as
//! `StrInterpStart("a ")`, `Ident(x)`, `StrInterpMid(" b ")`, `Ident(y)`,
//! `StrInterpEnd(" c")`. Interpolations nest (a string inside an interpolation may
//! itself interpolate), and `{`/`}` inside an interpolation are depth-tracked so map
//! literals and blocks work inside `{ }`.

use crate::diag::Diagnostic;
use crate::span::Span;
use crate::token::{Comment, Token, TokenKind};

pub struct LexOutput {
    pub tokens: Vec<Token>,
    pub comments: Vec<Comment>,
    pub diags: Vec<Diagnostic>,
}

pub fn lex(text: &str) -> LexOutput {
    let mut lx = Lexer {
        text,
        bytes: text.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
        comments: Vec::new(),
        diags: Vec::new(),
        modes: Vec::new(),
    };
    lx.run();
    LexOutput { tokens: lx.tokens, comments: lx.comments, diags: lx.diags }
}

/// Lexer mode: inside an interpolation hole, we count braces so `}` only closes
/// the hole at depth zero.
struct InterpMode {
    brace_depth: u32,
}

struct Lexer<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
    comments: Vec<Comment>,
    diags: Vec<Diagnostic>,
    modes: Vec<InterpMode>,
}

impl<'a> Lexer<'a> {
    fn run(&mut self) {
        loop {
            self.skip_trivia();
            let start = self.pos;
            let Some(c) = self.peek_char() else {
                if !self.modes.is_empty() {
                    self.error_at(
                        Span::new(start as u32, start as u32),
                        "E0103",
                        "unterminated string interpolation",
                    );
                }
                self.push(TokenKind::Eof, start, self.pos);
                return;
            };

            match c {
                '0'..='9' => self.number(),
                'a'..='z' | 'A'..='Z' | '_' => self.ident_or_keyword(),
                '"' => self.string(),
                _ => self.operator(c),
            }
        }
    }

    // ---- basic cursor ops ----

    fn peek_char(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    fn peek_byte(&self, off: usize) -> u8 {
        *self.bytes.get(self.pos + off).unwrap_or(&0)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek_char()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek_byte(0) == b {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.tokens.push(Token { kind, span: Span::new(start as u32, end as u32) });
    }

    fn error_at(&mut self, span: Span, code: &'static str, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(code, msg).with_label(span, ""));
    }

    // ---- trivia ----

    fn skip_trivia(&mut self) {
        loop {
            match self.peek_byte(0) {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.pos += 1;
                }
                b'/' if self.peek_byte(1) == b'/' => {
                    let start = self.pos;
                    while self.pos < self.bytes.len() && self.peek_byte(0) != b'\n' {
                        self.pos += 1;
                    }
                    self.comments.push(Comment {
                        text: self.text[start..self.pos].to_string(),
                        span: Span::new(start as u32, self.pos as u32),
                        block: false,
                    });
                }
                b'/' if self.peek_byte(1) == b'*' => {
                    let start = self.pos;
                    self.pos += 2;
                    let mut depth = 1u32;
                    while depth > 0 {
                        if self.pos >= self.bytes.len() {
                            self.error_at(
                                Span::new(start as u32, self.pos as u32),
                                "E0101",
                                "unterminated block comment",
                            );
                            break;
                        }
                        if self.peek_byte(0) == b'/' && self.peek_byte(1) == b'*' {
                            depth += 1;
                            self.pos += 2;
                        } else if self.peek_byte(0) == b'*' && self.peek_byte(1) == b'/' {
                            depth -= 1;
                            self.pos += 2;
                        } else {
                            self.bump();
                        }
                    }
                    self.comments.push(Comment {
                        text: self.text[start..self.pos].to_string(),
                        span: Span::new(start as u32, self.pos as u32),
                        block: true,
                    });
                }
                _ => return,
            }
        }
    }

    // ---- numbers ----

    fn number(&mut self) {
        let start = self.pos;
        if self.peek_byte(0) == b'0' && (self.peek_byte(1) == b'x' || self.peek_byte(1) == b'X') {
            self.pos += 2;
            return self.radix_number(start, 16);
        }
        if self.peek_byte(0) == b'0' && (self.peek_byte(1) == b'b' || self.peek_byte(1) == b'B') {
            self.pos += 2;
            return self.radix_number(start, 2);
        }

        self.digits();
        let mut is_float = false;
        // A `.` makes a float only if followed by a digit — `1..5` must stay Int,
        // `x.0` handled by the parser (this is `1` `.` `0` only when... no:
        // tuple access appears after expressions, never directly after a number,
        // so `1.0` is always a float literal and `xs.0` never reaches here).
        if self.peek_byte(0) == b'.' && self.peek_byte(1).is_ascii_digit() {
            is_float = true;
            self.pos += 1;
            self.digits();
        }
        if matches!(self.peek_byte(0), b'e' | b'E') {
            let mut look = 1;
            if matches!(self.peek_byte(1), b'+' | b'-') {
                look = 2;
            }
            if self.peek_byte(look).is_ascii_digit() {
                is_float = true;
                self.pos += look;
                self.digits();
            }
        }

        let raw: String = self.text[start..self.pos].chars().filter(|&c| c != '_').collect();
        let span = Span::new(start as u32, self.pos as u32);
        if is_float {
            match raw.parse::<f64>() {
                Ok(f) => self.push(TokenKind::Float(f), start, self.pos),
                Err(_) => {
                    self.error_at(span, "E0104", format!("invalid float literal `{raw}`"));
                    self.push(TokenKind::Float(0.0), start, self.pos);
                }
            }
        } else {
            match raw.parse::<i64>() {
                Ok(i) => self.push(TokenKind::Int(i), start, self.pos),
                Err(_) => {
                    self.error_at(
                        span,
                        "E0105",
                        format!("integer literal `{raw}` does not fit in 64 bits"),
                    );
                    self.push(TokenKind::Int(0), start, self.pos);
                }
            }
        }
    }

    fn digits(&mut self) {
        while self.peek_byte(0).is_ascii_digit() || self.peek_byte(0) == b'_' {
            self.pos += 1;
        }
    }

    fn radix_number(&mut self, start: usize, radix: u32) {
        let digits_start = self.pos;
        while self.peek_byte(0).is_ascii_alphanumeric() || self.peek_byte(0) == b'_' {
            self.pos += 1;
        }
        let raw: String =
            self.text[digits_start..self.pos].chars().filter(|&c| c != '_').collect();
        let span = Span::new(start as u32, self.pos as u32);
        match i64::from_str_radix(&raw, radix) {
            Ok(i) => self.push(TokenKind::Int(i), start, self.pos),
            Err(_) => {
                let base = if radix == 16 { "hex" } else { "binary" };
                self.error_at(span, "E0105", format!("invalid {base} integer literal"));
                self.push(TokenKind::Int(0), start, self.pos);
            }
        }
    }

    // ---- identifiers ----

    fn ident_or_keyword(&mut self) {
        let start = self.pos;
        while matches!(self.peek_byte(0), b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_') {
            self.pos += 1;
        }
        let word = &self.text[start..self.pos];
        let kind = match word {
            "let" => TokenKind::Let,
            "mut" => TokenKind::Mut,
            "fn" => TokenKind::Fn,
            "struct" => TokenKind::Struct,
            "enum" => TokenKind::Enum,
            "impl" => TokenKind::Impl,
            "import" => TokenKind::Import,
            "pub" => TokenKind::Pub,
            "match" => TokenKind::Match,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "_" => TokenKind::Underscore,
            _ => TokenKind::Ident(word.to_string()),
        };
        self.push(kind, start, self.pos);
    }

    // ---- strings ----

    /// Lex a string starting at the opening `"`. Emits `Str` when there are no
    /// interpolation holes, otherwise `StrInterpStart` and enters interp mode.
    fn string(&mut self) {
        let start = self.pos;
        self.pos += 1; // consume `"`
        self.string_body(start, true);
    }

    /// Scan literal string content until `"` (end), `{` (hole), or EOF.
    /// `opening` is true for the first segment of the string.
    fn string_body(&mut self, seg_start: usize, opening: bool) {
        let mut buf = String::new();
        loop {
            match self.peek_char() {
                None | Some('\n') => {
                    let span = Span::new(seg_start as u32, self.pos as u32);
                    self.error_at(span, "E0102", "unterminated string literal");
                    let kind = if opening {
                        TokenKind::Str(buf)
                    } else {
                        TokenKind::StrInterpEnd(buf)
                    };
                    self.push(kind, seg_start, self.pos);
                    return;
                }
                Some('"') => {
                    self.pos += 1;
                    let kind = if opening {
                        TokenKind::Str(buf)
                    } else {
                        TokenKind::StrInterpEnd(buf)
                    };
                    self.push(kind, seg_start, self.pos);
                    return;
                }
                Some('{') => {
                    self.pos += 1;
                    let kind = if opening {
                        TokenKind::StrInterpStart(buf)
                    } else {
                        TokenKind::StrInterpMid(buf)
                    };
                    self.push(kind, seg_start, self.pos);
                    self.modes.push(InterpMode { brace_depth: 0 });
                    return;
                }
                Some('\\') => {
                    let esc_start = self.pos;
                    self.pos += 1;
                    match self.bump() {
                        Some('n') => buf.push('\n'),
                        Some('t') => buf.push('\t'),
                        Some('r') => buf.push('\r'),
                        Some('\\') => buf.push('\\'),
                        Some('"') => buf.push('"'),
                        Some('0') => buf.push('\0'),
                        Some('{') => buf.push('{'),
                        Some('}') => buf.push('}'),
                        Some('u') => {
                            if self.eat(b'{') {
                                let hex_start = self.pos;
                                while self.peek_byte(0).is_ascii_hexdigit() {
                                    self.pos += 1;
                                }
                                let hex = &self.text[hex_start..self.pos];
                                let ok = self.eat(b'}');
                                let scalar = u32::from_str_radix(hex, 16)
                                    .ok()
                                    .and_then(char::from_u32);
                                match (ok, scalar) {
                                    (true, Some(c)) if !hex.is_empty() => buf.push(c),
                                    _ => {
                                        let span =
                                            Span::new(esc_start as u32, self.pos as u32);
                                        self.error_at(
                                            span,
                                            "E0107",
                                            "invalid unicode escape; expected `\\u{...}` with 1-6 hex digits naming a Unicode scalar",
                                        );
                                    }
                                }
                            } else {
                                let span = Span::new(esc_start as u32, self.pos as u32);
                                self.error_at(span, "E0107", "invalid escape `\\u`; expected `\\u{...}`");
                            }
                        }
                        other => {
                            let span = Span::new(esc_start as u32, self.pos as u32);
                            let shown = other.map(|c| c.to_string()).unwrap_or_default();
                            self.error_at(
                                span,
                                "E0107",
                                format!("unknown escape sequence `\\{shown}`"),
                            );
                        }
                    }
                }
                Some(c) => {
                    buf.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    // ---- operators & punctuation ----

    fn operator(&mut self, c: char) {
        let start = self.pos;
        match c {
            '{' => {
                self.pos += 1;
                if let Some(m) = self.modes.last_mut() {
                    m.brace_depth += 1;
                }
                self.push(TokenKind::LBrace, start, self.pos);
            }
            '}' => {
                self.pos += 1;
                match self.modes.last_mut() {
                    Some(m) if m.brace_depth == 0 => {
                        // This `}` closes an interpolation hole: resume the string.
                        self.modes.pop();
                        self.string_body(start, false);
                    }
                    Some(m) => {
                        m.brace_depth -= 1;
                        self.push(TokenKind::RBrace, start, self.pos);
                    }
                    None => self.push(TokenKind::RBrace, start, self.pos),
                }
            }
            '+' => {
                self.pos += 1;
                let k = if self.eat(b'=') { TokenKind::PlusEq } else { TokenKind::Plus };
                self.push(k, start, self.pos);
            }
            '-' => {
                self.pos += 1;
                let k = if self.eat(b'>') {
                    TokenKind::Arrow
                } else if self.eat(b'=') {
                    TokenKind::MinusEq
                } else {
                    TokenKind::Minus
                };
                self.push(k, start, self.pos);
            }
            '*' => {
                self.pos += 1;
                let k = if self.eat(b'=') { TokenKind::StarEq } else { TokenKind::Star };
                self.push(k, start, self.pos);
            }
            '/' => {
                self.pos += 1;
                let k = if self.eat(b'=') { TokenKind::SlashEq } else { TokenKind::Slash };
                self.push(k, start, self.pos);
            }
            '%' => {
                self.pos += 1;
                let k = if self.eat(b'=') { TokenKind::PercentEq } else { TokenKind::Percent };
                self.push(k, start, self.pos);
            }
            '=' => {
                self.pos += 1;
                let k = if self.eat(b'=') {
                    TokenKind::EqEq
                } else if self.eat(b'>') {
                    TokenKind::FatArrow
                } else {
                    TokenKind::Eq
                };
                self.push(k, start, self.pos);
            }
            '!' => {
                self.pos += 1;
                let k = if self.eat(b'=') { TokenKind::BangEq } else { TokenKind::Bang };
                self.push(k, start, self.pos);
            }
            '<' => {
                self.pos += 1;
                if self.eat(b'<') {
                    // `a << 1` otherwise dies as a bare "expected an
                    // expression" two tokens later; say what's missing.
                    self.error_at(
                        Span::new(start as u32, self.pos as u32),
                        "E0100",
                        "unexpected `<<`; Fable has no bitwise shift operators",
                    );
                } else {
                    let k = if self.eat(b'=') { TokenKind::Le } else { TokenKind::Lt };
                    self.push(k, start, self.pos);
                }
            }
            '>' => {
                self.pos += 1;
                if self.eat(b'>') {
                    self.error_at(
                        Span::new(start as u32, self.pos as u32),
                        "E0100",
                        "unexpected `>>`; Fable has no bitwise shift operators",
                    );
                } else {
                    let k = if self.eat(b'=') { TokenKind::Ge } else { TokenKind::Gt };
                    self.push(k, start, self.pos);
                }
            }
            '&' => {
                self.pos += 1;
                if self.eat(b'&') {
                    self.push(TokenKind::AmpAmp, start, self.pos);
                } else {
                    self.error_at(
                        Span::new(start as u32, self.pos as u32),
                        "E0100",
                        "unexpected character `&`; logical and is `&&`",
                    );
                }
            }
            '|' => {
                self.pos += 1;
                let k = if self.eat(b'|') { TokenKind::PipePipe } else { TokenKind::Pipe };
                self.push(k, start, self.pos);
            }
            '.' => {
                self.pos += 1;
                let k = if self.eat(b'.') {
                    if self.eat(b'=') {
                        TokenKind::DotDotEq
                    } else {
                        TokenKind::DotDot
                    }
                } else {
                    TokenKind::Dot
                };
                self.push(k, start, self.pos);
            }
            ',' => {
                self.pos += 1;
                self.push(TokenKind::Comma, start, self.pos);
            }
            '?' => {
                self.pos += 1;
                self.push(TokenKind::Question, start, self.pos);
            }
            ':' => {
                self.pos += 1;
                self.push(TokenKind::Colon, start, self.pos);
            }
            ';' => {
                self.pos += 1;
                self.push(TokenKind::Semi, start, self.pos);
            }
            '(' => {
                self.pos += 1;
                self.push(TokenKind::LParen, start, self.pos);
            }
            ')' => {
                self.pos += 1;
                self.push(TokenKind::RParen, start, self.pos);
            }
            '[' => {
                self.pos += 1;
                self.push(TokenKind::LBracket, start, self.pos);
            }
            ']' => {
                self.pos += 1;
                self.push(TokenKind::RBracket, start, self.pos);
            }
            other => {
                self.pos += other.len_utf8();
                self.error_at(
                    Span::new(start as u32, self.pos as u32),
                    "E0100",
                    format!("unexpected character `{other}`"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let out = lex(src);
        assert!(out.diags.is_empty(), "unexpected lex errors: {:?}", out.diags);
        out.tokens.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("42 1_000 0x2A 0b1010 6.25 1e9 2.5e-3"),
            vec![
                Int(42),
                Int(1000),
                Int(42),
                Int(10),
                Float(6.25),
                Float(1e9),
                Float(2.5e-3),
                Eof
            ]
        );
    }

    #[test]
    fn range_vs_float() {
        assert_eq!(kinds("1..5"), vec![Int(1), DotDot, Int(5), Eof]);
        assert_eq!(kinds("1..=5"), vec![Int(1), DotDotEq, Int(5), Eof]);
    }

    #[test]
    fn keywords_and_idents() {
        assert_eq!(
            kinds("let mut foo fn true"),
            vec![Let, Mut, Ident("foo".into()), Fn, True, Eof]
        );
    }

    #[test]
    fn simple_string() {
        assert_eq!(
            kinds(r#""hello\nworld""#),
            vec![Str("hello\nworld".into()), Eof]
        );
    }

    #[test]
    fn interpolated_string() {
        assert_eq!(
            kinds(r#""a {x} b {y} c""#),
            vec![
                StrInterpStart("a ".into()),
                Ident("x".into()),
                StrInterpMid(" b ".into()),
                Ident("y".into()),
                StrInterpEnd(" c".into()),
                Eof
            ]
        );
    }

    #[test]
    fn nested_interpolation() {
        assert_eq!(
            kinds(r#""outer {f("inner {x}")}!""#),
            vec![
                StrInterpStart("outer ".into()),
                Ident("f".into()),
                LParen,
                StrInterpStart("inner ".into()),
                Ident("x".into()),
                StrInterpEnd("".into()),
                RParen,
                StrInterpEnd("!".into()),
                Eof
            ]
        );
    }

    #[test]
    fn braces_inside_interpolation() {
        // A block expression inside a hole: `}` at depth>0 is a normal RBrace.
        assert_eq!(
            kinds(r#""v = {{ 1 }}""#),
            vec![
                StrInterpStart("v = ".into()),
                LBrace,
                Int(1),
                RBrace,
                StrInterpEnd("".into()),
                Eof
            ]
        );
    }

    #[test]
    fn escaped_brace() {
        assert_eq!(kinds(r#""\{x}""#), vec![Str("{x}".into()), Eof]);
    }

    #[test]
    fn comments_captured() {
        let out = lex("1 // line\n/* blk /* nested */ */ 2");
        assert_eq!(out.comments.len(), 2);
        assert!(out.comments[1].block);
        assert_eq!(
            out.tokens.iter().map(|t| t.kind.clone()).collect::<Vec<_>>(),
            vec![Int(1), Int(2), Eof]
        );
    }

    #[test]
    fn unicode_escape() {
        assert_eq!(kinds(r#""\u{1F600}""#), vec![Str("😀".into()), Eof]);
    }

    #[test]
    fn errors_reported() {
        assert!(!lex("\"unterminated").diags.is_empty());
        assert!(!lex("@").diags.is_empty());
    }

    #[test]
    fn and_or_not_are_identifiers() {
        // The words are legal identifiers; the parser gives targeted hints
        // when they appear in operator position.
        assert_eq!(kinds("or"), vec![Ident("or".into()), Eof]);
    }
}
