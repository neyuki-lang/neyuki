#[cfg(test)]
mod tests {
    use super::Lexer;

    #[test]
    fn tokenizes_keywords_numbers_and_strings() {
        let mut lexer = Lexer::new("local x = 42\nif x != 0 then\n  print(\"ok\")\nend");
        let tokens = lexer.tokenize();

        assert_eq!(tokens[0].kind, "keyword");
        assert_eq!(tokens[0].value, "local");
        assert_eq!(tokens[1].kind, "name");
        assert_eq!(tokens[1].value, "x");
        assert_eq!(tokens[2].kind, "symbol");
        assert_eq!(tokens[2].value, "=");
        assert_eq!(tokens[3].kind, "number");
        assert_eq!(tokens[3].value, "42");
        assert_eq!(tokens[4].kind, "keyword");
        assert_eq!(tokens[4].value, "if");
        assert_eq!(tokens[5].kind, "name");
        assert_eq!(tokens[5].value, "x");
        assert_eq!(tokens[6].kind, "symbol");
        assert_eq!(tokens[6].value, "!=");
        assert_eq!(tokens[7].kind, "number");
        assert_eq!(tokens[7].value, "0");
        assert_eq!(tokens[8].kind, "keyword");
        assert_eq!(tokens[8].value, "then");
        assert_eq!(tokens[11].kind, "string");
        assert_eq!(tokens[11].value, "ok");
    }

    #[test]
    fn reads_long_bracket_strings() {
        let mut lexer = Lexer::new("local s = [[hello\nworld]]");
        let tokens = lexer.tokenize();
        assert_eq!(tokens[3].kind, "string");
        assert_eq!(tokens[3].value, "hello\nworld");
    }

    #[test]
    fn tracks_line_and_column() {
        let mut lexer = Lexer::new("local x = 42\nif");
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].line, 1);
        assert_eq!(tokens[0].col, 1); // "local" starts at col 1
        assert_eq!(tokens[1].col, 7); // "x" starts at col 7
        assert_eq!(tokens[2].col, 9); // "=" at col 9
        assert_eq!(tokens[3].col, 11); // "42" at col 11
        assert_eq!(tokens[4].line, 2);
        assert_eq!(tokens[4].col, 1); // "if" starts at col 1 of line 2
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: &'static str,
    pub value: String,
    pub line: usize,
    pub col: usize,
}

pub struct Lexer {
    src: String,
    pos: usize,
    line: usize,
    col: usize,
    len: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            src: source.to_owned(),
            pos: 0,
            line: 1,
            col: 1,
            len: source.len(),
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        while !self.at_end() {
            self.skip_whitespace_and_comments();
            if self.at_end() {
                break;
            }

            let start_col = self.col;
            let ch = self.peek();
            if ch.is_ascii_alphabetic() || ch == '_' {
                let value = self.read_name();
                let kind = if is_keyword(&value) {
                    "keyword"
                } else {
                    "name"
                };
                tokens.push(Token {
                    kind,
                    value,
                    line: self.line,
                    col: start_col,
                });
            } else if ch.is_ascii_digit()
                || (ch == '.' && self.peek_next().is_some_and(|c| c.is_ascii_digit()))
            {
                let value = self.read_number();
                tokens.push(Token {
                    kind: "number",
                    value,
                    line: self.line,
                    col: start_col,
                });
            } else if ch == '"' || ch == '\'' {
                let value = self.read_string(ch);
                tokens.push(Token {
                    kind: "string",
                    value,
                    line: self.line,
                    col: start_col,
                });
            } else if ch == '[' && self.peek_next().is_some_and(|c| c == '[' || c == '=') {
                let value = self.read_long_bracket();
                tokens.push(Token {
                    kind: "string",
                    value,
                    line: self.line,
                    col: start_col,
                });
            } else if ch == '`' {
                let value = self.read_interpolated_string();
                tokens.push(Token {
                    kind: "interp",
                    value,
                    line: self.line,
                    col: start_col,
                });
            } else {
                let value = self.read_symbol();
                if value.is_empty() {
                    panic!("Unexpected character {:?} at line {}", ch, self.line);
                }
                tokens.push(Token {
                    kind: "symbol",
                    value,
                    line: self.line,
                    col: start_col,
                });
            }
        }

