// String and literal scanning engine for Neyuki tokenizer.

use super::scanner::Scanner;

pub struct StringEngine;

impl StringEngine {
    #[inline(always)]
    pub fn is_string_start(ch: char) -> bool {
        ch == '"' || ch == '\''
    }

    #[inline(always)]
    pub fn is_long_bracket_start(scanner: &Scanner<'_>) -> bool {
        scanner.peek() == '[' && scanner.peek_next().is_some_and(|c| c == '[' || c == '=')
    }

    #[inline(always)]
    pub fn is_interp_start(ch: char) -> bool {
        ch == '`'
    }

    pub fn scan_quoted_string(scanner: &mut Scanner<'_>, quote: char) -> Result<String, String> {
        scanner.advance(); // consume opening quote
        let mut out = String::new();

        while !scanner.is_at_end() {
            let ch = scanner.advance();
            if ch == quote {
                return Ok(out);
            }
            if ch == '\\' {
                if scanner.is_at_end() {
                    return Err("Unfinished string escape at EOF".to_string());
                }
                let esc = scanner.advance();
                match esc {
                    'a' => out.push('\x07'),
                    'b' => out.push('\x08'),
                    'f' => out.push('\x0C'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'v' => out.push('\x0B'),
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    '0' => out.push('\0'),
                    'x' => {
                        let hi = scanner.advance();
                        let lo = scanner.advance();
                        let hex = format!("{}{}", hi, lo);
                        let value = u8::from_str_radix(&hex, 16).unwrap_or(0);
                        out.push(value as char);
                    }
                    'u' if scanner.peek() == '{' => {
                        scanner.advance();
                        let mut digits = String::new();
                        while !scanner.is_at_end() && scanner.peek() != '}' {
                            digits.push(scanner.advance());
                        }
                        if scanner.peek() == '}' {
                            scanner.advance();
                        }
                        let code = u32::from_str_radix(&digits, 16).unwrap_or(0);
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                        }
                    }
                    'z' => {
                        while !scanner.is_at_end()
                            && matches!(scanner.peek(), ' ' | '\t' | '\r' | '\n')
                        {
                            scanner.advance();
                        }
                    }
                    _ => out.push(esc),
                }
            } else {
                out.push(ch);
            }
        }

        Err("Unfinished string literal".to_string())
    }

    pub fn scan_long_bracket(scanner: &mut Scanner<'_>) -> Result<String, String> {
        let start_pos = scanner.current_pos();
        if scanner.peek() != '[' {
            return Err("Long bracket expected".to_string());
        }
        scanner.advance();

        let mut level = 0;
        while !scanner.is_at_end() && scanner.peek() == '=' {
            level += 1;
            scanner.advance();
        }

        if scanner.peek() != '[' {
            scanner.restore(start_pos);
            return Err("Long bracket expected".to_string());
        }
        scanner.advance();

        // Skip immediately following newline if any
        if scanner.peek() == '\r' {
            scanner.advance();
        }
        if scanner.peek() == '\n' {
            scanner.advance();
        }

        let closer = format!("]{}]", "=".repeat(level));
        let content_start = scanner.pos();
        let mut found = None;

        while !scanner.is_at_end() {
            if scanner.starts_with(&closer) {
                found = Some(scanner.pos());
                break;
            }
            scanner.advance();
        }

        let Some(index) = found else {
            scanner.restore(start_pos);
            return Err("Unfinished long bracket".to_string());
        };

        let content = scanner.slice_from(content_start)[..index - content_start].to_owned();
        scanner.advance_bytes(closer.len());
        Ok(content)
    }

    pub fn scan_interpolated_string(scanner: &mut Scanner<'_>) -> Result<String, String> {
        scanner.advance(); // consume opening backtick
        let mut out = String::new();

        while !scanner.is_at_end() {
            let ch = scanner.advance();
            if ch == '`' {
                return Ok(out);
            }
            if ch == '\\' {
                if !scanner.is_at_end() {
                    let next = scanner.advance();
                    out.push(next);
                }
            } else if ch == '{' {
                let mut depth = 1;
                let mut expr = String::new();
                while !scanner.is_at_end() && depth > 0 {
                    let c = scanner.advance();
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    if depth > 0 {
                        expr.push(c);
                    }
                }
                out.push('{');
                out.push_str(&expr);
                out.push('}');
            } else {
                out.push(ch);
            }
        }

        Err("Unfinished interpolated string".to_string())
    }
}
