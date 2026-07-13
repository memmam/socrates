//! Byte-offset source spans.

/// A half-open byte range `[start, end)` into a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Span {
        debug_assert!(start <= end);
        Span { start, end }
    }

    pub fn point(at: u32) -> Span {
        Span { start: at, end: at }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Unique id for an AST node, assigned by the parser. The type checker keys its
/// side tables (inferred types, name resolutions, method resolutions) on these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const DUMMY: NodeId = NodeId(u32::MAX);
}
