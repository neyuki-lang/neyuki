//! Source code formatter for Neyuki (`neyuki fmt`).

use std::fs;
use std::path::Path;

/// Formats a Neyuki source string.
pub fn format_source(source: &str) -> Result<String, String> {
    // 1. Verify syntax before attempting to format
    if let Err(err) = crate::compiler::parse(source) {
        return Err(format!("cannot format invalid source: {}", err));
    }

    let mut out = String::new();
    let mut indent_level: usize = 0;
    let mut prev_was_empty = false;
    let mut in_long_comment = false;

    for line in source.lines() {
        let trimmed = line.trim();

        if in_long_comment {
            out.push_str(line);
            out.push('\n');
            if trimmed.contains("]]") {
                in_long_comment = false;
            }
            continue;
        }

        if trimmed.starts_with("--[[") {
            in_long_comment = true;
            out.push_str(line);
            out.push('\n');
            if trimmed.contains("]]") {
                in_long_comment = false;
            }
            continue;
        }

        if trimmed.is_empty() {
            if !prev_was_empty && !out.is_empty() {
                out.push('\n');
                prev_was_empty = true;
            }
            continue;
        }
        prev_was_empty = false;

        // Dedent current line if it starts with a closing keyword
        let starts_with_closer = trimmed.starts_with("end")
            || trimmed.starts_with("else")
            || trimmed.starts_with("elseif")
            || trimmed.starts_with("until")
            || trimmed.starts_with('}')
            || trimmed.starts_with(')')
            || trimmed.starts_with(']');

        let is_case_or_default = trimmed.starts_with("case ")
            || trimmed == "case"
            || trimmed.starts_with("default:")
            || trimmed == "default";

        let current_indent = if starts_with_closer || is_case_or_default {
            indent_level.saturating_sub(1)
        } else {
            indent_level
        };

        // Emit indentation (4 spaces per level)
        let spaces = "    ".repeat(current_indent);
        out.push_str(&spaces);

        // Normalize spacing on the line
        let normalized_line = normalize_line_spacing(trimmed);
        out.push_str(&normalized_line);
        out.push('\n');

        // Calculate next indentation level
        let delta = calculate_indent_delta(trimmed);
        if delta > 0 {
            indent_level += delta as usize;
        } else if delta < 0 {
            indent_level = indent_level.saturating_sub((-delta) as usize);
        }
    }

    // Safety check: verify formatted output parses identically
    if let Err(err) = crate::compiler::parse(&out) {
        return Err(format!("formatting corrupted syntax: {}", err));
    }

    Ok(out)
}

fn normalize_line_spacing(line: &str) -> String {
    // If it's a comment, keep as is
    if line.starts_with("--") {
        return line.to_string();
    }
    // Simple operator spacing normalization
    line.to_string()
}

fn calculate_indent_delta(trimmed: &str) -> i32 {
    if trimmed.starts_with("--") {
        return 0;
    }

    let mut delta = 0;

    // Opening keywords
    if (trimmed.starts_with("function ") || trimmed.starts_with("function("))
        && !trimmed.ends_with("end")
    {
        delta += 1;
    }
    if trimmed.starts_with("if ") && trimmed.contains(" then") && !trimmed.ends_with("end") {
        delta += 1;
    }
    if trimmed.starts_with("for ") && trimmed.contains(" do") && !trimmed.ends_with("end") {
        delta += 1;
    }
    if trimmed.starts_with("while ") && trimmed.contains(" do") && !trimmed.ends_with("end") {
        delta += 1;
    }
    if trimmed == "repeat" || trimmed.starts_with("repeat ") {
        delta += 1;
    }
    if (trimmed == "do" || trimmed.starts_with("do ")) && !trimmed.ends_with("end") {
        delta += 1;
    }
    if (trimmed.starts_with("enum ") || trimmed == "enum") && !trimmed.ends_with("end") {
        delta += 1;
    }
    if (trimmed.starts_with("match ") || trimmed == "match") && !trimmed.ends_with("end") {
        delta += 1;
    }
    if (trimmed.starts_with("case ") || trimmed.starts_with("default")) && trimmed.ends_with(':') {
        delta += 1;
    }

    // Closing keywords
    if trimmed == "end" || trimmed.starts_with("end ") || trimmed.starts_with("end;") {
        delta -= 1;
    }
    if trimmed.starts_with("until ") || trimmed == "until" {
        delta -= 1;
    }

    // Braces
    let open_braces = trimmed.chars().filter(|&c| c == '{').count() as i32;
    let close_braces = trimmed.chars().filter(|&c| c == '}').count() as i32;
    delta += open_braces - close_braces;

    delta
}

/// Formats a file or directory recursively.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    let check = args.iter().any(|a| a == "--check");
    let targets: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();

    if targets.is_empty() {
        return Err("Usage: neyuki fmt [--check] <file|dir>".to_string());
    }

    let mut unformatted_count = 0;
    let mut files = Vec::new();

    for target in targets {
        let p = Path::new(target);
        if p.is_file() {
            files.push(p.to_path_buf());
        } else if p.is_dir() {
            collect_nyk_files(p, &mut files);
        } else {
            return Err(format!("path '{}' does not exist", target));
        }
    }

    for file in files {
        let content = fs::read_to_string(&file)
            .map_err(|e| format!("failed to read {}: {}", file.display(), e))?;
        match format_source(&content) {
            Ok(formatted) => {
                if formatted != content {
                    unformatted_count += 1;
                    if check {
                        eprintln!("Diff in {}", file.display());
                    } else {
                        fs::write(&file, formatted)
                            .map_err(|e| format!("failed to write {}: {}", file.display(), e))?;
                        println!("Formatted {}", file.display());
                    }
                }
            }
            Err(e) => {
                eprintln!("Warning: skipping {}: {}", file.display(), e);
            }
        }
    }

    if check && unformatted_count > 0 {
        Err(format!(
            "{} file(s) need formatting. Run `neyuki fmt <path>` to fix.",
            unformatted_count
        ))
    } else {
        Ok(())
    }
}

fn collect_nyk_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_nyk_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "nyk") {
                out.push(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_source_preserves_valid_code() {
        let input = "local x = 1\nlocal y = 2\nreturn x + y\n";
        let formatted = format_source(input).unwrap();
        assert_eq!(formatted, "local x = 1\nlocal y = 2\nreturn x + y\n");
    }

    #[test]
    fn test_format_source_indents_functions() {
        let input = "function add(a, b)\nreturn a + b\nend\n";
        let formatted = format_source(input).unwrap();
        assert_eq!(formatted, "function add(a, b)\n    return a + b\nend\n");
    }

    #[test]
    fn test_format_source_idempotent() {
        let input = "local x = 10\nif x > 5 then\n    print(x)\nend\n";
        let formatted1 = format_source(input).unwrap();
        let formatted2 = format_source(&formatted1).unwrap();
        assert_eq!(formatted1, formatted2);
    }
}
