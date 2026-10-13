// Unwinding engine for extracting stack frames from the VM.

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
}
