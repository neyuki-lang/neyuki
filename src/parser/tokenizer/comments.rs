// Comment and doc comment scanning engine for Neyuki tokenizer.

use super::scanner::Scanner;
use super::strings::StringEngine;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocComment {
    pub line: usize,
    pub tag: Option<String>,
    pub text: String,
}

pub struct CommentEngine;

impl CommentEngine {
    #[inline(always)]
    pub fn is_comment_start(scanner: &Scanner<'_>, is_at_line_or_whitespace_start: bool) -> bool {
        is_at_line_or_whitespace_start && scanner.starts_with("--")
    }

    pub fn scan_comment(scanner: &mut Scanner<'_>) -> Option<DocComment> {
        if !scanner.starts_with("--") {
            return None;
        }

        let start_line = scanner.line();
        scanner.advance(); // first '-'
        scanner.advance(); // second '-'

        // Check if doc comment "---"
        let is_doc = scanner.peek() == '-' && scanner.peek_next() != Some('-');
        if is_doc {
            scanner.advance();
        }

        // Check if long bracket comment --[=[ ... ]=]
        if scanner.peek() == '[' {
            let saved = scanner.current_pos();
            if let Ok(content) = StringEngine::scan_long_bracket(scanner) {
                return Some(DocComment {
                    line: start_line,
                    tag: None,
                    text: content,
                });
            } else {
                scanner.restore(saved);
            }
        }

        // Single line comment
        let text_start = scanner.pos();
        while !scanner.is_at_end() && scanner.peek() != '\n' {
            scanner.advance();
        }
        let line_text = scanner.slice_from(text_start).trim();

        let tag = if is_doc && line_text.starts_with('@') {
            let mut parts = line_text.split_whitespace();
            parts.next().map(|t| t.to_string())
        } else {
            None
        };

        Some(DocComment {
            line: start_line,
            tag,
            text: line_text.to_string(),
        })
    }
}
