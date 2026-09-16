// Source span and location definitions for the unified diagnostics system.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SourceLocation {
    pub line: u32,
    pub column: u32,
    pub byte_offset: usize,
}

impl SourceLocation {
    pub fn new(line: u32, column: u32, byte_offset: usize) -> Self {
        Self { line, column, byte_offset }
    }

    pub fn line_col(line: u32, column: u32) -> Self {
        Self { line, column, byte_offset: 0 }
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

    pub fn single_line(line: u32, start_col: u32, end_col: u32) -> Self {
        Self {
            start: SourceLocation::line_col(line, start_col),
            end: SourceLocation::line_col(line, end_col),
        }
    }

    pub fn line(line: u32) -> Self {
        Self {
            start: SourceLocation::line_col(line, 1),
            end: SourceLocation::line_col(line, 1),
        }
    }
}
