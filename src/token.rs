//! Token definitions.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Int(i64),
    Float(f64),
    /// A plain string literal with no interpolation (already unescaped).
    Str(String),
    /// `"text {` — opens an interpolated string; payload is the leading text.
    StrInterpStart(String),
    /// `} text {` — between two interpolation expressions.
    StrInterpMid(String),
    /// `} text"` — closes an interpolated string.
    StrInterpEnd(String),

    Ident(String),

    // Keywords
    Let,
    Mut,
    Fn,
    Struct,
    Enum,
    Impl,
    Import,
    Match,
    If,
    Else,
    While,
    For,
    In,
    Return,
    Break,
    Continue,
    True,
    False,

    // Punctuation / operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    AmpAmp,
    PipePipe,
    Bang,
    Eq,
    Arrow,     // ->
    FatArrow,  // =>
    Dot,
    DotDot,    // ..
    DotDotEq,  // ..=
    Comma,
    Colon,
    Semi,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Pipe,
    Underscore,
    /// `?` — the try operator.
    Question,
    /// `{:}` empty-map literal is lexed as LBrace, Colon, RBrace by the parser;
    /// no dedicated token needed.
    Eof,
}

impl TokenKind {
    /// Human-readable name for error messages.
    pub fn describe(&self) -> String {
        use TokenKind::*;
        match self {
            Int(_) => "integer literal".into(),
            Float(_) => "float literal".into(),
            Str(_) | StrInterpStart(_) => "string literal".into(),
            StrInterpMid(_) | StrInterpEnd(_) => "string interpolation".into(),
            Ident(name) => format!("identifier `{name}`"),
            Let => "`let`".into(),
            Mut => "`mut`".into(),
            Fn => "`fn`".into(),
            Struct => "`struct`".into(),
            Enum => "`enum`".into(),
            Impl => "`impl`".into(),
            Import => "`import`".into(),
            Match => "`match`".into(),
            If => "`if`".into(),
            Else => "`else`".into(),
            While => "`while`".into(),
            For => "`for`".into(),
            In => "`in`".into(),
            Return => "`return`".into(),
            Break => "`break`".into(),
            Continue => "`continue`".into(),
            True => "`true`".into(),
            False => "`false`".into(),
            Plus => "`+`".into(),
            Minus => "`-`".into(),
            Star => "`*`".into(),
            Slash => "`/`".into(),
            Percent => "`%`".into(),
            PlusEq => "`+=`".into(),
            MinusEq => "`-=`".into(),
            StarEq => "`*=`".into(),
            SlashEq => "`/=`".into(),
            PercentEq => "`%=`".into(),
            EqEq => "`==`".into(),
            BangEq => "`!=`".into(),
            Lt => "`<`".into(),
            Le => "`<=`".into(),
            Gt => "`>`".into(),
            Ge => "`>=`".into(),
            AmpAmp => "`&&`".into(),
            PipePipe => "`||`".into(),
            Bang => "`!`".into(),
            Eq => "`=`".into(),
            Arrow => "`->`".into(),
            FatArrow => "`=>`".into(),
            Dot => "`.`".into(),
            DotDot => "`..`".into(),
            DotDotEq => "`..=`".into(),
            Comma => "`,`".into(),
            Colon => "`:`".into(),
            Semi => "`;`".into(),
            LParen => "`(`".into(),
            RParen => "`)`".into(),
            LBracket => "`[`".into(),
            RBracket => "`]`".into(),
            LBrace => "`{`".into(),
            RBrace => "`}`".into(),
            Pipe => "`|`".into(),
            Underscore => "`_`".into(),
            Question => "`?`".into(),
            Eof => "end of file".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// A comment captured during lexing, for the formatter.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    /// Text including the `//` or `/* */` delimiters.
    pub text: String,
    pub span: Span,
    /// True for `/* ... */` comments.
    pub block: bool,
}
