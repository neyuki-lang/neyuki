// Fast cursor and position scanning engine for Neyuki tokenizer.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePos {
    pub offset: usize,
    pub line: usize,
    pub col: usize,
}

impl SourcePos {
    pub fn new(offset: usize, line: usize, col: usize) -> Self {
        Self { offset, line, col }
    }
}

pub struct Scanner<'a> {
    src: &'a str,
    pos: usize,
    line: usize,
    col: usize,
    len: usize,
}

impl<'a> Scanner<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            src: source,
            pos: 0,
            line: 1,
            col: 1,
            len: source.len(),
        }
    }

    #[inline(always)]
    pub fn is_at_end(&self) -> bool {
        self.pos >= self.len
    }

    #[inline(always)]
    pub fn pos(&self) -> usize {
        self.pos
    }

    #[inline(always)]
    pub fn line(&self) -> usize {
        self.line
    }

    #[inline(always)]
    pub fn col(&self) -> usize {
        self.col
    }

    #[inline(always)]
    pub fn current_pos(&self) -> SourcePos {
        SourcePos::new(self.pos, self.line, self.col)
    }

    #[inline(always)]
    pub fn remaining(&self) -> &'a str {
        if self.pos < self.len {
            &self.src[self.pos..]
        } else {
            ""
        }
    }

    #[inline(always)]
    pub fn slice_from(&self, start: usize) -> &'a str {
        &self.src[start..self.pos]
    }

    #[inline(always)]
    pub fn peek(&self) -> char {
        self.remaining().chars().next().unwrap_or('\0')
    }

    #[inline(always)]
    pub fn peek_byte(&self) -> Option<u8> {
        if self.pos < self.len {
            Some(self.src.as_bytes()[self.pos])
        } else {
            None
        }
    }

    pub fn peek_next(&self) -> Option<char> {
        let mut chars = self.remaining().chars();
        let _ = chars.next()?;
        chars.next()
    }

    pub fn advance(&mut self) -> char {
        let ch = self.peek();
        let ch_len = ch.len_utf8();
        self.pos += ch_len;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    pub fn advance_bytes(&mut self, count: usize) {
        for _ in 0..count {
            if self.is_at_end() {
                break;
            }
            self.advance();
        }
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.remaining().starts_with(prefix)
    }

    pub fn skip_whitespace(&mut self) {
        while !self.is_at_end() {
            match self.peek() {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                }
                _ => break,
            }
        }
    }

    pub fn restore(&mut self, pos: SourcePos) {
        self.pos = pos.offset;
        self.line = pos.line;
        self.col = pos.col;
    }
}
