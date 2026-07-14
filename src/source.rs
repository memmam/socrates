//! Source file representation with line/column lookup.

use crate::span::Span;

/// A loaded source file. Owns the text and a precomputed table of line-start
/// offsets so spans can be mapped to line/column pairs in O(log n).
pub struct Source {
    pub name: String,
    pub text: String,
    line_starts: Vec<u32>,
}

/// A 1-based line/column position (column counts Unicode scalars, not bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

impl Source {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Source {
        let text = text.into();
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        Source { name: name.into(), text, line_starts }
    }

    /// Map a byte offset to a 1-based line/column. Offsets inside a multi-byte
    /// character are rounded down to the character's start.
    pub fn line_col(&self, offset: u32) -> LineCol {
        let mut offset = offset.min(self.text.len() as u32);
        while offset > 0 && !self.text.is_char_boundary(offset as usize) {
            offset -= 1;
        }
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let line_start = self.line_starts[line_idx] as usize;
        let col = self.text[line_start..offset as usize].chars().count() as u32;
        LineCol { line: line_idx as u32 + 1, col: col + 1 }
    }

    /// The full text of the 1-based line `line`, without its trailing newline.
    pub fn line_text(&self, line: u32) -> &str {
        let idx = (line - 1) as usize;
        let start = self.line_starts[idx] as usize;
        let end = self
            .line_starts
            .get(idx + 1)
            .map(|&s| s as usize)
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    pub fn num_lines(&self) -> u32 {
        self.line_starts.len() as u32
    }

    pub fn snippet(&self, span: Span) -> &str {
        &self.text[span.start as usize..span.end as usize]
    }
}
