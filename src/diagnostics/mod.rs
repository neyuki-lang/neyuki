// Unified compiler diagnostics and error reporting system for Neyuki.

pub mod diagnostic;
pub mod emitter;
pub mod error_code;
pub mod severity;
pub mod span;

pub use diagnostic::{Diagnostic, Label};
pub use emitter::render_report;
pub use error_code::ErrorCode;
pub use severity::Severity;
pub use span::{SourceLocation, Span};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_rendering() {
        let source = "local a = 10\nreturn a + b\n";
        let diag = Diagnostic::error("cannot find value 'b' in this scope")
            .with_code(ErrorCode::E0001)
            .with_label(Span::single_line(2, 12, 13), "not found in this scope")
            .with_help("declare 'local b = ...' before accessing it");

        let report = render_report("test.nyk", source, &[diag]);
        assert!(report.contains("error[E0001]: cannot find value 'b' in this scope"));
        assert!(report.contains("--> test.nyk:2:12"));
        assert!(report.contains("return a + b"));
        assert!(report.contains("^ not found in this scope"));
        assert!(report.contains("= help: declare 'local b = ...' before accessing it"));
    }
}
