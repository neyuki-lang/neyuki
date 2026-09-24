// Parser error types, error codes, and rich diagnostics engine for Neyuki.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    UnexpectedToken = 1001,
    UnfinishedConstruct = 1002,
    InvalidAssignment = 1003,
    TypeSyntaxError = 1004,
    ExpectedIdentifier = 1005,
    UnmatchedDelimiter = 1006,
    GeneralSyntaxError = 1099,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnexpectedToken => "E1001",
            Self::UnfinishedConstruct => "E1002",
            Self::InvalidAssignment => "E1003",
            Self::TypeSyntaxError => "E1004",
            Self::ExpectedIdentifier => "E1005",
            Self::UnmatchedDelimiter => "E1006",
            Self::GeneralSyntaxError => "E1099",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub line: usize,
    pub col: usize,
    pub len: usize,
    pub message: String,
    pub is_primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub code: ErrorCode,
    pub message: String,
    pub hint: Option<String>,
    pub labels: Vec<DiagnosticLabel>,
}

impl ParseError {
    pub fn new(line: usize, col: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            col,
            code: ErrorCode::GeneralSyntaxError,
            message: message.into(),
            hint: None,
            labels: Vec::new(),
        }
    }

    pub fn with_code(line: usize, col: usize, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            line,
            col,
            code,
            message: message.into(),
            hint: None,
            labels: Vec::new(),
        }
    }

    pub fn with_hint(
        line: usize,
        col: usize,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            line,
            col,
            code: ErrorCode::GeneralSyntaxError,
            message: message.into(),
            hint: Some(hint.into()),
            labels: Vec::new(),
        }
    }

    pub fn add_label(
        &mut self,
        line: usize,
        col: usize,
        len: usize,
        msg: impl Into<String>,
        is_primary: bool,
    ) {
        self.labels.push(DiagnosticLabel {
            line,
            col,
            len: if len == 0 { 1 } else { len },
            message: msg.into(),
            is_primary,
        });
    }

    pub fn format_diagnostic(&self, source_line: Option<&str>) -> String {
        let mut out = format!(
            "[{}] [line {}:{}] {}",
            self.code.as_str(),
            self.line,
            self.col,
            self.message
        );
        if let Some(src) = source_line {
            out.push('\n');
            out.push_str(src);
            out.push('\n');
            let indent = " ".repeat(self.col.saturating_sub(1));
            out.push_str(&format!("{}^", indent));
        }
        for label in &self.labels {
            out.push_str(&format!(
                "\n  --> line {}:{}: {}",
                label.line, label.col, label.message
            ));
        }
        if let Some(hint) = &self.hint {
            out.push_str(&format!("\nHint: {}", hint));
        }
        out
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[line {}] {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

impl From<ParseError> for String {
    fn from(err: ParseError) -> Self {
        err.to_string()
    }
}
