// Formatting engine for stack traces.

use super::frame::StackFrame;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackTrace {
    pub message: Option<String>,
    pub frames: Vec<StackFrame>,
}

impl StackTrace {
    pub fn new(message: Option<String>, frames: Vec<StackFrame>) -> Self {
        Self { message, frames }
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }
}

pub struct StackTraceFormatter;

impl StackTraceFormatter {
    /// Formats the stack trace in standard Lua/Neyuki traceback format.
    pub fn format_standard(trace: &StackTrace) -> String {
        let mut out = String::new();
        if let Some(msg) = &trace.message {
            out.push_str(msg);
            out.push('\n');
        }

        out.push_str("stack traceback:\n");
        for frame in &trace.frames {
            let loc_str = if let Some(file) = &frame.source_file {
                format!("in file '{}:{}'", file, frame.line)
            } else {
                format!("at line {}", frame.line)
            };

            let native_str = if frame.is_native { " [C native]" } else { "" };

            out.push_str(&format!(
                "  [frame {}] function '{}' {}{}\n",
                frame.depth, frame.function_name, loc_str, native_str
            ));
        }

        out
    }

    /// Formats the stack trace with rich diagnostics, source code snippets, and local variables.
    pub fn format_pretty(trace: &StackTrace) -> String {
        let mut out = String::new();
        if let Some(msg) = &trace.message {
            out.push_str(&format!("Error: {}\n", msg));
        }
        out.push_str("Stack Trace (most recent call last):\n");

        for frame in &trace.frames {
            let loc = frame.source_file.as_deref().unwrap_or("<unknown>");
            out.push_str(&format!(
                "  --> Frame {}: in '{}' ({}:{})\n",
                frame.depth, frame.function_name, loc, frame.line
            ));

            if let Some(snippet) = &frame.snippet {
                out.push_str(&snippet.render());
            }

            if !frame.locals.is_empty() {
                out.push_str("      Locals:\n");
                for (name, val) in &frame.locals {
                    out.push_str(&format!("        * {} = {}\n", name, val));
                }
            }
        }

        out
    }

    /// Formats the stack trace in GitHub Markdown format.
    pub fn format_markdown(trace: &StackTrace) -> String {
        let mut out = String::new();
        if let Some(msg) = &trace.message {
            out.push_str(&format!("### Error: `{}`\n\n", msg));
        }
        out.push_str("```neyuki-trace\n");
        out.push_str(&Self::format_standard(trace));
        out.push_str("```\n");
        out
    }
}
