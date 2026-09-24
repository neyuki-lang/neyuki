// Lexical token definitions and tokenization interface for Neyuki.

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

pub use crate::parser::tokenizer::{DocComment, TokenizerEngine};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: &'static str,
    pub value: String,
    pub line: usize,
    pub col: usize,
}

pub struct Lexer {
    src: String,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            src: source.to_owned(),
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut engine = TokenizerEngine::new(&self.src);
        engine.tokenize()
    }

    pub fn tokenize_with_comments(&mut self) -> (Vec<Token>, Vec<DocComment>) {
        let mut engine = TokenizerEngine::new(&self.src);
        engine.tokenize_with_comments()
    }
}

pub fn is_keyword(word: &str) -> bool {
    crate::parser::tokenizer::WordEngine::is_keyword(word)
}
