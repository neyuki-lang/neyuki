//! Type annotations.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeExpr {
    Named(String),
    Nil,
    Never,
    Optional(Box<TypeExpr>),
    Array(Box<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Record(Vec<(String, TypeExpr)>),
    Table {
        key: Box<TypeExpr>,
        val: Box<TypeExpr>,
    },
    Function {
        params: Vec<TypeExpr>,
        ret: Box<TypeExpr>,
    },
    Union(Vec<TypeExpr>),
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token<'a> {
    Ident(&'a str),
    Arrow,     // ->
    Pipe,      // |
    Colon,     // :
    Comma,     // ,
    Question,  // ?
    LParen,    // (
    RParen,    // )
    LBracket,  // [
    RBracket,  // ]
    LBrace,    // {
    RBrace,    // }
    DotDotDot, // ...
    Eof,
}

fn tokenize(input: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    let len = bytes.len();

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if b == b'-' && i + 1 < len && bytes[i + 1] == b'>' {
            tokens.push(Token::Arrow);
            i += 2;
            continue;
        }

        if b == b'.' && i + 2 < len && bytes[i + 1] == b'.' && bytes[i + 2] == b'.' {
            tokens.push(Token::DotDotDot);
            i += 3;
            continue;
        }

        match b {
            b'|' => {
                tokens.push(Token::Pipe);
                i += 1;
            }
            b':' => {
                tokens.push(Token::Colon);
                i += 1;
            }
            b',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            b'?' => {
                tokens.push(Token::Question);
                i += 1;
            }
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            b'[' => {
                tokens.push(Token::LBracket);
                i += 1;
            }
            b']' => {
                tokens.push(Token::RBracket);
                i += 1;
            }
            b'{' => {
                tokens.push(Token::LBrace);
                i += 1;
            }
            b'}' => {
                tokens.push(Token::RBrace);
                i += 1;
            }
            _ => {
                let start = i;
                while i < len {
                    let c = bytes[i];
                    if c.is_ascii_whitespace()
                        || matches!(
                            c,
                            b'|' | b':' | b',' | b'?' | b'(' | b')' | b'[' | b']' | b'{' | b'}'
                        )
                        || (c == b'-' && i + 1 < len && bytes[i + 1] == b'>')
                        || (c == b'.'
                            && i + 2 < len
                            && bytes[i + 1] == b'.'
                            && bytes[i + 2] == b'.')
                    {
                        break;
                    }
                    i += 1;
                }
                if start < i {
                    let text = &input[start..i];
                    tokens.push(Token::Ident(text));
                } else {
                    i += 1;
                }
            }
        }
    }
    tokens.push(Token::Eof);
    tokens
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: Vec<Token<'a>>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token<'a> {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn peek_ahead(&self, offset: usize) -> &Token<'a> {
        self.tokens.get(self.pos + offset).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token<'a> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            tok
        } else {
            Token::Eof
        }
    }

    fn eat(&mut self, expected: &Token<'a>) -> bool {
        if self.peek() == expected {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: &Token<'a>) -> bool {
        self.eat(expected)
    }

    fn parse_type(&mut self) -> TypeExpr {
        self.parse_union()
    }

    fn parse_union(&mut self) -> TypeExpr {
        let first = self.parse_function();
        if self.peek() != &Token::Pipe {
            return first;
        }

        let mut variants = vec![first];
        while self.eat(&Token::Pipe) {
            variants.push(self.parse_function());
        }

        let mut flat = Vec::new();
        for v in variants {
            if let TypeExpr::Union(nested) = v {
                flat.extend(nested);
            } else {
                flat.push(v);
            }
        }
        TypeExpr::Union(flat)
    }

    fn parse_function(&mut self) -> TypeExpr {
        let lhs = self.parse_postfix();
        if self.eat(&Token::Arrow) {
            let ret = self.parse_type();
            let params = match lhs {
                TypeExpr::Tuple(elems) => elems,
                TypeExpr::Nil => Vec::new(),
                other => vec![other],
            };
            TypeExpr::Function {
                params,
                ret: Box::new(ret),
            }
        } else {
            lhs
        }
    }

    fn parse_postfix(&mut self) -> TypeExpr {
        let mut ty = self.parse_primary();
        while self.eat(&Token::Question) {
            ty = TypeExpr::Optional(Box::new(ty));
        }
        ty
    }

    fn parse_primary(&mut self) -> TypeExpr {
        match self.peek() {
            Token::DotDotDot => {
                self.advance();
                if let Token::Ident(_) = self.peek() {
                    self.advance();
                }
                TypeExpr::Any
            }
            Token::Ident(name) => {
                let name = *name;
                self.advance();
                match name.to_lowercase().as_str() {
                    "any" => TypeExpr::Any,
                    "nil" => TypeExpr::Nil,
                    "never" | "void" => TypeExpr::Never,
                    _ => TypeExpr::Named(name.to_string()),
                }
            }
            Token::LParen => {
                self.advance();
                if self.eat(&Token::RParen) {
                    TypeExpr::Tuple(Vec::new())
                } else {
                    let mut elems = Vec::new();
                    loop {
                        if self.peek() == &Token::RParen || self.peek() == &Token::Eof {
                            break;
                        }
                        // Handle optional named parameter `name: Type` in function signatures
                        if matches!(self.peek(), Token::Ident(_))
                            && self.peek_ahead(1) == &Token::Colon
                        {
                            self.advance(); // consume param name
                            self.advance(); // consume colon
                        }
                        elems.push(self.parse_type());
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RParen);
                    if elems.len() == 1 {
                        elems.pop().unwrap()
                    } else {
                        TypeExpr::Tuple(elems)
                    }
                }
            }
            Token::LBracket => {
                self.advance();
                if self.eat(&Token::RBracket) {
                    TypeExpr::Array(Box::new(TypeExpr::Any))
                } else {
                    let mut elems = Vec::new();
                    loop {
                        if self.peek() == &Token::RBracket || self.peek() == &Token::Eof {
                            break;
                        }
                        elems.push(self.parse_type());
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RBracket);
                    if elems.len() == 1 {
                        TypeExpr::Array(Box::new(elems.pop().unwrap()))
                    } else {
                        TypeExpr::Tuple(elems)
                    }
                }
            }
            Token::LBrace => {
                self.advance();
                if self.eat(&Token::RBrace) {
                    TypeExpr::Table {
                        key: Box::new(TypeExpr::Any),
                        val: Box::new(TypeExpr::Any),
                    }
                } else if self.peek() == &Token::LBracket {
                    // Key-value map: {[k]: v} or { [k]: v }
                    self.advance(); // consume `[`
                    let key = self.parse_type();
                    self.expect(&Token::RBracket);
                    self.expect(&Token::Colon);
                    let val = self.parse_type();
                    self.eat(&Token::Comma);
                    self.expect(&Token::RBrace);
                    TypeExpr::Table {
                        key: Box::new(key),
                        val: Box::new(val),
                    }
                } else if matches!(self.peek(), Token::Ident(_))
                    && self.peek_ahead(1) == &Token::Colon
                {
                    // Record `{ field: Type, ... }`
                    let mut fields = Vec::new();
                    while self.peek() != &Token::RBrace && self.peek() != &Token::Eof {
                        let field_name = match self.advance() {
                            Token::Ident(s) => s.to_string(),
                            _ => continue,
                        };
                        self.expect(&Token::Colon);
                        let field_ty = self.parse_type();
                        fields.push((field_name, field_ty));
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RBrace);
                    TypeExpr::Record(fields)
                } else {
                    // Array in brace syntax: `{T}` (idiomatic in Lua/Luau)
                    let elem = self.parse_type();
                    self.expect(&Token::RBrace);
                    TypeExpr::Array(Box::new(elem))
                }
            }
            _ => {
                self.advance();
                TypeExpr::Any
            }
        }
    }
}

