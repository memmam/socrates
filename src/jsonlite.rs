//! A minimal JSON value, parser, and writer — just enough for JSON-RPC
//! (the language server), keeping the zero-dependency rule.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            J::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[J]> {
        match self {
            J::Arr(items) => Some(items),
            _ => None,
        }
    }

    /// Builder shorthand for objects.
    pub fn obj(entries: Vec<(&str, J)>) -> J {
        J::Obj(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn str(s: impl Into<String>) -> J {
        J::Str(s.into())
    }

    pub fn write(&self, out: &mut String) {
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 9e15 {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{n}");
                }
            }
            J::Str(s) => write_json_string(s, out),
            J::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            J::Obj(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

}

impl std::fmt::Display for J {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        self.write(&mut s);
        f.write_str(&s)
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn parse(text: &str) -> Result<J, String> {
    let mut p = P { bytes: text.as_bytes(), pos: 0 };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(format!("trailing input at byte {}", p.pos));
    }
    Ok(v)
}

struct P<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        if self.eat(b) {
            Ok(())
        } else {
            Err(format!("expected `{}` at byte {}", b as char, self.pos))
        }
    }

    fn lit(&mut self, word: &str, v: J) -> Result<J, String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(format!("invalid literal at byte {}", self.pos))
        }
    }

    fn value(&mut self) -> Result<J, String> {
        self.skip_ws();
        match self.peek() {
            None => Err("unexpected end of input".to_string()),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(J::Str(self.string()?)),
            Some(b't') => self.lit("true", J::Bool(true)),
            Some(b'f') => self.lit("false", J::Bool(false)),
            Some(b'n') => self.lit("null", J::Null),
            Some(_) => self.number(),
        }
    }

    fn number(&mut self) -> Result<J, String> {
        let start = self.pos;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("");
        text.parse::<f64>()
            .map(J::Num)
            .map_err(|_| format!("invalid number `{text}` at byte {start}"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(b) = self.peek() else {
                return Err("unterminated string".to_string());
            };
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(esc) = self.peek() else {
                        return Err("unterminated escape".to_string());
                    };
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                // Surrogate pair: expect \uDC00-\uDFFF next.
                                if !(self.eat(b'\\') && self.eat(b'u')) {
                                    return Err("unpaired surrogate".to_string());
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err("invalid low surrogate".to_string());
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            out.push(
                                char::from_u32(cp)
                                    .ok_or_else(|| "invalid \\u escape".to_string())?,
                            );
                        }
                        _ => return Err(format!("unsupported escape at byte {}", self.pos)),
                    }
                }
                _ => {
                    // Collect the full UTF-8 sequence starting at b.
                    let start = self.pos - 1;
                    let len = utf8_len(b);
                    self.pos = start + len;
                    let chunk = self
                        .bytes
                        .get(start..self.pos)
                        .and_then(|s| std::str::from_utf8(s).ok())
                        .ok_or_else(|| "invalid UTF-8 in string".to_string())?;
                    out.push_str(chunk);
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let s = self
            .bytes
            .get(self.pos..self.pos + 4)
            .and_then(|s| std::str::from_utf8(s).ok())
            .ok_or_else(|| "truncated \\u escape".to_string())?;
        self.pos += 4;
        u32::from_str_radix(s, 16).map_err(|_| "invalid \\u escape".to_string())
    }

    fn array(&mut self) -> Result<J, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.eat(b']') {
            return Ok(J::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            if self.eat(b']') {
                return Ok(J::Arr(items));
            }
            self.expect(b',')?;
        }
    }

    fn object(&mut self) -> Result<J, String> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_ws();
        if self.eat(b'}') {
            return Ok(J::Obj(entries));
        }
        loop {
            self.skip_ws();
            let k = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let v = self.value()?;
            entries.push((k, v));
            self.skip_ws();
            if self.eat(b'}') {
                return Ok(J::Obj(entries));
            }
            self.expect(b',')?;
        }
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let text = r#"{"a":[1,2.5,true,null],"s":"x\n\"y\"","o":{"k":-3}}"#;
        let v = parse(text).unwrap();
        assert_eq!(parse(&v.to_string()).unwrap(), v);
        assert_eq!(v.get("s").unwrap().as_str().unwrap(), "x\n\"y\"");
        assert_eq!(v.get("o").unwrap().get("k").unwrap().as_f64().unwrap(), -3.0);
    }

    #[test]
    fn unicode_escapes() {
        let v = parse(r#""é 😀""#).unwrap();
        assert_eq!(v.as_str().unwrap(), "é 😀");
        // Non-ASCII passes through the writer unescaped but valid.
        assert_eq!(parse(&v.to_string()).unwrap(), v);
    }

    #[test]
    fn errors() {
        assert!(parse("{").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse(r#""\ud800x""#).is_err());
        assert!(parse("1 2").is_err());
    }
}
