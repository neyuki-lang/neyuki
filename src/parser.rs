use crate::lexer::{Lexer, Token};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Literal(String),
    Str(String),
    Interp(String),
    Variable(String),
    Vararg,
    Member {
        object: Box<Expr>,
        field: String,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    Function {
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
    Unary {
        op: String,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: String,
        right: Box<Expr>,
    },
    Table(Vec<TableEntry>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableEntry {
    pub key: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    Local {
        name: String,
        is_const: bool,
        type_name: Option<String>,
        initializer: Option<Expr>,
    },
    LocalMany {
        names: Vec<String>,
        is_const: bool,
        initializers: Vec<Expr>,
    },
    Assign {
        target: Expr,
        value: Expr,
        is_const: bool,
    },
    AssignMany {
        targets: Vec<Expr>,
        values: Vec<Expr>,
    },
    Increment {
        target: Expr,
        amount: i8,
    },
    Function {
        name: Option<String>,
        is_const: bool,
        params: Vec<Param>,
        return_type: Option<String>,
        body: Vec<Stmt>,
    },
    If {
        condition: Expr,
        then_branch: Vec<Stmt>,
        else_if_branches: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Option<Vec<Stmt>>,
    },
    For {
        vars: Vec<String>,
        source: Expr,
        body: Vec<Stmt>,
    },
    NumericFor {
        var: String,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    Repeat {
        body: Vec<Stmt>,
        condition: Expr,
    },
    Return(Vec<Expr>),
    Break,
    Continue,
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    pub variadic: bool,
}

#[derive(Clone, Debug)]
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(source: &str) -> Self {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize();
        Self { tokens, pos: 0 }
    }

    pub fn parse_program(&mut self) -> Vec<Stmt> {
        let mut program = Vec::new();
        while !self.is_eof() {
            if self.check_keyword("end")
                || self.check_keyword("else")
                || self.check_keyword("elseif")
                || self.check_keyword("until")
            {
                break;
            }

            if self.check_symbol(";") {
                self.pos += 1;
                continue;
            }

            program.push(self.parse_statement());
            self.consume_semicolon_if_any();
        }
        program
    }

    fn parse_statement(&mut self) -> Stmt {
        if self.check_keyword("local")
            || self.check_keyword("const")
            || self.check_keyword("global")
        {
            return self.parse_binding();
        }
        if self.check_keyword("function") {
            return self.parse_function(false);
        }
        if self.check_keyword("if") {
            return self.parse_if();
        }
        if self.check_keyword("for") {
            return self.parse_for();
        }
        if self.check_keyword("while") {
            return self.parse_while();
        }
        if self.check_keyword("repeat") {
            return self.parse_repeat();
        }
        if self.check_keyword("return") {
            self.pos += 1;
            let expr = if !self.is_eof()
                && !self.check_keyword("end")
                && !self.check_keyword("else")
                && !self.check_keyword("elseif")
                && !self.check_keyword("until")
            {
                let mut values = vec![self.parse_expr()];
                while self.match_symbol(",") {
                    values.push(self.parse_expr());
                }
                values
            } else {
                Vec::new()
            };
            return Stmt::Return(expr);
        }
        if self.check_keyword("break") {
            self.pos += 1;
            return Stmt::Break;
        }
        if self.check_keyword("continue") {
            self.pos += 1;
            return Stmt::Continue;
        }

        let expr = self.parse_expr();
        if self.match_symbol("++") {
            return Stmt::Increment {
                target: expr,
                amount: 1,
            };
        }
        if self.match_symbol("--") {
            return Stmt::Increment {
                target: expr,
                amount: -1,
            };
        }
        if self.match_symbol("=") {
            let value = self.parse_expr();
            return Stmt::Assign {
                target: expr,
                value,
                is_const: false,
            };
        }
        if self.check_symbol(",") {
            let mut targets = vec![expr];
            while self.match_symbol(",") {
                targets.push(self.parse_expr());
            }
            self.expect_symbol("=");
            let mut values = vec![self.parse_expr()];
            while self.match_symbol(",") {
                values.push(self.parse_expr());
            }
            return Stmt::AssignMany { targets, values };
        }
        if let Some(op) = self.match_compound_assignment() {
            let right = self.parse_expr();
            return Stmt::Assign {
                target: expr.clone(),
                value: Expr::Binary {
                    left: Box::new(expr),
                    op,
                    right: Box::new(right),
                },
                is_const: false,
            };
        }
        self.consume_bracket_attributes();
        Stmt::Expr(expr)
    }

    fn parse_binding(&mut self) -> Stmt {
        let mut is_const = false;
        while self.check_keyword("local")
            || self.check_keyword("const")
            || self.check_keyword("global")
        {
            is_const |= self.check_keyword("const");
            self.pos += 1;
        }

        if self.check_keyword("function") {
            return self.parse_function(is_const);
        }

        let mut name = self.expect_name();
        while self.match_symbol(".") {
            name.push('.');
            name.push_str(&self.expect_name());
        }
        if name.contains('.') {
            self.expect_symbol("=");
            let mut target = Expr::Variable(name.split('.').next().unwrap().to_string());
            for field in name.split('.').skip(1) {
                target = Expr::Member {
                    object: Box::new(target),
                    field: field.to_string(),
                };
            }
            return Stmt::Assign {
                target,
                value: self.parse_expr(),
                is_const,
            };
        }
        if self.check_symbol(",") {
            let mut names = vec![name];
            while self.match_symbol(",") {
                names.push(self.expect_name());
            }
            self.expect_symbol("=");
            let mut initializers = vec![self.parse_expr()];
            while self.match_symbol(",") {
                initializers.push(self.parse_expr());
            }
            return Stmt::LocalMany {
                names,
                is_const,
                initializers,
            };
        }
        let type_name = if self.match_symbol(":") {
            Some(self.read_type_annotation())
        } else {
            None
        };

        let initializer = if self.match_symbol("=") {
            Some(self.parse_expr())
        } else {
            None
        };

        Stmt::Local {
            name,
            is_const,
            type_name,
            initializer,
        }
    }

    fn parse_function(&mut self, is_const: bool) -> Stmt {
        self.expect_keyword("function");

        let name = if self.peek().kind == "name" {
            let mut name = self.expect_name();
            while self.match_symbol(".") {
                name.push('.');
                name.push_str(&self.expect_name());
            }
            Some(name)
        } else {
            None
        };

        let params = if self.match_symbol("(") {
            self.parse_param_list()
        } else {
            Vec::new()
        };

        let return_type = if self.match_symbol(":") {
            Some(self.read_type_annotation())
        } else {
            None
        };

        let body = self.parse_block_until("end");
        self.expect_keyword("end");
        Stmt::Function {
            name,
            is_const,
            params,
            return_type,
            body,
        }
    }

    fn parse_if(&mut self) -> Stmt {
        self.expect_keyword("if");
        let condition = self.parse_expr();
        self.expect_keyword("then");

        let then_branch = self.parse_block_until_any(&["else", "elseif", "end"]);
        let mut else_if_branches = Vec::new();
        let mut else_branch = None;

        while self.check_keyword("elseif") {
            self.expect_keyword("elseif");
            let next_condition = self.parse_expr();
            self.expect_keyword("then");
            let next_branch = self.parse_block_until_any(&["else", "elseif", "end"]);
            else_if_branches.push((next_condition, next_branch));
        }

        if self.check_keyword("else") {
            self.expect_keyword("else");
            else_branch = Some(self.parse_block_until("end"));
        }

        self.expect_keyword("end");

        Stmt::If {
            condition,
            then_branch,
            else_if_branches,
            else_branch,
        }
    }

    fn parse_for(&mut self) -> Stmt {
        self.expect_keyword("for");
        let first_name = self.expect_name();
        if self.match_symbol("=") {
            let start = self.parse_expr();
            self.expect_symbol(",");
            let end = self.parse_expr();
            let step = if self.match_symbol(",") {
                Some(self.parse_expr())
            } else {
                None
            };
            self.expect_keyword("do");
            let body = self.parse_block_until("end");
            self.expect_keyword("end");
            return Stmt::NumericFor {
                var: first_name,
                start,
                end,
                step,
                body,
            };
        }

        let mut vars = vec![first_name];
        if self.match_symbol(",") {
            vars.push(self.expect_name());
        }
        self.expect_keyword("in");
        let source = self.parse_expr();
        self.expect_keyword("do");
        let body = self.parse_block_until("end");
        self.expect_keyword("end");
        Stmt::For { vars, source, body }
    }

    fn parse_while(&mut self) -> Stmt {
        self.expect_keyword("while");
        let condition = self.parse_expr();
        self.expect_keyword("do");
        let body = self.parse_block_until("end");
        self.expect_keyword("end");
        Stmt::While { condition, body }
    }

    fn parse_repeat(&mut self) -> Stmt {
        self.expect_keyword("repeat");
        let body = self.parse_block_until("until");
        self.expect_keyword("until");
        let condition = self.parse_expr();
        Stmt::Repeat { body, condition }
    }

    fn parse_param_list(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        if self.check_symbol(")") {
            self.expect_symbol(")");
            return params;
        }

        loop {
            if self.match_symbol("...") {
                let type_name = if self.match_symbol(":") {
                    Some(self.read_type_annotation())
                } else {
                    None
                };
                params.push(Param {
                    name: "...".to_string(),
                    type_name,
                    variadic: true,
                });
                self.expect_symbol(")");
                break;
            }
            let name = self.expect_name();
            let type_name = if self.match_symbol(":") {
                Some(self.read_type_annotation())
            } else {
                None
            };
            params.push(Param {
                name,
                type_name,
                variadic: false,
            });

            if self.match_symbol(",") {
                continue;
            }
            self.expect_symbol(")");
            break;
        }
        params
    }

    fn parse_block_until(&mut self, end_kw: &str) -> Vec<Stmt> {
        self.parse_block_until_any(&[end_kw])
    }

    fn parse_block_until_any(&mut self, end_words: &[&str]) -> Vec<Stmt> {
        let mut block = Vec::new();
        while !self.is_eof() {
            if self.check_symbol(";") {
                self.pos += 1;
                continue;
            }
            if end_words.iter().any(|word| self.check_keyword(word)) {
                break;
            }
            block.push(self.parse_statement());
        }
        block
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_precedence(0)
    }

    fn parse_precedence(&mut self, min_prec: u8) -> Expr {
        let mut left = self.parse_prefix();

        loop {
            if self.is_eof() {
                break;
            }

            let Some((op, prec)) = self.peek_binary_op() else {
                break;
            };
            if prec < min_prec {
                break;
            }
            self.pos += 1;
            let next_min = prec + 1;
            let right = self.parse_precedence(next_min);
            left = Expr::Binary {
                left: Box::new(left),
                op: op.clone(),
                right: Box::new(right),
            };
        }

        left
    }

    fn parse_prefix(&mut self) -> Expr {
        if self.check_keyword("function") {
            self.expect_keyword("function");
            self.expect_symbol("(");
            let params = self.parse_param_list();
            if self.match_symbol(":") {
                self.read_type_annotation();
            }
            let body = self.parse_block_until("end");
            self.expect_keyword("end");
            return Expr::Function { params, body };
        }

        if self.check_symbol("(") {
            self.expect_symbol("(");
            let expr = self.parse_expr();
            self.expect_symbol(")");
            return self.parse_postfix(expr);
        }

        if self.check_symbol("{") {
            return self.parse_table();
        }

        if self.check_keyword("not") {
            self.pos += 1;
            return Expr::Unary {
                op: "not".to_string(),
                expr: Box::new(self.parse_prefix()),
            };
        }

        if self.check_symbol("-") {
            self.pos += 1;
            return Expr::Unary {
                op: "-".to_string(),
                expr: Box::new(self.parse_prefix()),
            };
        }
        if self.check_symbol("#") {
            self.pos += 1;
            return Expr::Unary {
                op: "#".to_string(),
                expr: Box::new(self.parse_prefix()),
            };
        }

        if self.match_symbol("...") {
            return Expr::Vararg;
        }

        let token = self.peek().clone();
        match token.kind {
            "name" | "keyword" => {
                self.pos += 1;
                let expr = match token.value.as_str() {
                    "true" | "false" | "nil" => Expr::Literal(token.value),
                    _ => Expr::Variable(token.value),
                };
                self.parse_postfix(expr)
            }
            "string" => {
                self.pos += 1;
                Expr::Str(token.value)
            }
            "interp" => {
                self.pos += 1;
                Expr::Interp(token.value)
            }
            "number" => {
                self.pos += 1;
                Expr::Literal(token.value)
            }
            _ => panic!("unexpected token in expression: {:?}", token),
        }
    }

    /// Applies any chain of `.field`, `(args)`, `:method(args)` and `[index]`
    /// suffixes to an already parsed expression.
    fn parse_postfix(&mut self, mut expr: Expr) -> Expr {
        loop {
            if self.match_symbol(".") {
                let field = self.expect_name();
                expr = Expr::Member {
                    object: Box::new(expr),
                    field,
                };
            } else if self.match_symbol("(") {
                let args = self.parse_call_args();
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                };
            } else if self.check_method_call() {
                self.pos += 1;
                let method = self.expect_name();
                self.expect_symbol("(");
                let args = self.parse_call_args();
                expr = Expr::MethodCall {
                    object: Box::new(expr),
                    method,
                    args,
                };
            } else if self.match_symbol("[") {
                if self.peek().kind == "name" && self.peek().value == "cite" {
                    while !self.is_eof() && !self.check_symbol("]") {
                        self.pos += 1;
                    }
                    self.expect_symbol("]");
                    continue;
                }
                let field = self.parse_expr();
                self.expect_symbol("]");
                expr = Expr::Index {
                    object: Box::new(expr),
                    index: Box::new(field),
                };
            } else {
                break;
            }
        }
        expr
    }

    fn parse_table(&mut self) -> Expr {
        self.expect_symbol("{");
        let mut entries = Vec::new();
        if !self.check_symbol("}") {
            loop {
                if self.peek().kind == "name"
                    && self
                        .tokens
                        .get(self.pos + 1)
                        .is_some_and(|next| next.kind == "symbol" && next.value == "=")
                {
                    let key = self.expect_name();
                    self.expect_symbol("=");
                    let value = self.parse_expr();
                    entries.push(TableEntry {
                        key: Some(key),
                        value,
                    });
                } else {
                    let value = self.parse_expr();
                    entries.push(TableEntry { key: None, value });
                }

                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol("}");
        Expr::Table(entries)
    }

    fn read_type_annotation(&mut self) -> String {
        let mut parts = Vec::new();
        let mut braces = 0usize;
        let mut brackets = 0usize;
        let mut parens = 0usize;
        let mut last_line = None;
        while !self.is_eof() {
            let token = self.peek().clone();
            if token.kind == "eof" {
                break;
            }
            // An annotation ends at a line break unless a bracket is still open,
            // so the statement on the next line is not swallowed into the type.
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
            parts.push(self.advance_token().value);
        }
        parts.join(" ")
    }

    fn advance_token(&mut self) -> Token {
        let token = self.peek().clone();
        self.pos += 1;
        token
    }

    fn consume_bracket_attributes(&mut self) {
        while self.match_symbol("[") {
            while !self.is_eof() && !self.check_symbol("]") {
                if self.check_symbol("[") {
                    self.pos += 1;
                    continue;
                }
                self.pos += 1;
            }
            self.match_symbol("]");
        }
    }

    fn consume_semicolon_if_any(&mut self) {
        while self.match_symbol(";") {}
    }

    /// `object:name(` starts a method call; a bare `:` elsewhere is a type
    /// annotation and is left alone.
    fn check_method_call(&self) -> bool {
        self.check_symbol(":")
            && self
                .tokens
                .get(self.pos + 1)
                .is_some_and(|t| t.kind == "name")
            && self
                .tokens
                .get(self.pos + 2)
                .is_some_and(|t| t.kind == "symbol" && t.value == "(")
    }

    /// Parses call arguments after the opening `(` up to and including `)`.
    fn parse_call_args(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        if !self.check_symbol(")") {
            loop {
                args.push(self.parse_expr());
                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(")");
        args
    }

    /// Consumes an `op=` token and returns the binary operator it applies, so
    /// `a op= b` can be desugared to `a = a op b`.
    fn match_compound_assignment(&mut self) -> Option<String> {
        const OPERATORS: [&str; 13] = [
            "+=", "-=", "*=", "/=", "//=", "%=", "^=", "..=", "<<=", ">>=", "&=", "|=", "??=",
        ];
        let token = self.peek();
        if token.kind != "symbol" || !OPERATORS.contains(&token.value.as_str()) {
            return None;
        }
        let op = token.value[..token.value.len() - 1].to_string();
        self.pos += 1;
        Some(op)
    }

    fn match_symbol(&mut self, value: &str) -> bool {
        if self.peek().kind == "symbol" && self.peek().value == value {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_symbol(&mut self, value: &str) {
        if !self.match_symbol(value) {
            panic!("expected symbol {:?}", value);
        }
    }

    fn expect_keyword(&mut self, value: &str) {
        if !self.match_keyword(value) {
            panic!("expected keyword {:?}", value);
        }
    }

    fn match_keyword(&mut self, value: &str) -> bool {
        if self.peek().kind == "keyword" && self.peek().value == value {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn check_keyword(&self, value: &str) -> bool {
        self.peek().kind == "keyword" && self.peek().value == value
    }

    fn expect_name(&mut self) -> String {
        let token = self.peek().clone();
        if token.kind == "name" {
            self.pos += 1;
            token.value
        } else {
            panic!("expected name, got {:?}", token)
        }
    }

    fn check_symbol(&self, value: &str) -> bool {
        self.peek().kind == "symbol" && self.peek().value == value
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn is_eof(&self) -> bool {
        self.peek().kind == "eof"
    }

    fn peek_binary_op(&self) -> Option<(String, u8)> {
        if self.peek().kind != "symbol" && self.peek().kind != "keyword" {
            return None;
        }

        let value = self.peek().value.clone();
        let prec = match value.as_str() {
            "or" => 1,
            "and" => 2,
            "==" | "!=" | "<" | "<=" | ">" | ">=" => 3,
            "|" => 4,
            "~" => 5,
            "&" => 6,
            "<<" | ">>" => 7,
            ".." => 8,
            "+" | "-" => 9,
            "*" | "/" | "//" | "%" => 10,
            "^" => 11,
            "??" => 12,
            _ => return None,
        };

        Some((value, prec))
    }
}

#[cfg(test)]
mod tests {
    use super::{Expr, Parser, Stmt};

    #[test]
    fn parses_local_assignment_and_call() {
        let mut parser = Parser::new("local msg = \"hello\"\nprint(msg)");
        let program = parser.parse_program();

        assert_eq!(
            program,
            vec![
                Stmt::Local {
                    name: "msg".to_string(),
                    is_const: false,
                    type_name: None,
                    initializer: Some(Expr::Str("hello".to_string())),
                },
                Stmt::Expr(Expr::Call {
                    callee: Box::new(Expr::Variable("print".to_string())),
                    args: vec![Expr::Variable("msg".to_string())],
                }),
            ]
        );
    }

    #[test]
    fn parses_typed_local_and_function_bindings() {
        let mut parser = Parser::new(
            "local small: int = 2\nconst function divide(a: int, b: int): int\n    return a // b\nend",
        );
        let program = parser.parse_program();

        assert_eq!(
            program[0],
            Stmt::Local {
                name: "small".to_string(),
                is_const: false,
                type_name: Some("int".to_string()),
                initializer: Some(Expr::Literal("2".to_string())),
            }
        );

        assert!(matches!(program[1], Stmt::Function { .. }));
    }

    #[test]
    fn parses_dotted_const_binding_as_assignment() {
        let mut parser = Parser::new("const math.e = 1");
        let program = parser.parse_program();

        assert_eq!(
            program,
            vec![Stmt::Assign {
                target: Expr::Member {
                    object: Box::new(Expr::Variable("math".to_string())),
                    field: "e".to_string(),
                },
                value: Expr::Literal("1".to_string()),
                is_const: true,
            }]
        );
    }

    #[test]
    fn parses_immediately_invoked_function_expression() {
        let mut parser = Parser::new("local value = (function() return 1 end)()");
        let program = parser.parse_program();

        assert!(matches!(
            program.first(),
            Some(Stmt::Local {
                initializer: Some(Expr::Call { .. }),
                ..
            })
        ));
    }

    #[test]
    fn parses_compound_and_multiple_assignment() {
        let mut parser = Parser::new("n += 2
a, b = b, a");
        let program = parser.parse_program();

        assert_eq!(
            program[0],
            Stmt::Assign {
                target: Expr::Variable("n".to_string()),
                value: Expr::Binary {
                    left: Box::new(Expr::Variable("n".to_string())),
                    op: "+".to_string(),
                    right: Box::new(Expr::Literal("2".to_string())),
                },
                is_const: false,
            }
        );
        assert_eq!(
            program[1],
            Stmt::AssignMany {
                targets: vec![
                    Expr::Variable("a".to_string()),
                    Expr::Variable("b".to_string())
                ],
                values: vec![
                    Expr::Variable("b".to_string()),
                    Expr::Variable("a".to_string())
                ],
            }
        );
    }

    #[test]
    fn parses_dotted_typed_function_bindings() {
        let mut parser = Parser::new(
            "function math.random(min: number, max: number): number\n    return 1\nend",
        );
        let program = parser.parse_program();

        assert_eq!(
            program[0],
            Stmt::Function {
                name: Some("math.random".to_string()),
                is_const: false,
                params: vec![
                    super::Param {
                        name: "min".to_string(),
                        type_name: Some("number".to_string()),
                        variadic: false
                    },
                    super::Param {
                        name: "max".to_string(),
                        type_name: Some("number".to_string()),
                        variadic: false
                    },
                ],
                return_type: Some("number".to_string()),
                body: vec![Stmt::Return(vec![Expr::Literal("1".to_string())])],
            }
        );
    }

    #[test]
    fn parses_typed_variadic_parameter_and_increment() {
        let mut parser = Parser::new(
            "function collect(first: number, ...: number): number\n    local total = first\n    total++\n    total--\n    return total\nend",
        );
        let program = parser.parse_program();

        let Stmt::Function { params, body, .. } = &program[0] else {
            panic!("expected function");
        };
        assert_eq!(
            params[1],
            super::Param {
                name: "...".to_string(),
                type_name: Some("number".to_string()),
                variadic: true
            }
        );
        assert!(matches!(body[1], Stmt::Increment { amount: 1, .. }));
        assert!(matches!(body[2], Stmt::Increment { amount: -1, .. }));
    }

    #[test]
    fn parses_numeric_for_loop() {
        let mut parser = Parser::new("for i = 1, 10, 2 do\n    print(i)\nend");
        let program = parser.parse_program();

        assert!(matches!(
            &program[0],
            Stmt::NumericFor {
                var,
                step: Some(_),
                body,
                ..
            } if var == "i" && body.len() == 1
        ));
    }

    #[test]
    fn parses_anonymous_function_expression() {
        let mut parser = Parser::new("table.sort(numbers, function(a, b) return a < b end)");
        let program = parser.parse_program();

        assert!(matches!(
            &program[0],
            Stmt::Expr(Expr::Call { args, .. })
                if matches!(args.get(1), Some(Expr::Function { params, body })
                    if params.len() == 2 && body.len() == 1)
        ));
    }
}
