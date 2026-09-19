// Source code span and location tracking for AST nodes.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SourceLocation {
    pub line: u32,
    pub column: u32,
}

impl SourceLocation {
    pub fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

impl Span {
    pub fn new(start: SourceLocation, end: SourceLocation) -> Self {
        Self { start, end }
    }

    pub fn point(line: u32, column: u32) -> Self {
        let loc = SourceLocation::new(line, column);
        Self {
            start: loc,
            end: loc,
        }
    }

    pub fn dummy() -> Self {
        Self::default()
    }

    pub fn combine(self, other: Self) -> Self {
        if self == Self::default() {
            return other;
        }
        if other == Self::default() {
            return self;
        }
        let start = if self.start.line < other.start.line
            || (self.start.line == other.start.line && self.start.column <= other.start.column)
        {
            self.start
        } else {
            other.start
        };
        let end = if self.end.line > other.end.line
            || (self.end.line == other.end.line && self.end.column >= other.end.column)
        {
            self.end
        } else {
            other.end
        };
        Self { start, end }
    }
}

impl std::fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpannedNode<T> {
    pub node: T,
    pub span: Span,
    pub id: crate::ast::node_id::NodeId,
}

impl<T> SpannedNode<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self {
            node,
            span,
            id: crate::ast::node_id::NodeId::next(),
        }
    }

    pub fn with_id(node: T, span: Span, id: crate::ast::node_id::NodeId) -> Self {
        Self { node, span, id }
    }
}

use crate::ast::node_id::NodeId;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanPool {
    map: HashMap<NodeId, Span>,
}

impl SpanPool {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn insert(&mut self, id: NodeId, span: Span) {
        self.map.insert(id, span);
    }

    pub fn get(&self, id: NodeId) -> Option<Span> {
        self.map.get(&id).copied()
    }

    pub fn get_or_dummy(&self, id: NodeId) -> Span {
        self.map.get(&id).copied().unwrap_or_default()
    }
}

impl Default for SpanPool {
    fn default() -> Self {
        Self::new()
    }
}
