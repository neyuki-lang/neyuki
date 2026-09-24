// Type annotation and expression parsing engine for Neyuki parser.

use super::Parser;
use crate::ast::ty::TypeExpr;

pub struct TypeParserEngine;

impl TypeParserEngine {
    /// Reads raw string type annotation tokens until the next statement delimiter.
    pub fn read_type_annotation(parser: &mut Parser) -> String {
        let mut parts = Vec::new();
        let mut braces = 0usize;
        let mut brackets = 0usize;
        let mut parens = 0usize;
        let mut last_line = None;
        while !parser.is_eof() {
            let token = parser.peek().clone();
            if token.kind == "eof" {
                break;
            }
            if braces == 0
                && brackets == 0
                && parens == 0
                && last_line.is_some_and(|line| line != token.line)
            {
                break;
            }
            last_line = Some(token.line);
            if token.kind == "symbol" {
                if braces == 0
                    && brackets == 0
                    && parens == 0
                    && matches!(token.value.as_str(), "=" | "," | ")" | ";")
                {
                    break;
                }
                match token.value.as_str() {
                    "{" => braces += 1,
                    "}" if braces > 0 => braces -= 1,
                    "[" => brackets += 1,
                    "]" if brackets > 0 => brackets -= 1,
                    "(" => parens += 1,
                    ")" if parens > 0 => parens -= 1,
                    _ => {}
                }
            }
            if token.kind == "keyword"
                && braces == 0
                && brackets == 0
                && parens == 0
                && matches!(
                    token.value.as_str(),
                    "end"
                        | "do"
                        | "then"
                        | "else"
                        | "elseif"
                        | "until"
                        | "return"
                        | "local"
                        | "const"
                        | "global"
                        | "if"
                        | "for"
                        | "while"
                        | "repeat"
                )
            {
                break;
            }
            parts.push(parser.advance_token().value);
        }
        parts.join(" ")
    }

    /// Parses a structured `TypeExpr` from the current token stream.
    pub fn parse_type_expr(parser: &mut Parser) -> Result<TypeExpr, String> {
        let raw = Self::read_type_annotation(parser);
        if raw.trim().is_empty() {
            return Ok(TypeExpr::Any);
        }
        Ok(TypeExpr::parse(&raw))
    }
}

pub fn read_type_annotation(parser: &mut Parser) -> String {
    TypeParserEngine::read_type_annotation(parser)
}

pub fn parse_type_expr(parser: &mut Parser) -> Result<TypeExpr, String> {
    TypeParserEngine::parse_type_expr(parser)
}
