//! Source code formatter for Neyuki (`neyuki fmt`).
//!
//! # Architecture and Limitations
//!
//! The formatter currently operates using a line-oriented indentation engine with
//! block tracking. It preserves raw multiline strings (`[=*[...]=*]`) and multiline
//! comments (`--[=*[...]=*]`) verbatim.
//!
//! ## Current Limitations:
//! - Multi-line statement headers (e.g., an `if` condition whose `then` appears on a
//!   later line, or a `for` header whose `do` appears on a later line) are formatted
//!   based on line heuristics rather than a full token/AST-driven layout engine.
//! - Whitespace inside single-line expressions is preserved to protect custom tabular
//!   alignments.
//! - Any formatted output is verified against the compiler parser to ensure that the
//!   resulting Abstract Syntax Tree (AST) is strictly identical to the input.

use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockKind {
    Match { has_case: bool },
    Normal,
}

/// Checks whether a line starts with a specific keyword at a word boundary.
fn starts_with_keyword(line: &str, kw: &str) -> bool {
    if !line.starts_with(kw) {
        return false;
    }
    let rest = &line[kw.len()..];
    rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
}

/// Formats a Neyuki source string.
pub fn format_source(source: &str) -> Result<String, String> {
    // 1. Verify syntax before attempting to format
    crate::compiler::parse(source)
        .map_err(|err| format!("cannot format invalid source: {}", err))?;

    let mut out = String::new();
    let mut prev_was_empty = false;
    let mut multiline_closer: Option<String> = None;
    let mut block_stack: Vec<BlockKind> = Vec::new();
    let mut indent_level: usize = 0;

    for line in source.lines() {
        // If inside a multiline raw string or long comment, preserve verbatim
        if let Some(ref closer) = multiline_closer {
            out.push_str(line);
            out.push('\n');
            if let Some(pos) = line.find(closer) {
                let after = &line[pos + closer.len()..];
                multiline_closer = find_unclosed_multiline(after);
            }
            continue;
        }

        let trimmed = line.trim();

        if trimmed.is_empty() {
            if !prev_was_empty && !out.is_empty() {
                out.push('\n');
                prev_was_empty = true;
            }
            continue;
        }
        prev_was_empty = false;

        let stripped = strip_strings_and_comments(trimmed);

        let is_case_or_default = (starts_with_keyword(trimmed, "case")
            || starts_with_keyword(trimmed, "default"))
            && trimmed.ends_with(':');

        let starts_with_closer = starts_with_keyword(trimmed, "end")
            || starts_with_keyword(trimmed, "until")
            || trimmed.starts_with('}')
            || trimmed.starts_with(')')
            || trimmed.starts_with(']');

        let is_else_or_elseif =
            starts_with_keyword(trimmed, "else") || starts_with_keyword(trimmed, "elseif");

        // Calculate current line indentation:
        let current_indent = if is_case_or_default {
            if let Some(BlockKind::Match { has_case: true }) = block_stack.last() {
                indent_level.saturating_sub(1)
            } else {
                indent_level
            }
        } else if starts_with_closer {
            if starts_with_keyword(trimmed, "end")
                && matches!(
                    block_stack.last(),
                    Some(BlockKind::Match { has_case: true })
                )
            {
                indent_level.saturating_sub(2)
            } else {
                indent_level.saturating_sub(1)
            }
        } else if is_else_or_elseif {
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

        // Check if a multiline raw string or comment started on this line
        if let Some(closer) = find_unclosed_multiline(trimmed) {
            multiline_closer = Some(closer);
        }

        // Update block stack and indentation for subsequent lines
        if is_case_or_default {
            if let Some(BlockKind::Match { has_case }) = block_stack.last_mut()
                && !*has_case
            {
                *has_case = true;
                indent_level += 1;
            }
        } else {
            // Check closing keywords first
            if starts_with_keyword(trimmed, "end") {
                if let Some(top) = block_stack.pop() {
                    match top {
                        BlockKind::Match { has_case: true } => {
                            indent_level = indent_level.saturating_sub(2);
                        }
                        _ => {
                            indent_level = indent_level.saturating_sub(1);
                        }
                    }
                }
            } else if starts_with_keyword(trimmed, "until") {
                block_stack.pop();
                indent_level = indent_level.saturating_sub(1);
            }

            let opens_normal_block = (has_word(&stripped, "function")
                && !has_word(&stripped, "end"))
                || (has_word(&stripped, "if")
                    && !has_word(&stripped, "elseif")
                    && has_word(&stripped, "then")
                    && !has_word(&stripped, "end"))
                || (has_word(&stripped, "for")
                    && has_word(&stripped, "do")
                    && !has_word(&stripped, "end"))
                || (has_word(&stripped, "while")
                    && has_word(&stripped, "do")
                    && !has_word(&stripped, "end"))
                || (starts_with_keyword(trimmed, "do") && !has_word(&stripped, "end"))
                || (starts_with_keyword(trimmed, "repeat") && !has_word(&stripped, "until"))
                || (starts_with_keyword(trimmed, "enum") && !has_word(&stripped, "end"));

            let is_match_block = starts_with_keyword(trimmed, "match")
                && !trimmed["match".len()..].trim_start().starts_with('=')
                && !has_word(&stripped, "end");

            if is_match_block {
                block_stack.push(BlockKind::Match { has_case: false });
                indent_level += 1;
            } else if opens_normal_block {
                block_stack.push(BlockKind::Normal);
                indent_level += 1;
            }

            // Braces inside stripped code (safe from strings & comments)
            let open_braces = stripped.chars().filter(|&c| c == '{').count();
            let close_braces = stripped.chars().filter(|&c| c == '}').count();
            if open_braces > close_braces {
                for _ in 0..(open_braces - close_braces) {
                    block_stack.push(BlockKind::Normal);
                    indent_level += 1;
                }
            } else if close_braces > open_braces {
                for _ in 0..(close_braces - open_braces) {
                    block_stack.pop();
                    indent_level = indent_level.saturating_sub(1);
                }
            }
        }
    }

    // Safety check: verify formatted output preserves identical tokens and valid syntax
    crate::compiler::parse(&out).map_err(|err| format!("formatting corrupted syntax: {}", err))?;

    let tokens_before: Vec<(&'static str, String)> = crate::lexer::Lexer::new(source)
        .tokenize()
        .map_err(|err| format!("cannot tokenize source: {}", err))?
        .into_iter()
        .map(|t| (t.kind, t.value))
        .collect();

    let tokens_after: Vec<(&'static str, String)> = crate::lexer::Lexer::new(&out)
        .tokenize()
        .map_err(|err| format!("formatting corrupted tokens: {}", err))?
        .into_iter()
        .map(|t| (t.kind, t.value))
        .collect();

    if tokens_before != tokens_after {
        return Err("formatting altered program tokens".to_string());
    }

    Ok(out)
}

fn find_unclosed_multiline(line: &str) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                } else if chars[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if chars[i] == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            let j = i + 2;
            if j < chars.len() && chars[j] == '[' {
                let eq_start = j + 1;
                let mut eq_end = eq_start;
                while eq_end < chars.len() && chars[eq_end] == '=' {
                    eq_end += 1;
                }
                if eq_end < chars.len() && chars[eq_end] == '[' {
                    let eq_count = eq_end - eq_start;
                    let closer = format!("]{}]", "=".repeat(eq_count));
                    let rest: String = chars[eq_end + 1..].iter().collect();
                    if let Some(pos) = rest.find(&closer) {
                        i = eq_end + 1 + pos + closer.len();
                        continue;
                    } else {
                        return Some(closer);
                    }
                }
            }
            return None;
        }
        if chars[i] == '[' && i + 1 < chars.len() {
            let eq_start = i + 1;
            let mut eq_end = eq_start;
            while eq_end < chars.len() && chars[eq_end] == '=' {
                eq_end += 1;
            }
            if eq_end < chars.len() && chars[eq_end] == '[' {
                let eq_count = eq_end - eq_start;
                let closer = format!("]{}]", "=".repeat(eq_count));
                let rest: String = chars[eq_end + 1..].iter().collect();
                if let Some(pos) = rest.find(&closer) {
                    i = eq_end + 1 + pos + closer.len();
                    continue;
                } else {
                    return Some(closer);
                }
            }
        }
        i += 1;
    }
    None
}

