use crate::lexer::{Lexer, Token};

pub use crate::ast::*;

#[derive(Clone, Debug)]
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    span_pool: SpanPool,
}

impl Parser {
    pub fn new(source: &str) -> Self {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize();
        Self {
            tokens,
            pos: 0,
            depth: 0,
            span_pool: SpanPool::new(),
        }
    }

    fn current_loc(&self) -> SourceLocation {
        let tok = &self.tokens[self.pos.min(self.tokens.len().saturating_sub(1))];
        SourceLocation::new(tok.line as u32, tok.col as u32)
    }

    fn record_span(&mut self, id: NodeId, start: SourceLocation) {
        let end = self.current_loc();
        self.span_pool.insert(id, Span::new(start, end));
    }

    pub fn parse_program(&mut self) -> Result<(Vec<Stmt>, SpanPool), String> {
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

            program.push(self.parse_statement()?);
            self.consume_semicolon_if_any();
        }
        Ok((program, std::mem::take(&mut self.span_pool)))
    }

    fn parse_statement(&mut self) -> Result<Stmt, String> {
        let start = self.current_loc();
        let stmt = self.parse_statement_inner()?;
        self.record_span(stmt.node_id(), start);
        Ok(stmt)
    }

    fn parse_statement_inner(&mut self) -> Result<Stmt, String> {
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
                let mut values = vec![self.parse_expr()?];
                while self.match_symbol(",") {
                    values.push(self.parse_expr()?);
                }
                values
            } else {
                Vec::new()
            };
            return Ok(Stmt::Return {
                id: NodeId::next(),
                values: expr,
            });
        }
        if self.check_keyword("break") {
            self.pos += 1;
            return Ok(Stmt::Break { id: NodeId::next() });
        }
        if self.check_keyword("continue") {
            self.pos += 1;
            return Ok(Stmt::Continue { id: NodeId::next() });
        }

        let expr = self.parse_expr()?;
        if self.match_symbol("++") {
            let target = AssignTarget::from_expr(expr)?;
            return Ok(Stmt::Increment {
                id: NodeId::next(),
                target,
                amount: 1,
            });
        }
        if self.match_symbol("--") {
            let target = AssignTarget::from_expr(expr)?;
            return Ok(Stmt::Increment {
                id: NodeId::next(),
                target,
                amount: -1,
            });
        }
        if self.match_symbol("=") {
            let target = AssignTarget::from_expr(expr)?;
            let value = self.parse_expr()?;
            return Ok(Stmt::Assign {
                id: NodeId::next(),
                target,
                value,
                is_const: false,
            });
        }
        if self.check_symbol(",") {
            let mut targets = vec![AssignTarget::from_expr(expr)?];
            while self.match_symbol(",") {
                let next_expr = self.parse_expr()?;
                targets.push(AssignTarget::from_expr(next_expr)?);
            }
            self.expect_symbol("=")?;
            let mut values = vec![self.parse_expr()?];
            while self.match_symbol(",") {
                values.push(self.parse_expr()?);
            }
            return Ok(Stmt::AssignMany {
                id: NodeId::next(),
                targets,
                values,
            });
        }
        if let Some(op) = self.match_compound_assignment() {
            let target = AssignTarget::from_expr_ref(&expr)?;
            let right = self.parse_expr()?;
            let bin_op = BinOp::parse_str(&op).ok_or_else(|| {
                format!(
                    "[line {}] unknown compound operator '{}'",
                    self.peek().line,
                    op
                )
            })?;
            let bin_start = self
                .span_pool
                .get(expr.node_id())
                .map(|s| s.start)
                .unwrap_or_else(|| self.current_loc());
            let bin_expr = Expr::Binary {
                id: NodeId::next(),
                left: Box::new(expr),
                op: bin_op,
                right: Box::new(right),
            };
            self.record_span(bin_expr.node_id(), bin_start);
            return Ok(Stmt::Assign {
                id: NodeId::next(),
                target,
                value: bin_expr,
                is_const: false,
            });
        }
        self.consume_bracket_attributes();
        Ok(Stmt::Expr {
            id: NodeId::next(),
            expr,
        })
    }

    fn parse_binding(&mut self) -> Result<Stmt, String> {
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

        let mut name = self.expect_name()?;
        while self.match_symbol(".") {
            name.push('.');
            name.push_str(&self.expect_name()?);
        }
        if name.contains('.') {
            self.expect_symbol("=")?;
            let mut target = Expr::Variable {
                id: NodeId::next(),
                name: name.split('.').next().unwrap().to_string(),
            };
            for field in name.split('.').skip(1) {
                target = Expr::Member {
                    id: NodeId::next(),
                    object: Box::new(target),
                    field: field.to_string(),
                };
            }
            let assign_target = AssignTarget::from_expr(target)?;
            return Ok(Stmt::Assign {
                id: NodeId::next(),
                target: assign_target,
                value: self.parse_expr()?,
                is_const,
            });
        }
        if self.check_symbol(",") {
            let mut names = vec![name];
            while self.match_symbol(",") {
                names.push(self.expect_name()?);
            }
            self.expect_symbol("=")?;
            let mut initializers = vec![self.parse_expr()?];
            while self.match_symbol(",") {
                initializers.push(self.parse_expr()?);
            }
            return Ok(Stmt::LocalMany {
                id: NodeId::next(),
                names,
                is_const,
                initializers,
            });
        }
        let type_name = if self.match_symbol(":") {
            Some(self.read_type_annotation())
        } else {
            None
        };

        let initializer = if self.match_symbol("=") {
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(Stmt::Local {
            id: NodeId::next(),
            name,
            is_const,
            type_name,
            initializer,
        })
    }

    fn parse_function(&mut self, is_const: bool) -> Result<Stmt, String> {
        self.expect_keyword("function")?;

        let name = if self.peek().kind == "name" {
            let mut name = self.expect_name()?;
            while self.match_symbol(".") {
                name.push('.');
                name.push_str(&self.expect_name()?);
            }
            Some(name)
        } else {
            None
        };

        let params = if self.match_symbol("(") {
            self.parse_param_list()?
        } else {
            Vec::new()
        };

        let return_type = if self.match_symbol(":") {
            Some(self.read_type_annotation())
        } else {
            None
        };

        let body = self.parse_block_until("end")?;
        self.expect_keyword("end")?;
        Ok(Stmt::Function {
            id: NodeId::next(),
            name,
            is_const,
            params,
            return_type,
            body,
        })
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        self.expect_keyword("if")?;
        let condition = self.parse_expr()?;
        self.expect_keyword("then")?;

        let then_branch = self.parse_block_until_any(&["else", "elseif", "end"])?;
        let mut else_if_branches = Vec::new();
        let mut else_branch = None;

        while self.check_keyword("elseif") {
            self.expect_keyword("elseif")?;
            let next_condition = self.parse_expr()?;
            self.expect_keyword("then")?;
            let next_branch = self.parse_block_until_any(&["else", "elseif", "end"])?;
            else_if_branches.push((next_condition, next_branch));
        }

        if self.check_keyword("else") {
            self.expect_keyword("else")?;
            else_branch = Some(self.parse_block_until("end")?);
        }

        self.expect_keyword("end")?;

        Ok(Stmt::If {
            id: NodeId::next(),
            condition,
            then_branch,
            else_if_branches,
            else_branch,
        })
    }

    fn parse_for(&mut self) -> Result<Stmt, String> {
        self.expect_keyword("for")?;
        let first_name = self.expect_name()?;
        if self.match_symbol("=") {
            let start = self.parse_expr()?;
            self.expect_symbol(",")?;
            let end = self.parse_expr()?;
            let step = if self.match_symbol(",") {
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect_keyword("do")?;
            let body = self.parse_block_until("end")?;
            self.expect_keyword("end")?;
            return Ok(Stmt::NumericFor {
                id: NodeId::next(),
                var: first_name,
                start,
                end,
                step,
                body,
            });
        }

        let mut vars = vec![first_name];
        if self.match_symbol(",") {
            vars.push(self.expect_name()?);
        }
        self.expect_keyword("in")?;
        let source = self.parse_expr()?;
        self.expect_keyword("do")?;
        let body = self.parse_block_until("end")?;
        self.expect_keyword("end")?;
        Ok(Stmt::For {
            id: NodeId::next(),
            vars,
            source,
            body,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, String> {
        self.expect_keyword("while")?;
        let condition = self.parse_expr()?;
        self.expect_keyword("do")?;
        let body = self.parse_block_until("end")?;
        self.expect_keyword("end")?;
        Ok(Stmt::While {
            id: NodeId::next(),
            condition,
            body,
        })
    }

    fn parse_repeat(&mut self) -> Result<Stmt, String> {
        self.expect_keyword("repeat")?;
        let body = self.parse_block_until("until")?;
        self.expect_keyword("until")?;
        let condition = self.parse_expr()?;
        Ok(Stmt::Repeat {
            id: NodeId::next(),
            body,
            condition,
        })
    }

    fn parse_param_list(&mut self) -> Result<Vec<Param>, String> {
        let mut params = Vec::new();
        if self.check_symbol(")") {
            self.expect_symbol(")")?;
            return Ok(params);
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
                self.expect_symbol(")")?;
                break;
            }
            let name = self.expect_name()?;
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
            self.expect_symbol(")")?;
            break;
        }
        Ok(params)
    }

    fn parse_block_until(&mut self, end_kw: &str) -> Result<Vec<Stmt>, String> {
        self.parse_block_until_any(&[end_kw])
    }

    fn parse_block_until_any(&mut self, end_words: &[&str]) -> Result<Vec<Stmt>, String> {
        let mut block = Vec::new();
        while !self.is_eof() {
            if self.check_symbol(";") {
                self.pos += 1;
                continue;
            }
            if end_words.iter().any(|word| self.check_keyword(word)) {
                break;
            }
            block.push(self.parse_statement()?);
        }
        Ok(block)
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_precedence(0)
    }

    fn parse_precedence(&mut self, min_prec: u8) -> Result<Expr, String> {
        let mut left = self.parse_prefix()?;

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
            let right = self.parse_precedence(next_min)?;
            let bin_op = BinOp::parse_str(&op).ok_or_else(|| {
                format!(
                    "[line {}] unknown binary operator '{}'",
                    self.peek().line,
                    op
                )
            })?;
            let start = self
                .span_pool
                .get(left.node_id())
                .map(|s| s.start)
                .unwrap_or_else(|| self.current_loc());
            let bin_expr = Expr::Binary {
                id: NodeId::next(),
                left: Box::new(left),
                op: bin_op,
                right: Box::new(right),
            };
            self.record_span(bin_expr.node_id(), start);
            left = bin_expr;
        }

        Ok(left)
    }

    fn parse_prefix(&mut self) -> Result<Expr, String> {
        const MAX_PARSE_DEPTH: usize = 120;
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(format!(
                "[line {}] parse recursion depth limit (120) exceeded",
                self.peek().line
            ));
        }
        self.depth += 1;
        let res = self.parse_prefix_inner();
        self.depth -= 1;
        res
    }

    fn parse_prefix_inner(&mut self) -> Result<Expr, String> {
        let start = self.current_loc();
        if self.check_keyword("function") {
            self.expect_keyword("function")?;
            self.expect_symbol("(")?;
            let params = self.parse_param_list()?;
            if self.match_symbol(":") {
                self.read_type_annotation();
            }
            let body = self.parse_block_until("end")?;
            self.expect_keyword("end")?;
            let expr = Expr::Function {
                id: NodeId::next(),
                params,
                body,
            };
            self.record_span(expr.node_id(), start);
            return Ok(expr);
        }

        if self.check_symbol("(") {
            self.expect_symbol("(")?;
            let expr = self.parse_expr()?;
            self.expect_symbol(")")?;
            return self.parse_postfix(expr);
        }

        if self.check_symbol("{") {
            return self.parse_table();
        }

        if self.check_keyword("not") {
            self.pos += 1;
            let inner = self.parse_prefix()?;
            let expr = Expr::Unary {
                id: NodeId::next(),
                op: UnOp::Not,
                expr: Box::new(inner),
            };
            self.record_span(expr.node_id(), start);
            return Ok(expr);
        }

        if self.check_symbol("-") {
            self.pos += 1;
            let inner = self.parse_prefix()?;
            let expr = Expr::Unary {
                id: NodeId::next(),
                op: UnOp::Neg,
                expr: Box::new(inner),
            };
            self.record_span(expr.node_id(), start);
            return Ok(expr);
        }
        if self.check_symbol("#") {
            self.pos += 1;
            let inner = self.parse_prefix()?;
            let expr = Expr::Unary {
                id: NodeId::next(),
                op: UnOp::Len,
                expr: Box::new(inner),
            };
            self.record_span(expr.node_id(), start);
            return Ok(expr);
        }
        if self.check_symbol("~") {
            self.pos += 1;
            let inner = self.parse_prefix()?;
            let expr = Expr::Unary {
                id: NodeId::next(),
                op: UnOp::BitNot,
                expr: Box::new(inner),
            };
            self.record_span(expr.node_id(), start);
            return Ok(expr);
        }

        if self.match_symbol("...") {
            let expr = Expr::Vararg { id: NodeId::next() };
            self.record_span(expr.node_id(), start);
            return Ok(expr);
        }

        let token = self.peek().clone();
        match token.kind {
            "name" | "keyword" => {
                self.pos += 1;
                let expr = match token.value.as_str() {
                    "true" => Expr::Literal {
                        id: NodeId::next(),
                        value: Literal::Bool(true),
                    },
                    "false" => Expr::Literal {
                        id: NodeId::next(),
                        value: Literal::Bool(false),
                    },
                    "nil" => Expr::Literal {
                        id: NodeId::next(),
                        value: Literal::Nil,
                    },
                    _ => Expr::Variable {
                        id: NodeId::next(),
                        name: token.value,
                    },
                };
                self.record_span(expr.node_id(), start);
                self.parse_postfix(expr)
            }
            "string" => {
                self.pos += 1;
                let expr = Expr::Literal {
                    id: NodeId::next(),
                    value: Literal::String(token.value),
                };
                self.record_span(expr.node_id(), start);
                Ok(expr)
            }
            "interp" => {
                self.pos += 1;
                let parts = parse_interp_parts(&token.value)?;
                let expr = Expr::Interp {
                    id: NodeId::next(),
                    parts,
                };
                self.record_span(expr.node_id(), start);
                Ok(expr)
            }
            "number" => {
                self.pos += 1;
                let lit = Literal::parse_number(&token.value).ok_or_else(|| {
                    format!(
                        "[line {}] invalid number literal '{}'",
                        token.line, token.value
                    )
                })?;
                let expr = Expr::Literal {
                    id: NodeId::next(),
                    value: lit,
                };
                self.record_span(expr.node_id(), start);
                Ok(expr)
            }
            _ => Err(format!(
                "[line {}] unexpected token in expression: {:?}",
                token.line, token.value
            )),
        }
    }

    /// Applies any chain of `.field`, `(args)`, `:method(args)` and `[index]`
    /// suffixes to an already parsed expression.
    fn parse_postfix(&mut self, mut expr: Expr) -> Result<Expr, String> {
        loop {
            let start = self
                .span_pool
                .get(expr.node_id())
                .map(|s| s.start)
                .unwrap_or_else(|| self.current_loc());
            if self.match_symbol(".") {
                let field = self.expect_name()?;
                let node = Expr::Member {
                    id: NodeId::next(),
                    object: Box::new(expr),
                    field,
                };
                self.record_span(node.node_id(), start);
                expr = node;
            } else if self.match_symbol("(") {
                let args = self.parse_call_args()?;
                let node = Expr::Call {
                    id: NodeId::next(),
                    callee: Box::new(expr),
                    args,
                };
                self.record_span(node.node_id(), start);
                expr = node;
            } else if self.check_method_call() {
                self.pos += 1;
                let method = self.expect_name()?;
                self.expect_symbol("(")?;
                let args = self.parse_call_args()?;
                let node = Expr::MethodCall {
                    id: NodeId::next(),
                    object: Box::new(expr),
                    method,
                    args,
                };
                self.record_span(node.node_id(), start);
                expr = node;
            } else if self.match_symbol("[") {
                if self.peek().kind == "name" && self.peek().value == "cite" {
                    while !self.is_eof() && !self.check_symbol("]") {
                        self.pos += 1;
                    }
                    self.expect_symbol("]")?;
                    continue;
                }
                let field = self.parse_expr()?;
                self.expect_symbol("]")?;
                let node = Expr::Index {
                    id: NodeId::next(),
                    object: Box::new(expr),
                    index: Box::new(field),
                };
                self.record_span(node.node_id(), start);
                expr = node;
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_table(&mut self) -> Result<Expr, String> {
        let start = self.current_loc();
        self.expect_symbol("{")?;
        let mut entries = Vec::new();
        if !self.check_symbol("}") {
            loop {
                if self.peek().kind == "name"
                    && self
                        .tokens
                        .get(self.pos + 1)
                        .is_some_and(|next| next.kind == "symbol" && next.value == "=")
                {
                    let key = self.expect_name()?;
                    self.expect_symbol("=")?;
                    let value = self.parse_expr()?;
                    entries.push(TableEntry {
                        key: Some(key),
                        value,
                    });
                } else if self.check_symbol("[")
                    && self
                        .tokens
                        .get(self.pos + 1)
                        .is_some_and(|next| next.kind == "string")
                    && self
                        .tokens
                        .get(self.pos + 2)
                        .is_some_and(|next| next.kind == "symbol" && next.value == "]")
                {
                    self.expect_symbol("[")?;
                    let key = self.advance_token().value;
                    self.expect_symbol("]")?;
                    self.expect_symbol("=")?;
                    let value = self.parse_expr()?;
                    entries.push(TableEntry {
                        key: Some(key),
                        value,
                    });
                } else {
                    let value = self.parse_expr()?;
                    entries.push(TableEntry { key: None, value });
                }

                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        let expr = Expr::Table {
            id: NodeId::next(),
            entries,
        };
        self.record_span(expr.node_id(), start);
        Ok(expr)
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
            let _ = self.match_symbol("]");
        }
    }

    fn consume_semicolon_if_any(&mut self) {
        while self.match_symbol(";") {}
    }

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

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if !self.check_symbol(")") {
            loop {
                args.push(self.parse_expr()?);
                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(")")?;
        Ok(args)
    }

    fn match_compound_assignment(&mut self) -> Option<String> {
        const OPERATORS: [&str; 15] = [
            "+=", "-=", "*=", "/=", "//=", "%=", "^=", "..=", "<<=", ">>=", "<<<=", ">>>=", "&=",
            "|=", "??=",
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

    fn expect_symbol(&mut self, value: &str) -> Result<(), String> {
        if !self.match_symbol(value) {
            Err(format!(
                "[line {}] expected symbol {:?}",
                self.peek().line,
                value
            ))
        } else {
            Ok(())
        }
    }

    fn expect_keyword(&mut self, value: &str) -> Result<(), String> {
        if !self.match_keyword(value) {
            Err(format!(
                "[line {}] expected keyword {:?}",
                self.peek().line,
                value
            ))
        } else {
            Ok(())
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

    fn expect_name(&mut self) -> Result<String, String> {
        let token = self.peek().clone();
        if token.kind == "name" {
            self.pos += 1;
            Ok(token.value)
        } else {
            Err(format!(
                "[line {}] expected name, got {:?}",
                token.line, token.value
            ))
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

        Some((value, prec))
    }
}

/// Splits a lexed interpolation token (e.g. `"x = {a + b}!"`) into literal
/// and expression parts once, at parse time, returning a Result instead of panicking.
fn parse_interp_parts(value: &str) -> Result<Vec<InterpPart>, String> {
    let mut parts = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('{') {
        if start > 0 {
            parts.push(InterpPart::Literal(rest[..start].to_string()));
        }
        let after_start = &rest[start + 1..];
        let end = match after_start.find('}') {
            Some(e) => e,
            None => {
                return Err(format!(
                    "syntax error: unfinished string interpolation in {:?}",
                    value
                ));
            }
        };
        let expression = &after_start[..end];
        let mut parser = Parser::new(expression);
        let (statements, _pool) = parser.parse_program()?;
        if statements.len() != 1 {
            return Err(format!(
                "syntax error: string interpolation must contain exactly one expression: {:?}",
                expression
            ));
        }
        match statements.into_iter().next().unwrap() {
            Stmt::Expr { expr, .. } => {
                parts.push(InterpPart::Expr(expr));
            }
            _ => {
                return Err(format!(
                    "syntax error: string interpolation must contain an expression: {:?}",
                    expression
                ));
            }
        }
        rest = &after_start[end + 1..];
    }
    if !rest.is_empty() {
        parts.push(InterpPart::Literal(rest.to_string()));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::{AssignTarget, BinOp, Expr, NodeId, Parser, Stmt};

    #[test]
    fn parses_local_assignment_and_call() {
        let mut parser = Parser::new("local msg = \"hello\"\nprint(msg)");
        let (program, _pool) = parser.parse_program().expect("failed to parse");

        assert_eq!(
            program,
            vec![
                Stmt::Local {
                    id: NodeId::next(),
                    name: "msg".to_string(),
                    is_const: false,
                    type_name: None,
                    initializer: Some(Expr::string("hello")),
                },
                Stmt::Expr {
                    id: NodeId::next(),
                    expr: Expr::Call {
                        id: NodeId::next(),
                        callee: Box::new(Expr::Variable {
                            id: NodeId::next(),
                            name: "print".to_string(),
                        }),
                        args: vec![Expr::Variable {
                            id: NodeId::next(),
                            name: "msg".to_string(),
                        }],
                    },
                },
            ]
        );
    }

    #[test]
    fn parses_typed_local_and_function_bindings() {
        let mut parser = Parser::new(
            "local small: int = 2\nconst function divide(a: int, b: int): int\n    return a // b\nend",
        );
        let (program, _pool) = parser.parse_program().expect("failed to parse");

        assert_eq!(
            program[0],
            Stmt::Local {
                id: NodeId::next(),
                name: "small".to_string(),
                is_const: false,
                type_name: Some("int".to_string()),
                initializer: Some(Expr::int(2)),
            }
        );

        assert!(matches!(program[1], Stmt::Function { .. }));
    }

    #[test]
    fn parses_dotted_const_binding_as_assignment() {
        let mut parser = Parser::new("const math.e = 1");
        let (program, _pool) = parser.parse_program().expect("failed to parse");

        assert_eq!(
            program,
            vec![Stmt::Assign {
                id: NodeId::next(),
                target: AssignTarget::Member {
                    object: Box::new(Expr::Variable {
                        id: NodeId::next(),
                        name: "math".to_string(),
                    }),
                    field: "e".to_string(),
                },
                value: Expr::int(1),
                is_const: true,
            }]
        );
    }

    #[test]
    fn parses_immediately_invoked_function_expression() {
        let mut parser = Parser::new("local value = (function() return 1 end)()");
        let (program, _pool) = parser.parse_program().expect("failed to parse");

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
        let mut parser = Parser::new("n += 2\na, b = b, a");
        let (program, _pool) = parser.parse_program().expect("failed to parse");

        assert_eq!(
            program[0],
            Stmt::Assign {
                id: NodeId::next(),
                target: AssignTarget::Variable("n".to_string()),
                value: Expr::Binary {
                    id: NodeId::next(),
                    left: Box::new(Expr::Variable {
                        id: NodeId::next(),
                        name: "n".to_string(),
                    }),
                    op: BinOp::Add,
                    right: Box::new(Expr::int(2)),
                },
                is_const: false,
            }
        );
        assert_eq!(
            program[1],
            Stmt::AssignMany {
                id: NodeId::next(),
                targets: vec![
                    AssignTarget::Variable("a".to_string()),
                    AssignTarget::Variable("b".to_string()),
                ],
                values: vec![
                    Expr::Variable {
                        id: NodeId::next(),
                        name: "b".to_string(),
                    },
                    Expr::Variable {
                        id: NodeId::next(),
                        name: "a".to_string(),
                    },
                ],
            }
        );
    }

    #[test]
    fn parses_dotted_typed_function_bindings() {
        let mut parser = Parser::new(
            "function math.random(min: number, max: number): number\n    return 1\nend",
        );
        let (program, _pool) = parser.parse_program().expect("failed to parse");

        assert_eq!(
            program[0],
            Stmt::Function {
                id: NodeId::next(),
                name: Some("math.random".to_string()),
                is_const: false,
                params: vec![
                    super::Param {
                        name: "min".to_string(),
                        type_name: Some("number".to_string()),
                        variadic: false,
                    },
                    super::Param {
                        name: "max".to_string(),
                        type_name: Some("number".to_string()),
                        variadic: false,
                    },
                ],
                return_type: Some("number".to_string()),
                body: vec![Stmt::Return {
                    id: NodeId::next(),
                    values: vec![Expr::int(1)],
                }],
            }
        );
    }

    #[test]
    fn parses_typed_variadic_parameter_and_increment() {
        let mut parser = Parser::new(
            "function collect(first: number, ...: number): number\n    local total = first\n    total++\n    total--\n    return total\nend",
        );
        let (program, _pool) = parser.parse_program().expect("failed to parse");

        let Stmt::Function { params, body, .. } = &program[0] else {
            panic!("expected function");
        };
        assert_eq!(
            params[1],
            super::Param {
                name: "...".to_string(),
                type_name: Some("number".to_string()),
                variadic: true,
            }
        );
        assert!(matches!(body[1], Stmt::Increment { amount: 1, .. }));
        assert!(matches!(body[2], Stmt::Increment { amount: -1, .. }));
    }

    #[test]
    fn parses_numeric_for_loop() {
        let mut parser = Parser::new("for i = 1, 10, 2 do\n    print(i)\nend");
        let (program, _pool) = parser.parse_program().expect("failed to parse");

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
        let (program, _pool) = parser.parse_program().expect("failed to parse");

        assert!(matches!(
            &program[0],
            Stmt::Expr {
                expr: Expr::Call { args, .. },
                ..
            } if matches!(args.get(1), Some(Expr::Function { params, body, .. })
                if params.len() == 2 && body.len() == 1)
        ));
    }

    #[test]
    fn test_parser_returns_err_on_depth_limit() {
        // Deeply nested parentheses exceeding 120 should return Err, not panic
        let mut expr = "1".to_string();
        for _ in 0..130 {
            expr = format!("({})", expr);
        }
        let mut parser = Parser::new(&expr);
        let res = parser.parse_program();
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("parse recursion depth limit"));
    }

    #[test]
    fn test_parser_returns_err_on_syntax_errors() {
        let mut p1 = Parser::new("local x =");
        assert!(p1.parse_program().is_err());

        let mut p2 = Parser::new("if x then");
        assert!(p2.parse_program().is_err());

        let mut p3 = Parser::new("for i in do end");
        assert!(p3.parse_program().is_err());
    }

    #[test]
    fn parse_program_produces_real_spans() {
        let mut parser = Parser::new("local x = 42");
        let (program, pool) = parser.parse_program().expect("parse ok");
        let span = pool.get_or_dummy(program[0].node_id());
        assert_eq!(span.start.line, 1);
        assert_eq!(span.start.column, 1);
    }
}
