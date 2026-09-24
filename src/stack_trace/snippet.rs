// Source snippet extraction engine for stack traces.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLine {
    pub line_number: usize,
    pub content: String,
    pub is_highlight: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSnippet {
    pub file_path: Option<String>,
    pub target_line: usize,
    pub lines: Vec<SourceLine>,
}

impl SourceSnippet {
    pub fn from_source(source: &str, target_line: usize, context_lines: usize) -> Self {
        let all_lines: Vec<&str> = source.lines().collect();
        let total = all_lines.len();
        if total == 0 || target_line == 0 {
            return Self {
                file_path: None,
                target_line,
                lines: Vec::new(),
            };
        }

        let start_idx = target_line.saturating_sub(context_lines + 1);
        let end_idx = (target_line + context_lines).min(total);

        let mut lines = Vec::new();
        for (i, line_content) in all_lines.iter().enumerate().take(end_idx).skip(start_idx) {
            let line_num = i + 1;
            lines.push(SourceLine {
                line_number: line_num,
                content: (*line_content).to_string(),
                is_highlight: line_num == target_line,
            });
        }

        Self {
            file_path: None,
            target_line,
            lines,
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            let marker = if line.is_highlight { ">" } else { " " };
            out.push_str(&format!(
                "  {} {:4} | {}\n",
                marker, line.line_number, line.content
            ));
        }
        out
    }
}
