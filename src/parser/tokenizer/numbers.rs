// Numeric scanning engine for Neyuki tokenizer.

use super::scanner::Scanner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberFormat {
    Decimal,
    Hexadecimal,
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberType {
    Integer,
    Float,
}

pub struct NumberResult<'a> {
    pub raw: &'a str,
    pub format: NumberFormat,
    pub num_type: NumberType,
}

pub struct NumberEngine;

impl NumberEngine {
    #[inline(always)]
    pub fn is_number_start(scanner: &Scanner<'_>) -> bool {
        let ch = scanner.peek();
        ch.is_ascii_digit()
            || (ch == '.' && scanner.peek_next().is_some_and(|c| c.is_ascii_digit()))
    }

    pub fn scan_number<'a>(scanner: &mut Scanner<'a>) -> NumberResult<'a> {
        let start = scanner.pos();
        let mut is_float = false;

        // Check hex prefix 0x / 0X
        if scanner.peek() == '0' && matches!(scanner.peek_next(), Some('x' | 'X')) {
            scanner.advance();
            scanner.advance();
            while !scanner.is_at_end()
                && (scanner.peek().is_ascii_hexdigit() || scanner.peek() == '_')
            {
                scanner.advance();
            }
            if scanner.peek() == '.' {
                is_float = true;
                scanner.advance();
                while !scanner.is_at_end()
                    && (scanner.peek().is_ascii_hexdigit() || scanner.peek() == '_')
                {
                    scanner.advance();
                }
            }
            if matches!(scanner.peek(), 'p' | 'P') {
                is_float = true;
                scanner.advance();
                if matches!(scanner.peek(), '+' | '-') {
                    scanner.advance();
                }
                while !scanner.is_at_end() && scanner.peek().is_ascii_digit() {
                    scanner.advance();
                }
            }
            return NumberResult {
                raw: scanner.slice_from(start),
                format: NumberFormat::Hexadecimal,
                num_type: if is_float {
                    NumberType::Float
                } else {
                    NumberType::Integer
                },
            };
        }

        // Check binary prefix 0b / 0B
        if scanner.peek() == '0' && matches!(scanner.peek_next(), Some('b' | 'B')) {
            scanner.advance();
            scanner.advance();
            while !scanner.is_at_end() && matches!(scanner.peek(), '0' | '1' | '_') {
                scanner.advance();
            }
            return NumberResult {
                raw: scanner.slice_from(start),
                format: NumberFormat::Binary,
                num_type: NumberType::Integer,
            };
        }

        // Decimal digits
        while !scanner.is_at_end() && (scanner.peek().is_ascii_digit() || scanner.peek() == '_') {
            scanner.advance();
        }

        // Fractional part
        if scanner.peek() == '.' && scanner.peek_next() != Some('.') {
            is_float = true;
            scanner.advance();
            while !scanner.is_at_end() && (scanner.peek().is_ascii_digit() || scanner.peek() == '_')
            {
                scanner.advance();
            }
        }

        // Exponent part
        if matches!(scanner.peek(), 'e' | 'E') {
            is_float = true;
            scanner.advance();
            if matches!(scanner.peek(), '+' | '-') {
                scanner.advance();
            }
            while !scanner.is_at_end() && scanner.peek().is_ascii_digit() {
                scanner.advance();
            }
        }

        NumberResult {
            raw: scanner.slice_from(start),
            format: NumberFormat::Decimal,
            num_type: if is_float {
                NumberType::Float
            } else {
                NumberType::Integer
            },
        }
    }
}
