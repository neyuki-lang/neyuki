// Statement parsing implementation for Neyuki.

use super::Parser;
use super::action_helper::ActionHelper;
use super::expr::parse_expr;
use super::types::read_type_annotation;
use crate::ast::*;

pub fn parse_statement(parser: &mut Parser) -> Result<Stmt, String> {
    let start = parser.current_loc();
    let stmt = parse_statement_inner(parser)?;
    parser.record_span(stmt.node_id(), start);
    Ok(stmt)
}

pub fn parse_statement_inner(parser: &mut Parser) -> Result<Stmt, String> {
    if parser.check_keyword("local")
        || parser.check_keyword("const")
        || parser.check_keyword("global")
    {
        return parse_binding(parser);
    }
    if parser.check_keyword("function") {
        return parse_function(parser, false);
    }
    if parser.check_keyword("if") {
        return parse_if(parser);
    }
    if parser.check_keyword("for") {
        return parse_for(parser);
    }
    if parser.check_keyword("while") {
        return parse_while(parser);
    }
    if parser.check_keyword("repeat") {
        return parse_repeat(parser);
    }
    if parser.check_keyword("return") {
        parser.pos += 1;
        let expr = if !parser.is_eof()
            && !parser.check_keyword("end")
            && !parser.check_keyword("else")
            && !parser.check_keyword("elseif")
            && !parser.check_keyword("until")
        {
            let mut values = vec![parse_expr(parser)?];
            while parser.match_symbol(",") {
                values.push(parse_expr(parser)?);
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
    if parser.check_keyword("break") {
        parser.pos += 1;
        return Ok(Stmt::Break { id: NodeId::next() });
    }
    if parser.check_keyword("continue") {
        parser.pos += 1;
        return Ok(Stmt::Continue { id: NodeId::next() });
    }

    let expr = parse_expr(parser)?;
    if parser.match_symbol("++") {
        let target = AssignTarget::from_expr(expr)?;
        return Ok(Stmt::Increment {
            id: NodeId::next(),
            target,
            amount: 1,
        });
    }
    if parser.match_symbol("--") {
        let target = AssignTarget::from_expr(expr)?;
        return Ok(Stmt::Increment {
            id: NodeId::next(),
            target,
            amount: -1,
        });
    }
    if parser.match_symbol("=") {
        let target = AssignTarget::from_expr(expr)?;
        let value = parse_expr(parser)?;
        return Ok(Stmt::Assign {
            id: NodeId::next(),
            target,
            value,
            is_const: false,
        });
    }
    if parser.check_symbol(",") {
        let mut targets = vec![AssignTarget::from_expr(expr)?];
        while parser.match_symbol(",") {
            let next_expr = parse_expr(parser)?;
            targets.push(AssignTarget::from_expr(next_expr)?);
        }
        parser.expect_symbol("=")?;
        let mut values = vec![parse_expr(parser)?];
        while parser.match_symbol(",") {
            values.push(parse_expr(parser)?);
        }
        return Ok(Stmt::AssignMany {
            id: NodeId::next(),
            targets,
            values,
        });
    }
    if let Some(op) = match_compound_assignment(parser) {
        let target = AssignTarget::from_expr_ref(&expr)?;
        let right = parse_expr(parser)?;
        let bin_op = BinOp::parse_str(&op).ok_or_else(|| {
            format!(
                "[line {}] unknown compound operator '{}'",
                parser.peek().line,
                op
            )
        })?;
        let bin_start = parser
            .span_pool
            .get(expr.node_id())
            .map(|s| s.start)
            .unwrap_or_else(|| parser.current_loc());
        let bin_expr = Expr::Binary {
            id: NodeId::next(),
            left: Box::new(expr),
            op: bin_op,
            right: Box::new(right),
        };
        parser.record_span(bin_expr.node_id(), bin_start);
        return Ok(Stmt::Assign {
            id: NodeId::next(),
            target,
            value: bin_expr,
            is_const: false,
        });
    }
    parser.consume_bracket_attributes();
    Ok(Stmt::Expr {
        id: NodeId::next(),
        expr,
    })
}

pub fn parse_binding(parser: &mut Parser) -> Result<Stmt, String> {
    let mut is_const = false;
    while parser.check_keyword("local")
        || parser.check_keyword("const")
        || parser.check_keyword("global")
    {
        is_const |= parser.check_keyword("const");
        parser.pos += 1;
    }

    if parser.check_keyword("function") {
        return parse_function(parser, is_const);
    }

    let mut name = parser.expect_name()?;
    while parser.match_symbol(".") {
        name.push('.');
        name.push_str(&parser.expect_name()?);
    }
    if name.contains('.') {
        parser.expect_symbol("=")?;
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
            value: parse_expr(parser)?,
            is_const,
        });
    }
    if parser.check_symbol(",") {
        let mut names = vec![name];
        while parser.match_symbol(",") {
            names.push(parser.expect_name()?);
        }
        parser.expect_symbol("=")?;
        let mut initializers = vec![parse_expr(parser)?];
        while parser.match_symbol(",") {
            initializers.push(parse_expr(parser)?);
        }
        return Ok(Stmt::LocalMany {
            id: NodeId::next(),
            names,
            is_const,
            initializers,
        });
    }
    let type_name = if parser.match_symbol(":") {
        Some(read_type_annotation(parser))
    } else {
        None
    };

    let initializer = if parser.match_symbol("=") {
        Some(parse_expr(parser)?)
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

pub fn parse_function(parser: &mut Parser, is_const: bool) -> Result<Stmt, String> {
    parser.expect_keyword("function")?;

    let name = if parser.peek().kind == "name" {
        let mut name = parser.expect_name()?;
        while parser.match_symbol(".") {
            name.push('.');
            name.push_str(&parser.expect_name()?);
        }
        Some(name)
    } else {
        None
    };

    let params = if parser.match_symbol("(") {
        parse_param_list(parser)?
    } else {
        Vec::new()
    };

    let return_type = if parser.match_symbol(":") {
        Some(read_type_annotation(parser))
    } else {
        None
    };

    let body = parse_block_until(parser, "end")?;
    parser.expect_keyword("end")?;
    Ok(Stmt::Function {
        id: NodeId::next(),
        name,
        is_const,
        params,
        return_type,
        body,
    })
}

pub fn parse_if(parser: &mut Parser) -> Result<Stmt, String> {
    parser.expect_keyword("if")?;
    let condition = parse_expr(parser)?;
    parser.expect_keyword("then")?;

    let then_branch = parse_block_until_any(parser, &["else", "elseif", "end"])?;
    let mut else_if_branches = Vec::new();
    let mut else_branch = None;

    while parser.check_keyword("elseif") {
        parser.expect_keyword("elseif")?;
        let next_condition = parse_expr(parser)?;
        parser.expect_keyword("then")?;
        let next_branch = parse_block_until_any(parser, &["else", "elseif", "end"])?;
        else_if_branches.push((next_condition, next_branch));
    }

    if parser.check_keyword("else") {
        parser.expect_keyword("else")?;
        else_branch = Some(parse_block_until(parser, "end")?);
    }

    parser.expect_keyword("end")?;

    Ok(Stmt::If {
        id: NodeId::next(),
        condition,
        then_branch,
        else_if_branches,
        else_branch,
    })
}

pub fn parse_for(parser: &mut Parser) -> Result<Stmt, String> {
    parser.expect_keyword("for")?;
    let first_name = parser.expect_name()?;
    if parser.match_symbol("=") {
        let start = parse_expr(parser)?;
        parser.expect_symbol(",")?;
        let end = parse_expr(parser)?;
        let step = if parser.match_symbol(",") {
            Some(parse_expr(parser)?)
        } else {
            None
        };
        parser.expect_keyword("do")?;
        let body = parse_block_until(parser, "end")?;
        parser.expect_keyword("end")?;
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
    if parser.match_symbol(",") {
        vars.push(parser.expect_name()?);
    }
    parser.expect_keyword("in")?;
    let source = parse_expr(parser)?;
    parser.expect_keyword("do")?;
    let body = parse_block_until(parser, "end")?;
    parser.expect_keyword("end")?;
    Ok(Stmt::For {
        id: NodeId::next(),
        vars,
        source,
        body,
    })
}

pub fn parse_while(parser: &mut Parser) -> Result<Stmt, String> {
    parser.expect_keyword("while")?;
    let condition = parse_expr(parser)?;
    parser.expect_keyword("do")?;
    let body = parse_block_until(parser, "end")?;
    parser.expect_keyword("end")?;
    Ok(Stmt::While {
        id: NodeId::next(),
        condition,
        body,
    })
}

pub fn parse_repeat(parser: &mut Parser) -> Result<Stmt, String> {
    parser.expect_keyword("repeat")?;
    let body = parse_block_until(parser, "until")?;
    parser.expect_keyword("until")?;
    let condition = parse_expr(parser)?;
    Ok(Stmt::Repeat {
        id: NodeId::next(),
        body,
        condition,
    })
}

pub fn parse_param_list(parser: &mut Parser) -> Result<Vec<Param>, String> {
    let mut params = Vec::new();
    if parser.check_symbol(")") {
        parser.expect_symbol(")")?;
        return Ok(params);
    }

    loop {
        if parser.match_symbol("...") {
            let type_name = if parser.match_symbol(":") {
                Some(read_type_annotation(parser))
            } else {
                None
            };
            params.push(Param {
                name: "...".to_string(),
                type_name,
                variadic: true,
            });
            parser.expect_symbol(")")?;
            break;
        }
        let name = parser.expect_name()?;
        let type_name = if parser.match_symbol(":") {
            Some(read_type_annotation(parser))
        } else {
            None
        };
        params.push(Param {
            name,
            type_name,
            variadic: false,
        });

        if parser.match_symbol(",") {
            continue;
        }
        parser.expect_symbol(")")?;
        break;
    }
    Ok(params)
}

pub fn parse_block_until(parser: &mut Parser, end_kw: &str) -> Result<Vec<Stmt>, String> {
    parse_block_until_any(parser, &[end_kw])
}

pub fn parse_block_until_any(parser: &mut Parser, end_words: &[&str]) -> Result<Vec<Stmt>, String> {
    let mut block = Vec::new();
    while !parser.is_eof() {
        if parser.check_symbol(";") {
            parser.pos += 1;
            continue;
        }
        if end_words.iter().any(|word| parser.check_keyword(word)) {
            break;
        }
        block.push(parse_statement(parser)?);
    }
    Ok(block)
}

pub fn match_compound_assignment(parser: &mut Parser) -> Option<String> {
    let token = parser.peek();
    if token.kind != "symbol" {
        return None;
    }
    if ActionHelper::compound_assign_binop(&token.value).is_none() && token.value != "??=" {
        return None;
    }
    let op = token.value[..token.value.len() - 1].to_string();
    parser.pos += 1;
    Some(op)
}