fn strip_strings_and_comments(line: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            break;
        }
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                } else if chars[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if chars[i] == '[' && i + 1 < chars.len() {
            let mut j = i + 1;
            while j < chars.len() && chars[j] == '=' {
                j += 1;
            }
            if j < chars.len() && chars[j] == '[' {
                let eq_count = j - (i + 1);
                let closer = format!("]{}]", "=".repeat(eq_count));
                let rest: String = chars[j + 1..].iter().collect();
                if let Some(pos) = rest.find(&closer) {
                    i = j + 1 + pos + closer.len();
                    continue;
                } else {
                    break;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn has_word(s: &str, word: &str) -> bool {
    let bytes = s.as_bytes();
    let word_bytes = word.as_bytes();
    let mut pos = 0;
    while pos + word_bytes.len() <= bytes.len() {
        if let Some(idx) = s[pos..].find(word) {
            let start = pos + idx;
            let end = start + word_bytes.len();
            let before_ok = start == 0
                || (!bytes[start - 1].is_ascii_alphanumeric() && bytes[start - 1] != b'_');
            let after_ok =
                end == bytes.len() || (!bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_');
            if before_ok && after_ok {
                return true;
            }
            pos = start + 1;
        } else {
            break;
        }
    }
    false
}

fn normalize_line_spacing(line: &str) -> String {
    // If it's a comment, keep as is
    if line.starts_with("--") {
        return line.to_string();
    }
    line.to_string()
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
    fn test_format_source_indents_local_functions_and_assigned_functions() {
        let input = "local function add(a, b)\nreturn a + b\nend\nlocal f = function(x)\nreturn x * 2\nend\n";
        let formatted = format_source(input).unwrap();
        assert_eq!(
            formatted,
            "local function add(a, b)\n    return a + b\nend\nlocal f = function(x)\n    return x * 2\nend\n"
        );
    }

    #[test]
    fn test_format_source_preserves_multiline_raw_strings() {
        let input = "local s = [[\nfirst line\n    indented line\nlast line\n]]\nlocal x = 1\n";
        let formatted = format_source(input).unwrap();
        assert_eq!(formatted, input);
    }

    #[test]
    fn test_format_source_ignores_braces_in_strings_and_comments() {
        let input = "local s = \"{foo}\"\n-- comment with {bar}\nlocal t = {\na = 1,\n}\n";
        let formatted = format_source(input).unwrap();
        assert_eq!(
            formatted,
            "local s = \"{foo}\"\n-- comment with {bar}\nlocal t = {\n    a = 1,\n}\n"
        );
    }

    #[test]
    fn test_format_source_match_case_no_staircase() {
        let input = "match c\ncase 1:\nname = \"one\"\ncase 2:\nname = \"two\"\ndefault:\nname = \"other\"\nend\nlocal done = true\n";
        let formatted = format_source(input).unwrap();
        let expected = "match c\n    case 1:\n        name = \"one\"\n    case 2:\n        name = \"two\"\n    default:\n        name = \"other\"\nend\nlocal done = true\n";
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_format_source_idempotent() {
        let input = "local x = 10\nif x > 5 then\n    print(x)\nend\n";
        let formatted1 = format_source(input).unwrap();
        let formatted2 = format_source(&formatted1).unwrap();
        assert_eq!(formatted1, formatted2);
    }

    #[test]
    fn test_format_source_identifiers_with_keyword_prefixes() {
        let input = "function test()\nendpoint = 1\nending = 2\nelsewhere = 3\nuntil_done = 4\nreturn endpoint\nend\n";
        let formatted = format_source(input).unwrap();
        let expected = "function test()\n    endpoint = 1\n    ending = 2\n    elsewhere = 3\n    until_done = 4\n    return endpoint\nend\n";
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_format_source_local_match_and_assignment_not_block() {
        let input =
            "function test()\nlocal match = \"regex\"\nmatch = \"updated\"\nreturn match\nend\n";
        let formatted = format_source(input).unwrap();
        let expected = "function test()\n    local match = \"regex\"\n    match = \"updated\"\n    return match\nend\n";
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_format_source_multiline_comments_midline_and_level_matching() {
        let input = "local x = 1 --[[\n    first comment line\n    second comment line\n]]\nlocal y = 2 --[==[\n    comment with ]] inside\n]==]\nlocal z = 3\n";
        let formatted = format_source(input).unwrap();
        assert_eq!(formatted, input);
    }
}
