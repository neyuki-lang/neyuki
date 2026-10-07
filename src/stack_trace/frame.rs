// Stack frame model for Neyuki stack traces.

use super::snippet::SourceSnippet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackFrame {
    pub depth: usize,
    pub function_name: String,
    pub source_file: Option<String>,
    pub line: usize,
    pub col: Option<usize>,
    pub is_native: bool,
    pub locals: Vec<(String, String)>,
    pub snippet: Option<SourceSnippet>,
}

impl StackFrame {
    pub fn new(depth: usize, function_name: impl Into<String>, line: usize) -> Self {
        Self {
            depth,
            function_name: function_name.into(),
            source_file: None,
            line,
            col: None,
            is_native: false,
            locals: Vec::new(),
            snippet: None,
        }
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.source_file = Some(file.into());
        self
    }

    pub fn with_col(mut self, col: usize) -> Self {
        self.col = Some(col);
        self
    }

    pub fn with_native(mut self, is_native: bool) -> Self {
        self.is_native = is_native;
        self
    }

    pub fn add_local(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.locals.push((name.into(), value.into()));
    }

    pub fn with_snippet(mut self, snippet: SourceSnippet) -> Self {
        self.snippet = Some(snippet);
        self
    }
}
