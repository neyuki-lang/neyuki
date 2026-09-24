// Interactive line reader and multi-line chunk engine for Neyuki parser & REPL.

#[derive(Clone, Debug, Default)]
pub struct LineHistory {
    entries: Vec<String>,
    cursor: usize,
    max_entries: usize,
}

impl LineHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            max_entries: if max_entries == 0 { 100 } else { max_entries },
        }
    }

    pub fn add(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.entries.last().map(|s| s.as_str()) == Some(trimmed) {
            return;
        }
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(trimmed.to_string());
        self.cursor = self.entries.len();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChunkStatus {
    Complete,
    Incomplete { depth: usize, reason: &'static str },
    Empty,
}

#[derive(Clone, Debug)]
pub struct ReadLineEngine {
    buffer: String,
    history: LineHistory,
    prompt: &'static str,
    continuation_prompt: &'static str,
}

impl Default for ReadLineEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadLineEngine {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            history: LineHistory::new(200),
            prompt: "> ",
            continuation_prompt: ">> ",
        }
    }

    pub fn prompt(&self) -> &'static str {
        if self.is_continuation() {
            self.continuation_prompt
        } else {
            self.prompt
        }
    }

    pub fn is_continuation(&self) -> bool {
        !self.buffer.is_empty()
    }

    pub fn feed_line(&mut self, line: &str) -> ChunkStatus {
        if self.buffer.is_empty() && line.trim().is_empty() {
            return ChunkStatus::Empty;
        }

        self.buffer.push_str(line);
        let status = self.analyze_buffer();
        if matches!(status, ChunkStatus::Complete) {
            self.history.add(&self.buffer);
        }
        status
    }

    pub fn take_chunk(&mut self) -> String {
        std::mem::take(&mut self.buffer)
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn current_buffer(&self) -> &str {
        &self.buffer
    }

    pub fn history(&self) -> &LineHistory {
        &self.history
    }

    /// Analyzes the accumulated buffer to check if it has complete constructs
    /// or needs continuation.
    pub fn analyze_buffer(&self) -> ChunkStatus {
        let src = self.buffer.trim();
        if src.is_empty() {
            return ChunkStatus::Empty;
        }

        let mut block_depth: isize = 0;
        let mut paren_depth: isize = 0;
        let mut bracket_depth: isize = 0;
        let mut brace_depth: isize = 0;
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut in_backtick = false;

        let mut chars = src.chars().peekable();
        let mut word = String::new();

        while let Some(ch) = chars.next() {
            if in_single_quote {
                if ch == '\\' {
                    let _ = chars.next();
                } else if ch == '\'' {
                    in_single_quote = false;
                }
                continue;
            }
            if in_double_quote {
                if ch == '\\' {
                    let _ = chars.next();
                } else if ch == '"' {
                    in_double_quote = false;
                }
                continue;
            }
            if in_backtick {
                if ch == '\\' {
                    let _ = chars.next();
                } else if ch == '`' {
                    in_backtick = false;
                }
                continue;
            }

            // Line comment --
            if ch == '-' && chars.peek() == Some(&'-') {
                chars.next();
                // skip till newline
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }

            match ch {
                '\'' => in_single_quote = true,
                '"' => in_double_quote = true,
                '`' => in_backtick = true,
                '(' => paren_depth += 1,
                ')' => paren_depth = (paren_depth - 1).max(0),
                '[' => bracket_depth += 1,
                ']' => bracket_depth = (bracket_depth - 1).max(0),
                '{' => brace_depth += 1,
                '}' => brace_depth = (brace_depth - 1).max(0),
                c if c.is_ascii_alphanumeric() || c == '_' => {
                    word.push(c);
                    // check next char
                    if chars
                        .peek()
                        .is_none_or(|next| !next.is_ascii_alphanumeric() && *next != '_')
                    {
                        match word.as_str() {
                            "function" | "do" | "then" | "repeat" => {
                                block_depth += 1;
                            }
                            "end" | "until" => {
                                block_depth = (block_depth - 1).max(0);
                            }
                            _ => {}
                        }
                        word.clear();
                    }
                }
                _ => {
                    word.clear();
                }
            }
        }

        if in_single_quote || in_double_quote {
            return ChunkStatus::Incomplete {
                depth: 1,
                reason: "Unterminated string literal",
            };
        }
        if in_backtick {
            return ChunkStatus::Incomplete {
                depth: 1,
                reason: "Unterminated template literal",
            };
        }
        if paren_depth > 0 || bracket_depth > 0 || brace_depth > 0 {
            let total = (paren_depth + bracket_depth + brace_depth) as usize;
            return ChunkStatus::Incomplete {
                depth: total,
                reason: "Unclosed delimiter",
            };
        }
        if block_depth > 0 {
            return ChunkStatus::Incomplete {
                depth: block_depth as usize,
                reason: "Unclosed block (expected 'end' or 'until')",
            };
        }

        ChunkStatus::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_readline_complete_and_empty() {
        let mut engine = ReadLineEngine::new();
        assert_eq!(engine.prompt(), "> ");
        assert_eq!(engine.feed_line(""), ChunkStatus::Empty);
        assert_eq!(engine.feed_line("   \n"), ChunkStatus::Empty);

        let status = engine.feed_line("local x = 10\n");
        assert_eq!(status, ChunkStatus::Complete);
        assert_eq!(engine.take_chunk(), "local x = 10\n");
    }

    #[test]
    fn test_readline_multiline_blocks() {
        let mut engine = ReadLineEngine::new();
        let s1 = engine.feed_line("function calculate(a, b)\n");
        assert!(matches!(s1, ChunkStatus::Incomplete { .. }));
        assert!(engine.is_continuation());
        assert_eq!(engine.prompt(), ">> ");

        let s2 = engine.feed_line("    return a + b\n");
        assert!(matches!(s2, ChunkStatus::Incomplete { .. }));

        let s3 = engine.feed_line("end\n");
        assert_eq!(s3, ChunkStatus::Complete);
        assert_eq!(
            engine.take_chunk(),
            "function calculate(a, b)\n    return a + b\nend\n"
        );
    }

    #[test]
    fn test_readline_unclosed_delimiters() {
        let mut engine = ReadLineEngine::new();
        let s1 = engine.feed_line("local list = [1, 2, \n");
        assert!(matches!(s1, ChunkStatus::Incomplete { .. }));

        let s2 = engine.feed_line("3]\n");
        assert_eq!(s2, ChunkStatus::Complete);
    }

    #[test]
    fn test_readline_history() {
        let mut engine = ReadLineEngine::new();
        engine.feed_line("print(1)\n");
        engine.take_chunk();
        engine.feed_line("print(2)\n");
        engine.take_chunk();

        assert_eq!(engine.history().len(), 2);
        assert_eq!(engine.history().entries()[0], "print(1)");
        assert_eq!(engine.history().entries()[1], "print(2)");
    }
}