        tokens.push(Token {
            kind: "eof",
            value: String::new(),
            line: self.line,
            col: self.col,
        });

        tokens
    }

    fn at_end(&self) -> bool {
        self.pos >= self.len
    }

    fn peek(&self) -> char {
        self.src[self.pos..].chars().next().unwrap_or('\0')
    }

    fn peek_next(&self) -> Option<char> {
        let mut chars = self.src[self.pos..].chars();
        let _ = chars.next()?;
        chars.next()
    }

    fn advance(&mut self) -> char {
        let ch = self.peek();
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    fn skip_whitespace_and_comments(&mut self) {
        while !self.at_end() {
            let ch = self.peek();
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    self.advance();
                }
                '-' if self.peek_next() == Some('-') && self.can_start_comment() => {
                    self.advance();
                    self.advance();
                    if self.peek() == '[' {
                        let saved = self.pos;
                        let saved_line = self.line;
                        let saved_col = self.col;
                        if self.read_long_bracket_opt().is_none() {
                            self.pos = saved;
                            self.line = saved_line;
                            self.col = saved_col;
                            while !self.at_end() && self.peek() != '\n' {
                                self.advance();
                            }
                        }
                    } else {
                        while !self.at_end() && self.peek() != '\n' {
                            self.advance();
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn read_name(&mut self) -> String {
        let start = self.pos;
        while !self.at_end() {
            let ch = self.peek();
            if ch.is_ascii_alphanumeric() || ch == '_' {
                self.advance();
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_owned()
    }

    fn can_start_comment(&self) -> bool {
        self.pos == 0
            || self.src[..self.pos]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
    }

    fn read_number(&mut self) -> String {
        let start = self.pos;
        if self.peek() == '0' && matches!(self.peek_next(), Some('x' | 'X')) {
            self.advance();
            self.advance();
            while !self.at_end() && (self.peek().is_ascii_hexdigit() || self.peek() == '_') {
                self.advance();
            }
            return self.src[start..self.pos].to_owned();
        }

        if self.peek() == '0' && matches!(self.peek_next(), Some('b' | 'B')) {
            self.advance();
            self.advance();
            while !self.at_end() && matches!(self.peek(), '0' | '1' | '_') {
                self.advance();
            }
            return self.src[start..self.pos].to_owned();
        }

        while !self.at_end() && (self.peek().is_ascii_digit() || self.peek() == '_') {
            self.advance();
        }

        if self.peek() == '.' {
            self.advance();
            while !self.at_end() && (self.peek().is_ascii_digit() || self.peek() == '_') {
                self.advance();
            }
        }

        if matches!(self.peek(), 'e' | 'E') {
            self.advance();
            if matches!(self.peek(), '+' | '-') {
                self.advance();
            }
            while !self.at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }

        self.src[start..self.pos].to_owned()
    }

    fn read_string(&mut self, quote: char) -> String {
        self.advance();
        let mut out = String::new();
        while !self.at_end() {
            let ch = self.advance();
            if ch == quote {
                return out;
            }
            if ch == '\\' {
                let esc = self.advance();
                match esc {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    '0' => out.push('\0'),
                    'x' => {
                        let hi = self.advance();
                        let lo = self.advance();
                        let hex = format!("{}{}", hi, lo);
                        let value = u8::from_str_radix(&hex, 16).unwrap_or(0);
                        out.push(value as char);
                    }
                    'u' if self.peek() == '{' => {
                        self.advance();
                        let mut digits = String::new();
                        while !self.at_end() && self.peek() != '}' {
                            digits.push(self.advance());
                        }
                        if self.peek() == '}' {
                            self.advance();
                        }
                        let code = u32::from_str_radix(&digits, 16).unwrap_or(0);
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                        }
                    }
                    'z' => {
                        while !self.at_end() && matches!(self.peek(), ' ' | '\t' | '\r' | '\n') {
                            self.advance();
                        }
                    }
                    _ => out.push(esc),
                }
            } else {
                out.push(ch);
            }
        }
        panic!("Unfinished string literal");
    }

    fn read_long_bracket(&mut self) -> String {
        let start = self.pos;
        if self.peek() != '[' {
            panic!("Long bracket expected");
        }
        self.advance();

        let mut level = 0;
        while !self.at_end() && self.peek() == '=' {
            level += 1;
            self.advance();
        }
        if self.peek() != '[' {
            self.pos = start;
            panic!("Long bracket expected");
        }
        self.advance();

        if self.peek() == '\r' {
            self.advance();
        }
        if self.peek() == '\n' {
            self.advance();
        }

        let closer = format!("]{}]", "=".repeat(level));
        let content_start = self.pos;
        let mut found = None;
        while !self.at_end() {
            if self.src[self.pos..].starts_with(&closer) {
                found = Some(self.pos);
                break;
            }
            self.advance();
        }
        let Some(index) = found else {
            self.pos = start;
            panic!("Unfinished long bracket");
        };
        let content = self.src[content_start..index].to_owned();
        self.pos = index + closer.len();
        self.col += closer.len();
        content
    }

    fn read_long_bracket_opt(&mut self) -> Option<String> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.col;
        if self.peek() != '[' {
            return None;
        }
        self.advance();

        let mut level = 0;
        while !self.at_end() && self.peek() == '=' {
            level += 1;
            self.advance();
        }
        if self.peek() != '[' {
            self.pos = start;
            self.line = start_line;
            self.col = start_col;
            return None;
        }
        self.advance();

        if self.peek() == '\r' {
            self.advance();
        }
        if self.peek() == '\n' {
            self.advance();
        }

        let closer = format!("]{}]", "=".repeat(level));
        let content_start = self.pos;
        let mut found = None;
        while !self.at_end() {
            if self.src[self.pos..].starts_with(&closer) {
                found = Some(self.pos);
                break;
            }
            self.advance();
        }
        if let Some(index) = found {
            let content = self.src[content_start..index].to_owned();
            self.pos = index + closer.len();
            self.col += closer.len();
            Some(content)
        } else {
            self.pos = start;
            self.line = start_line;
            self.col = start_col;
            None
        }
    }

    fn read_interpolated_string(&mut self) -> String {
        self.advance();
        let mut out = String::new();
        while !self.at_end() {
            let ch = self.advance();
            if ch == '`' {
                return out;
            }
            if ch == '\\' {
                if let Some(next) = self.src[self.pos..].chars().next() {
                    out.push(next);
                    self.advance();
                }
            } else if ch == '{' {
                let mut depth = 1;
                let mut expr = String::new();
                while !self.at_end() && depth > 0 {
                    let c = self.advance();
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
        panic!("Unfinished interpolated string");
    }

    fn read_symbol(&mut self) -> String {
        let start = self.pos;
        let candidates = [
            "<<<=", ">>>=", "??=", "<<<", ">>>", "<<=", ">>=", "==", "!=", "~=", "<=", ">=", "??",
            "..=", "//=", "::", "->", "...", "..", "+=", "-=", "*=", "/=", "%=", "^=", "<<", ">>",
            "//", "++", "--", "&=", "|=", "&", "|", "~", "^", "?", ";", ":", ",", ".", "=", "+",
            "-", "*", "/", "%", "#", "<", ">", "(", ")", "{", "}", "[", "]",
        ];

        for candidate in candidates {
            if self.src[start..].starts_with(candidate) {
                self.pos += candidate.len();
                self.col += candidate.len();
                return candidate.to_owned();
            }
        }

        let ch = self.advance();
        ch.to_string()
    }
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "and"
            | "break"
            | "const"
            | "continue"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "global"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "type"
            | "until"
            | "while"
    )
}
