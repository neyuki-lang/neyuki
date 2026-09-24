// Unwinding engine for extracting stack frames from the VM and runtime.

use super::formatter::StackTrace;
use super::frame::StackFrame;
use crate::vm::frame::CallFrame;
use crate::vm::value::Value;

pub struct StackUnwinder;

impl StackUnwinder {
    /// Unwinds VM call frames into a structured `StackTrace`.
    pub fn unwind_vm(
        frames: &[CallFrame],
        stack: &[Value],
        message: Option<String>,
        level_offset: usize,
    ) -> StackTrace {
        let total_frames = frames.len();
        let mut trace_frames = Vec::new();

        for (i, frame) in frames.iter().rev().enumerate() {
            if i < level_offset.saturating_sub(1) {
                continue;
            }

            let depth = total_frames.saturating_sub(i);
            let proto = &frame.closure.proto;
            let name = proto.name.as_deref().unwrap_or("<anonymous>");

            let line = if frame.ip < proto.lines.len() {
                proto.lines[frame.ip]
            } else {
                proto.lines.last().copied().unwrap_or(1)
            };

            let mut stack_frame = StackFrame::new(depth, name, line as usize);

            // Extract visible local variables from this frame's register window
            for loc in &proto.local_names {
                let pc = frame.ip as u32;
                if pc >= loc.from_pc && pc <= loc.to_pc {
                    let reg_idx = frame.base + (loc.reg as usize);
                    if reg_idx < stack.len() {
                        let val_str = stack[reg_idx].to_string();
                        stack_frame.add_local(&loc.name, val_str);
                    }
                }
            }

            trace_frames.push(stack_frame);
        }

        StackTrace::new(message, trace_frames)
    }

    /// Unwinds simple string frames (e.g. from AST interpreter runtime stack).
    pub fn unwind_runtime_strings(raw_frames: &[String], message: Option<String>) -> StackTrace {
        let total = raw_frames.len();
        let mut frames = Vec::new();

        for (i, raw) in raw_frames.iter().rev().enumerate() {
            let depth = total.saturating_sub(i);
            // Example format: "function 'foo' at line 10" or "foo:10"
            let func_name = if let Some(start) = raw.find("function '") {
                let rest = &raw[start + 10..];
                if let Some(end) = rest.find('\'') {
                    rest[..end].to_string()
                } else {
                    raw.clone()
                }
            } else {
                raw.clone()
            };

            let line = if let Some(idx) = raw.rfind("line ") {
                raw[idx + 5..].trim().parse::<usize>().unwrap_or(1)
            } else {
                1
            };

            frames.push(StackFrame::new(depth, func_name, line));
        }

        StackTrace::new(message, frames)
    }
}