impl TypeExpr {
    pub fn parse(s: &str) -> Self {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Self::Any;
        }
        let tokens = tokenize(trimmed);
        let mut parser = Parser::new(tokens);
        parser.parse_type()
    }

    pub fn is_optional(&self) -> bool {
        matches!(self, Self::Optional(_))
    }

    pub fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => write!(f, "{}", name),
            Self::Nil => write!(f, "nil"),
            Self::Never => write!(f, "never"),
            Self::Optional(inner) => write!(f, "{}?", inner),
            Self::Array(inner) => write!(f, "[{}]", inner),
            Self::Tuple(elems) => {
                let s = elems
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "({})", s)
            }
            Self::Record(fields) => {
                let s = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{{{}}}", s)
            }
            Self::Table { key, val } => write!(f, "{{[{}]: {}}}", key, val),
            Self::Function { params, ret } => {
                let ps = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "({}) -> {}", ps, ret)
            }
            Self::Union(types) => {
                let s = types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(" | ");
                write!(f, "{}", s)
            }
            Self::Any => write!(f, "any"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nested_record_with_function() {
        let ty = TypeExpr::parse("{f: (int, int) -> int}");
        match ty {
            TypeExpr::Record(fields) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].0, "f");
                match &fields[0].1 {
                    TypeExpr::Function { params, ret } => {
                        assert_eq!(params.len(), 2);
                        assert_eq!(params[0], TypeExpr::Named("int".to_string()));
                        assert_eq!(params[1], TypeExpr::Named("int".to_string()));
                        assert_eq!(**ret, TypeExpr::Named("int".to_string()));
                    }
                    other => panic!("expected function type, got {:?}", other),
                }
            }
            other => panic!("expected record type, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_complex_nested_record() {
        let ty = TypeExpr::parse(
            "{ name: string, age: int, scores: [int], transform: (int, string) -> bool }",
        );
        match ty {
            TypeExpr::Record(fields) => {
                assert_eq!(fields.len(), 4);
                assert_eq!(fields[0].0, "name");
                assert_eq!(fields[0].1, TypeExpr::Named("string".to_string()));
                assert_eq!(fields[1].0, "age");
                assert_eq!(fields[1].1, TypeExpr::Named("int".to_string()));
                assert_eq!(fields[2].0, "scores");
                assert_eq!(
                    fields[2].1,
                    TypeExpr::Array(Box::new(TypeExpr::Named("int".to_string())))
                );
                assert_eq!(fields[3].0, "transform");
                match &fields[3].1 {
                    TypeExpr::Function { params, ret } => {
                        assert_eq!(params.len(), 2);
                        assert_eq!(params[0], TypeExpr::Named("int".to_string()));
                        assert_eq!(params[1], TypeExpr::Named("string".to_string()));
                        assert_eq!(**ret, TypeExpr::Named("bool".to_string()));
                    }
                    other => panic!("expected function, got {:?}", other),
                }
            }
            other => panic!("expected record, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_table_map() {
        let ty = TypeExpr::parse("{ [string]: [int | string] }");
        match ty {
            TypeExpr::Table { key, val } => {
                assert_eq!(*key, TypeExpr::Named("string".to_string()));
                match *val {
                    TypeExpr::Array(inner) => match *inner {
                        TypeExpr::Union(variants) => {
                            assert_eq!(variants.len(), 2);
                            assert_eq!(variants[0], TypeExpr::Named("int".to_string()));
                            assert_eq!(variants[1], TypeExpr::Named("string".to_string()));
                        }
                        other => panic!("expected union, got {:?}", other),
                    },
                    other => panic!("expected array, got {:?}", other),
                }
            }
            other => panic!("expected table map, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_function_arrow_variants() {
        // (int, int) -> int
        let f1 = TypeExpr::parse("(int, int) -> int");
        assert_eq!(
            f1,
            TypeExpr::Function {
                params: vec![
                    TypeExpr::Named("int".to_string()),
                    TypeExpr::Named("int".to_string())
                ],
                ret: Box::new(TypeExpr::Named("int".to_string())),
            }
        );

        // int -> string
        let f2 = TypeExpr::parse("int -> string");
        assert_eq!(
            f2,
            TypeExpr::Function {
                params: vec![TypeExpr::Named("int".to_string())],
                ret: Box::new(TypeExpr::Named("string".to_string())),
            }
        );

        // () -> void
        let f3 = TypeExpr::parse("() -> void");
        assert_eq!(
            f3,
            TypeExpr::Function {
                params: vec![],
                ret: Box::new(TypeExpr::Never),
            }
        );

        // ((int) -> string) -> bool
        let f4 = TypeExpr::parse("((int) -> string) -> bool");
        assert_eq!(
            f4,
            TypeExpr::Function {
                params: vec![TypeExpr::Function {
                    params: vec![TypeExpr::Named("int".to_string())],
                    ret: Box::new(TypeExpr::Named("string".to_string())),
                }],
                ret: Box::new(TypeExpr::Named("bool".to_string())),
            }
        );

        // Named parameters in function signature: (x: int, y: string) -> bool
        let f5 = TypeExpr::parse("(x: int, y: string) -> bool");
        assert_eq!(
            f5,
            TypeExpr::Function {
                params: vec![
                    TypeExpr::Named("int".to_string()),
                    TypeExpr::Named("string".to_string())
                ],
                ret: Box::new(TypeExpr::Named("bool".to_string())),
            }
        );
    }

    #[test]
    fn test_parse_unions_and_optional() {
        let u1 = TypeExpr::parse("int | string | nil");
        assert_eq!(
            u1,
            TypeExpr::Union(vec![
                TypeExpr::Named("int".to_string()),
                TypeExpr::Named("string".to_string()),
                TypeExpr::Nil,
            ])
        );

        let opt1 = TypeExpr::parse("int?");
        assert_eq!(
            opt1,
            TypeExpr::Optional(Box::new(TypeExpr::Named("int".to_string())))
        );

        let opt_union = TypeExpr::parse("(int | string)?");
        assert_eq!(
            opt_union,
            TypeExpr::Optional(Box::new(TypeExpr::Union(vec![
                TypeExpr::Named("int".to_string()),
                TypeExpr::Named("string".to_string()),
            ])))
        );
    }

    #[test]
    fn test_parse_tuples_and_arrays() {
        let arr = TypeExpr::parse("[int]");
        assert_eq!(
            arr,
            TypeExpr::Array(Box::new(TypeExpr::Named("int".to_string())))
        );

        let arr2 = TypeExpr::parse("{string}");
        assert_eq!(
            arr2,
            TypeExpr::Array(Box::new(TypeExpr::Named("string".to_string())))
        );

        let tup = TypeExpr::parse("(int, string, bool)");
        assert_eq!(
            tup,
            TypeExpr::Tuple(vec![
                TypeExpr::Named("int".to_string()),
                TypeExpr::Named("string".to_string()),
                TypeExpr::Named("bool".to_string()),
            ])
        );
    }

    #[test]
    fn test_parse_primitives_and_wildcards() {
        assert_eq!(TypeExpr::parse("any"), TypeExpr::Any);
        assert_eq!(TypeExpr::parse("..."), TypeExpr::Any);
        assert_eq!(TypeExpr::parse("...T"), TypeExpr::Any);
        assert_eq!(TypeExpr::parse("nil"), TypeExpr::Nil);
        assert_eq!(TypeExpr::parse("never"), TypeExpr::Never);
        assert_eq!(TypeExpr::parse("void"), TypeExpr::Never);
        assert_eq!(TypeExpr::parse(""), TypeExpr::Any);
        assert_eq!(TypeExpr::parse("   "), TypeExpr::Any);
    }
}
