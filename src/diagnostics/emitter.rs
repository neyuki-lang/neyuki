// Terminal formatter rendering point-to-source diagnostics reports with source code context.

use crate::diagnostics::diagnostic::Diagnostic;

pub fn render_report(source_name: &str, source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = source.lines().collect();

    for (i, diag) in diagnostics.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }

        // 1. Header: severity[code]: message
        if let Some(code) = diag.code {
            out.push_str(&format!("{}[{}]: {}\n", diag.severity, code, diag.message));
        } else {
            out.push_str(&format!("{}: {}\n", diag.severity, diag.message));
        }

        // 2. Source file location
        if let Some(label) = &diag.primary_label {
            let line_idx = label.span.start.line as usize;
            let col_idx = label.span.start.column.max(1) as usize;
            let pad = line_idx.to_string().len();

            out.push_str(&format!(
                "{:pad$}--> {}:{}:{}\n",
                "",
                source_name,
                line_idx,
                col_idx,
                pad = pad
            ));
            out.push_str(&format!("{:pad$} |\n", "", pad = pad));

            // Show source line if within bounds
            if line_idx >= 1 && line_idx <= lines.len() {
                let line_content = lines[line_idx - 1];
                out.push_str(&format!(
                    "{:pad$} | {}\n",
                    line_idx,
                    line_content,
                    pad = pad
                ));

                // Caret underline
                let start_col = col_idx.saturating_sub(1);
                let end_col = if label.span.end.line == label.span.start.line {
                    (label.span.end.column as usize).max(col_idx)
                } else {
                    col_idx
                };
                let width = (end_col - start_col).max(1);
                let spaces = " ".repeat(start_col);
                let carets = "^".repeat(width);

                if !label.message.is_empty() {
                    out.push_str(&format!(
                        "{:pad$} | {}{} {}\n",
                        "",
                        spaces,
                        carets,
                        label.message,
                        pad = pad
                    ));
                } else {
                    out.push_str(&format!("{:pad$} | {}{}\n", "", spaces, carets, pad = pad));
                }
            }

            out.push_str(&format!("{:pad$} |\n", "", pad = pad));
        }

        // 3. Secondary labels
        for sec in &diag.secondary_labels {
            let line_idx = sec.span.start.line as usize;
            if line_idx >= 1 && line_idx <= lines.len() {
                let pad = line_idx.to_string().len();
                let line_content = lines[line_idx - 1];
                out.push_str(&format!(
                    "{:pad$} | {}\n",
                    line_idx,
                    line_content,
                    pad = pad
                ));
                let col = sec.span.start.column.max(1) as usize - 1;
                let spaces = " ".repeat(col);
                out.push_str(&format!(
                    "{:pad$} | {}--- {}\n",
                    "",
                    spaces,
                    sec.message,
                    pad = pad
                ));
            }
        }

        // 4. Notes
        for note in &diag.notes {
            out.push_str(&format!("  = note: {}\n", note));
        }

        // 5. Help suggestions
        if let Some(help) = &diag.help {
            out.push_str(&format!("  = help: {}\n", help));
        }
    }

    out
}
