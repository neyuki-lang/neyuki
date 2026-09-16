// Core Diagnostic structure representing a structured compiler diagnostic.

use crate::diagnostics::error_code::ErrorCode;
use crate::diagnostics::severity::Severity;
use crate::diagnostics::span::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

impl Label {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<ErrorCode>,
    pub message: String,
    pub primary_label: Option<Label>,
    pub secondary_labels: Vec<Label>,
    pub notes: Vec<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: None,
            message: message.into(),
            primary_label: None,
            secondary_labels: Vec::new(),
            notes: Vec::new(),
            help: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(Severity::Info, message)
    }

    pub fn with_code(mut self, code: ErrorCode) -> Self {
        self.code = Some(code);
        self
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.primary_label = Some(Label::new(span, message));
        self
    }

    pub fn with_secondary_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.secondary_labels.push(Label::new(span, message));
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(code) = self.code {
            write!(f, "{}[{}]: {}", self.severity, code, self.message)?;
        } else {
            write!(f, "{}: {}", self.severity, self.message)?;
        }
        if let Some(label) = &self.primary_label {
            write!(f, " (at line {})", label.span.start.line)?;
        }
        Ok(())
    }
}
