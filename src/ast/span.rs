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
}
