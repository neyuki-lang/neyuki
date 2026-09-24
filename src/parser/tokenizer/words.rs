// Keyword and identifier scanning engine for Neyuki tokenizer.

use super::scanner::Scanner;

pub struct WordEngine;

impl WordEngine {
    #[inline(always)]
    pub fn is_identifier_start(ch: char) -> bool {
        ch.is_ascii_alphabetic() || ch == '_'
    }

    #[inline(always)]
    pub fn is_identifier_continue(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_'
    }

    pub fn scan_word<'a>(scanner: &mut Scanner<'a>) -> &'a str {
        let start = scanner.pos();
        while !scanner.is_at_end() && Self::is_identifier_continue(scanner.peek()) {
            scanner.advance();
        }
        scanner.slice_from(start)
    }

    #[inline]
    pub fn is_keyword(word: &str) -> bool {
        matches!(
            word,
            "and"
                | "break"
                | "const"
                | "continue"
                | "defer"
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

    #[inline]
    pub fn classify_word(word: &str) -> &'static str {
        if Self::is_keyword(word) {
            "keyword"
        } else {
            "name"
        }
    }
}
