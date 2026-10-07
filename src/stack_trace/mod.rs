// Modular stack trace engine for Neyuki runtime & VM.

pub mod formatter;
pub mod frame;
pub mod snippet;
pub mod unwind;

pub use formatter::{StackTrace, StackTraceFormatter};
pub use frame::StackFrame;
pub use snippet::{SourceLine, SourceSnippet};
pub use unwind::StackUnwinder;

use crate::vm::machine::VM;

/// Creates a standard traceback string for the VM.
pub fn build_vm_traceback(vm: &VM, message: Option<String>, level_offset: usize) -> String {
    let trace = StackUnwinder::unwind_vm(&vm.frames, &vm.stack, message, level_offset);
    StackTraceFormatter::format_standard(&trace)
}

/// Creates a rich diagnostic pretty traceback for the VM.
pub fn build_vm_traceback_pretty(vm: &VM, message: Option<String>, level_offset: usize) -> String {
    let trace = StackUnwinder::unwind_vm(&vm.frames, &vm.stack, message, level_offset);
    StackTraceFormatter::format_pretty(&trace)
}

/// Creates a traceback string from AST interpreter runtime call frames.
pub fn build_runtime_traceback(raw_frames: &[String], message: Option<String>) -> String {
    let trace = StackUnwinder::unwind_runtime_strings(raw_frames, message);
    StackTraceFormatter::format_standard(&trace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snippet_extraction() {
        let code = "local a = 1\nlocal b = 2\nerror('fail')\nlocal c = 3";
        let snippet = SourceSnippet::from_source(code, 3, 1);
        assert_eq!(snippet.lines.len(), 3);
        assert_eq!(snippet.lines[1].line_number, 3);
        assert!(snippet.lines[1].is_highlight);
        let rendered = snippet.render();
        assert!(rendered.contains(">    3 | error('fail')"));
    }

    #[test]
    fn test_stack_trace_formatting() {
        let mut frame1 = StackFrame::new(1, "foo", 42).with_file("test.nyk");
        frame1.add_local("x", "10");
        let frame2 = StackFrame::new(2, "main", 10).with_file("main.nyk");

        let trace = StackTrace::new(Some("something broke".to_string()), vec![frame1, frame2]);
        let std_output = StackTraceFormatter::format_standard(&trace);
        assert!(std_output.contains("something broke"));
        assert!(std_output.contains("stack traceback:"));
        assert!(std_output.contains("[frame 1] function 'foo' in file 'test.nyk:42'"));
        assert!(std_output.contains("[frame 2] function 'main' in file 'main.nyk:10'"));

        let pretty_output = StackTraceFormatter::format_pretty(&trace);
        assert!(pretty_output.contains("Error: something broke"));
        assert!(pretty_output.contains("--> Frame 1: in 'foo' (test.nyk:42)"));
        assert!(pretty_output.contains("* x = 10"));

        let md_output = StackTraceFormatter::format_markdown(&trace);
        assert!(md_output.contains("### Error: `something broke`"));
        assert!(md_output.contains("```neyuki-trace"));
    }
}
