// Syntactic action helpers and precedence engines for the Neyuki parser.

use crate::ast::*;
use crate::lexer::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrecedenceLevel {
    None = 0,
    Or = 1,
    And = 2,
    Comparison = 3,
    BitwiseOr = 4,
    BitwiseXor = 5,
    BitwiseAnd = 6,
    Shift = 7,
    Concat = 8,
    Term = 9,
    Factor = 10,
    Unary = 11,
    Power = 12,
    Call = 13,
}

pub struct ActionHelper;

impl ActionHelper {
    /// Maps an infix binary operator to its left and right binding powers (left, right).
    /// For left-associative operators: right = left + 1.
    /// For right-associative operators (e.g. `^` power, `..` concat): left > right.
    pub fn infix_binding_power(op: &str) -> Option<(u8, u8)> {
        let powers = match op {
            "or" => (1, 2),
            "and" => (3, 4),
            "==" | "!=" | "~=" | "<" | "<=" | ">" | ">=" => (5, 6),
            "|" => (7, 8),
            "~" => (9, 10),
            "&" => (11, 12),
            "<<" | ">>" | "<<<" | ">>>" => (13, 14),
            ".." => (16, 15), // Right-associative
            "+" | "-" => (17, 18),
            "*" | "/" | "//" | "%" => (19, 20),
            "^" => (22, 21), // Right-associative
            "??" => (23, 24),
            _ => return None,
        };
        Some(powers)
    }

    pub fn binary_precedence(op: &str) -> Option<u8> {
        let prec = match op {
            "or" => 1,
            "and" => 2,
            "==" | "!=" | "~=" | "<" | "<=" | ">" | ">=" => 3,
            "|" => 4,
            "~" => 5,
            "&" => 6,
            "<<" | ">>" | "<<<" | ">>>" => 7,
            ".." => 8,
            "+" | "-" => 9,
            "*" | "/" | "//" | "%" => 10,
            "^" => 11,
            "??" => 12,
            _ => return None,
        };
        Some(prec)
    }

    /// Maps a prefix unary operator to its binding power.
    pub fn prefix_binding_power(op: &str) -> Option<u8> {
        let power = match op {
            "not" | "-" | "~" | "#" => 25,
            _ => return None,
        };
        Some(power)
    }

    /// Converts a binary operator token string into an AST `BinOp`.
    pub fn parse_binop(op: &str) -> Option<BinOp> {
        match op {
            "+" => Some(BinOp::Add),
            "-" => Some(BinOp::Sub),
            "*" => Some(BinOp::Mul),
            "/" => Some(BinOp::Div),
            "//" => Some(BinOp::IDiv),
            "%" => Some(BinOp::Mod),
            "^" => Some(BinOp::Pow),
            "==" => Some(BinOp::Eq),
            "!=" | "~=" => Some(BinOp::Ne),
            "<" => Some(BinOp::Lt),
            "<=" => Some(BinOp::Le),
            ">" => Some(BinOp::Gt),
            ">=" => Some(BinOp::Ge),
            "and" => Some(BinOp::And),
            "or" => Some(BinOp::Or),
            ".." => Some(BinOp::Concat),
            "&" => Some(BinOp::BitAnd),
            "|" => Some(BinOp::BitOr),
            "~" => Some(BinOp::BitXor),
            "<<" => Some(BinOp::Shl),
            ">>" => Some(BinOp::Shr),
            _ => None,
        }
    }

    /// Converts a unary operator token string into an AST `UnOp`.
    pub fn parse_unary_op(op: &str) -> Option<UnOp> {
        match op {
            "-" => Some(UnOp::Neg),
            "not" => Some(UnOp::Not),
            "#" => Some(UnOp::Len),
            "~" => Some(UnOp::BitNot),
            _ => None,
        }
    }

    /// Maps a compound assignment operator (e.g. `+=`, `..=`) to its underlying `BinOp`.
    pub fn compound_assign_binop(op: &str) -> Option<BinOp> {
        match op {
            "+=" => Some(BinOp::Add),
            "-=" => Some(BinOp::Sub),
            "*=" => Some(BinOp::Mul),
            "/=" => Some(BinOp::Div),
            "//=" => Some(BinOp::IDiv),
            "%=" => Some(BinOp::Mod),
            "^=" => Some(BinOp::Pow),
            "..=" => Some(BinOp::Concat),
            "&=" => Some(BinOp::BitAnd),
            "|=" => Some(BinOp::BitOr),
            "<<=" | "<<<=" => Some(BinOp::Shl),
            ">>=" | ">>>=" => Some(BinOp::Shr),
            _ => None,
        }
    }

    /// Checks if a token represents a statement synchronization boundary for error recovery.
    pub fn is_statement_sync_token(token: &Token) -> bool {
        if token.kind == "keyword" {
            matches!(
                token.value.as_str(),
                "local"
                    | "const"
                    | "function"
                    | "if"
                    | "while"
                    | "repeat"
                    | "for"
                    | "return"
                    | "break"
                    | "continue"
                    | "defer"
                    | "end"
            )
        } else if token.kind == "symbol" {
            token.value == ";"
        } else {
            token.kind == "eof"
        }
    }

    /// Checks if a closing delimiter matches the given opening delimiter.
    pub fn is_matching_delimiter(open: char, close: char) -> bool {
        matches!((open, close), ('(', ')') | ('[', ']') | ('{', '}'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_helper_binding_powers_and_precedence() {
        let (add_l, add_r) = ActionHelper::infix_binding_power("+").unwrap();
        let (mul_l, mul_r) = ActionHelper::infix_binding_power("*").unwrap();
        assert!(mul_l > add_l);
        assert!(mul_r > add_r);

        assert_eq!(ActionHelper::binary_precedence("+"), Some(9));
        assert_eq!(ActionHelper::binary_precedence("*"), Some(10));
        assert_eq!(ActionHelper::binary_precedence("or"), Some(1));
    }

    #[test]
    fn test_action_helper_compound_and_unary_ops() {
        assert_eq!(ActionHelper::compound_assign_binop("+="), Some(BinOp::Add));
        assert_eq!(
            ActionHelper::compound_assign_binop("..="),
            Some(BinOp::Concat)
        );
        assert_eq!(
            ActionHelper::compound_assign_binop("//="),
            Some(BinOp::IDiv)
        );

        assert_eq!(ActionHelper::parse_unary_op("-"), Some(UnOp::Neg));
        assert_eq!(ActionHelper::parse_unary_op("not"), Some(UnOp::Not));
        assert_eq!(ActionHelper::parse_unary_op("#"), Some(UnOp::Len));
        assert_eq!(ActionHelper::parse_unary_op("~"), Some(UnOp::BitNot));
    }

    #[test]
    fn test_action_helper_delimiters_and_sync() {
        assert!(ActionHelper::is_matching_delimiter('(', ')'));
        assert!(ActionHelper::is_matching_delimiter('[', ']'));
        assert!(ActionHelper::is_matching_delimiter('{', '}'));
        assert!(!ActionHelper::is_matching_delimiter('(', ']'));

        let tok_local = Token {
            kind: "keyword",
            value: "local".to_string(),
            line: 1,
            col: 1,
        };
        assert!(ActionHelper::is_statement_sync_token(&tok_local));

        let tok_name = Token {
            kind: "name",
            value: "foo".to_string(),
            line: 1,
            col: 1,
        };
        assert!(!ActionHelper::is_statement_sync_token(&tok_name));
    }
}
