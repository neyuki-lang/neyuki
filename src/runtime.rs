use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{FromPrimitive, ToPrimitive, Zero};
use rand::{Rng, RngCore};

use crate::parser::{Expr, Param, Stmt};

const BUNDLED_LIBRARIES: &[(&str, &str)] =
    include!(concat!(env!("OUT_DIR"), "/bundled_libraries.rs"));

type EnvRef = Rc<RefCell<Env>>;
type Native = fn(Vec<Value>) -> Result<Vec<Value>, String>;

#[derive(Clone)]
enum Value {
    Nil,
    Bool(bool),
    Number(f64),
    Integer(BigInt),
    String(String),
    Table(Rc<RefCell<Table>>),
    Function(Rc<Function>),
    Varargs(Vec<Value>),
}

struct Table {
    pub array: Vec<Value>,
    pub fields: HashMap<String, Value>,
    pub const_fields: HashSet<String>,
    pub frozen: bool,
}

enum Function {
    Native {
        name: &'static str,
        call: Native,
    },
    User {
        params: Vec<Param>,
        body: Vec<Stmt>,
        env: EnvRef,
    },
}

struct Env {
    values: HashMap<String, Value>,
    const_names: HashSet<String>,
    parent: Option<EnvRef>,
}

enum Flow {
    Normal,
    Return(Vec<Value>),
    Break,
    Continue,
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(value) => write!(f, "{}", value),
            Value::Number(value) => write!(f, "{}", value),
            Value::Integer(value) => write!(f, "{}", value),
            Value::String(value) => write!(f, "{}", value),
            Value::Table(_) => write!(f, "table"),
            Value::Function(_) => write!(f, "function"),
            Value::Varargs(values) => write!(f, "varargs({})", values.len()),
        }
    }
}

impl Value {
    fn truthy_bool(&self) -> Result<bool, String> {
        match self {
            Value::Bool(value) => Ok(*value),
            _ => Err("condition must be boolean".to_string()),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "float",
            Value::Integer(_) => "bigint",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::Varargs(_) => "varargs",
        }
    }
}

pub fn run_file(path: &str) -> Result<(), String> {
    let program = crate::compiler::compile_file(path)?;
    let mut runtime = Runtime::new();
    runtime.execute(&program).map(|_| ())
}

pub struct Runtime {
    global: EnvRef,
}

impl Runtime {
    pub fn new() -> Self {
        let env = Rc::new(RefCell::new(Env {
            values: HashMap::new(),
            const_names: HashSet::new(),
            parent: None,
        }));
        for (name, function) in [
            ("print", native("print", builtin_print)),
            ("tostring", native("tostring", builtin_tostring)),
            ("type", native("type", builtin_type)),
            ("typeof", native("typeof", builtin_typeof)),
            ("assert", native("assert", builtin_assert)),
            ("error", native("error", builtin_error)),
            ("int", native("int", builtin_int)),
            ("float", native("float", builtin_float)),
            ("__floor", native("__floor", builtin_floor)),
            ("__sqrt", native("__sqrt", builtin_sqrt)),
            ("__random_int", native("__random_int", builtin_random_int)),
            (
                "__random_bigint",
                native("__random_bigint", builtin_random_bigint),
            ),
            (
                "__table_unpack",
                native("__table_unpack", builtin_table_unpack),
            ),
            (
                "__table_freeze",
                native("__table_freeze", builtin_table_freeze),
            ),
            (
                "__table_isfrozen",
                native("__table_isfrozen", builtin_table_isfrozen),
            ),
            ("try", native("try", builtin_try)),
            ("require", native("require", builtin_require)),
        ] {
            env.borrow_mut().values.insert(name.to_string(), function);
        }
        Self { global: env }
    }

    fn execute(&mut self, program: &[Stmt]) -> Result<Vec<Value>, String> {
        match self.exec_block(program, self.global.clone())? {
            Flow::Return(values) => Ok(values),
            Flow::Normal => Ok(Vec::new()),
            Flow::Break | Flow::Continue => Err("loop control used outside a loop".to_string()),
        }
    }

    fn exec_block(&self, block: &[Stmt], env: EnvRef) -> Result<Flow, String> {
        for statement in block {
            match self.exec_stmt(statement, env.clone())? {
                Flow::Normal => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_stmt(&self, stmt: &Stmt, env: EnvRef) -> Result<Flow, String> {
        match stmt {
            Stmt::Local {
                name,
                is_const,
                initializer,
                ..
            } => {
                let value = initializer
                    .as_ref()
                    .map(|expr| self.eval(expr, env.clone()))
                    .transpose()?
                    .unwrap_or(Value::Nil);
                let mut env = env.borrow_mut();
                env.values.insert(name.clone(), value);
                if *is_const {
                    env.const_names.insert(name.clone());
                }
            }
            Stmt::LocalMany {
                names,
                is_const,
                initializers,
            } => {
                let values = initializers
                    .iter()
                    .map(|expr| self.eval(expr, env.clone()))
                    .collect::<Result<Vec<_>, _>>()?;
                for (index, name) in names.iter().enumerate() {
                    env.borrow_mut().values.insert(
                        name.clone(),
                        values.get(index).cloned().unwrap_or(Value::Nil),
                    );
                    if *is_const {
                        env.borrow_mut().const_names.insert(name.clone());
                    }
                }
            }
            Stmt::Assign {
                target,
                value,
                is_const,
            } => {
                let value = self.eval(value, env.clone())?;
                self.assign(target, value, env.clone())?;
                if *is_const {
                    self.protect_member(target, env)?;
                }
            }
            Stmt::Increment { target, amount } => {
                let current = self.eval(target, env.clone())?;
                let value = self.numeric(current, "+", Value::Integer(BigInt::from(*amount)))?;
                self.assign(target, value, env)?;
            }
            Stmt::Function {
                name: Some(name),
                is_const,
                params,
                body,
                ..
            } => {
                let value = Value::Function(Rc::new(Function::User {
                    params: params.clone(),
                    body: body.clone(),
                    env: env.clone(),
                }));
                if name.contains('.') {
                    let mut target = Expr::Variable(name.split('.').next().unwrap().to_string());
                    for field in name.split('.').skip(1) {
                        target = Expr::Member {
                            object: Box::new(target),
                            field: field.to_string(),
                        };
                    }
                    self.assign(&target, value, env.clone())?;
                    if *is_const {
                        self.protect_member(&target, env)?;
                    }
                } else {
                    let mut env = env.borrow_mut();
                    env.values.insert(name.clone(), value);
                    if *is_const {
                        env.const_names.insert(name.clone());
                    }
                }
            }
            Stmt::Function { name: None, .. } => {
                return Err("anonymous function is only valid as an expression".to_string());
            }
            Stmt::Expr(expr) => {
                self.eval(expr, env)?;
            }
            Stmt::Return(expr) => {
                let value = expr
                    .as_ref()
                    .map(|expr| self.eval(expr, env))
                    .transpose()?
                    .unwrap_or(Value::Nil);
                let values = match value {
                    Value::Varargs(values) => values,
                    value => vec![value],
                };
                return Ok(Flow::Return(values));
            }
            Stmt::If {
                condition,
                then_branch,
                else_if_branches,
                else_branch,
            } => {
                if self.eval(condition, env.clone())?.truthy_bool()? {
                    return self.exec_block(then_branch, child(&env));
                }
                for (condition, branch) in else_if_branches {
                    if self.eval(condition, env.clone())?.truthy_bool()? {
                        return self.exec_block(branch, child(&env));
                    }
                }
                if let Some(branch) = else_branch {
                    return self.exec_block(branch, child(&env));
                }
            }
            Stmt::While { condition, body } => {
                while self.eval(condition, env.clone())?.truthy_bool()? {
                    match self.exec_block(body, child(&env))? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                }
            }
            Stmt::Repeat { body, condition } => loop {
                match self.exec_block(body, child(&env))? {
                    Flow::Normal | Flow::Continue => {}
                    Flow::Break => break,
                    flow => return Ok(flow),
                }
                if self.eval(condition, env.clone())?.truthy_bool()? {
                    break;
                }
            },
            Stmt::For { vars, source, body } => {
                let table = self.eval(source, env.clone())?;
                let values: Vec<Vec<Value>> = match table {
                    Value::Table(table) => {
                        let table = table.borrow();
                        if vars.len() > 1 && !table.array.is_empty() {
                            table
                                .array
                                .iter()
                                .enumerate()
                                .map(|(i, v)| vec![Value::Integer(BigInt::from(i + 1)), v.clone()])
                                .collect()
                        } else if vars.len() > 1 {
                            table
                                .fields
                                .iter()
                                .map(|(key, value)| vec![Value::String(key.clone()), value.clone()])
                                .collect()
                        } else {
                            table.array.iter().map(|v| vec![v.clone()]).collect()
                        }
                    }
                    _ => return Err("generic for expects a table in the base runtime".to_string()),
                };
                for values in values {
                    let loop_env = child(&env);
                    for (index, name) in vars.iter().enumerate() {
                        loop_env.borrow_mut().values.insert(
                            name.clone(),
                            values.get(index).cloned().unwrap_or(Value::Nil),
                        );
                    }
                    match self.exec_block(body, loop_env)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                }
            }
            Stmt::NumericFor {
                var,
                start,
                end,
                step,
                body,
            } => {
                let mut current = number(self.eval(start, env.clone())?)?;
                let limit = number(self.eval(end, env.clone())?)?;
                let increment = match step {
                    Some(step) => number(self.eval(step, env.clone())?)?,
                    None => 1.0,
                };
                if increment == 0.0 {
                    return Err("numeric for step cannot be zero".to_string());
                }

                while (increment > 0.0 && current <= limit) || (increment < 0.0 && current >= limit)
                {
                    let loop_env = child(&env);
                    let loop_value = if current.fract() == 0.0 {
                        Value::Integer(BigInt::from_f64(current).unwrap())
                    } else {
                        Value::Number(current)
                    };
                    loop_env.borrow_mut().values.insert(var.clone(), loop_value);
                    match self.exec_block(body, loop_env)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                    current += increment;
                }
            }
            Stmt::Break => return Ok(Flow::Break),
            Stmt::Continue => return Ok(Flow::Continue),
        }
        Ok(Flow::Normal)
    }

    fn eval(&self, expr: &Expr, env: EnvRef) -> Result<Value, String> {
        match expr {
            Expr::Literal(value) if value.contains('{') => self.interpolate(value, env),
            Expr::Literal(value) => parse_literal(value),
            Expr::Variable(name) => {
                lookup(&env, name).ok_or_else(|| format!("undefined name `{}`", name))
            }
            Expr::Vararg => {
                let values = lookup(&env, "__varargs")
                    .ok_or_else(|| "vararg expression outside a variadic function".to_string())?;
                match values {
                    Value::Table(values) => Ok(Value::Varargs(values.borrow().array.clone())),
                    _ => Ok(values),
                }
            }
            Expr::Member { object, field } => {
                self.index(&self.eval(object, env)?, &Value::String(field.clone()))
            }
            Expr::Index { object, index } => {
                self.index(&self.eval(object, env.clone())?, &self.eval(index, env)?)
            }
            Expr::Table(entries) => {
                let mut table = Table {
                    array: Vec::new(),
                    fields: HashMap::new(),
                    const_fields: HashSet::new(),
                    frozen: false,
                };
                for entry in entries {
                    if matches!(entry.value, Expr::Vararg) {
                        if let Some(Value::Table(values)) = lookup(&env, "__varargs") {
                            table.array.extend(values.borrow().array.iter().cloned());
                        }
                        continue;
                    }
                    let value = self.eval(&entry.value, env.clone())?;
                    match value {
                        Value::Varargs(values) => {
                            if let Some(key) = &entry.key {
                                table.fields.insert(key.clone(), Value::Varargs(values));
                            } else {
                                table.array.extend(values);
                            }
                        }
                        value => {
                            if let Some(key) = &entry.key {
                                table.fields.insert(key.clone(), value);
                            } else {
                                table.array.push(value);
                            }
                        }
                    }
                }
                Ok(Value::Table(Rc::new(RefCell::new(table))))
            }
            Expr::Unary { op, expr } => {
                let value = self.eval(expr, env)?;
                match op.as_str() {
                    "not" => Ok(Value::Bool(!value.truthy_bool()?)),
                    "-" => self.number_unary(value, true),
                    "#" => self.length(value),
                    _ => Err(format!("unsupported unary operator {}", op)),
                }
            }
            Expr::Binary { left, op, right } => self.binary(left, op, right, env),
            Expr::Call { callee, args } => {
                let function = self.eval(callee, env.clone())?;
                let mut values = Vec::new();
                for arg in args {
                    if matches!(arg, Expr::Vararg) {
                        if let Some(Value::Table(varargs)) = lookup(&env, "__varargs") {
                            values.extend(varargs.borrow().array.iter().cloned());
                        }
                    } else {
                        let value = self.eval(arg, env.clone())?;
                        match value {
                            Value::Varargs(varargs) => values.extend(varargs),
                            value => values.push(value),
                        }
                    }
                }
                self.call(function, values)
                    .map(|values| match values.as_slice() {
                        [] => Value::Nil,
                        [value] => value.clone(),
                        _ => Value::Varargs(values),
                    })
            }
            Expr::Function { params, body } => Ok(Value::Function(Rc::new(Function::User {
                params: params.clone(),
                body: body.clone(),
                env,
            }))),
        }
    }

    fn call(&self, function: Value, args: Vec<Value>) -> Result<Vec<Value>, String> {
        match function {
            Value::Function(function) => match &*function {
                Function::Native { name: "try", .. } => self.call_try(args),
                Function::Native {
                    name: "require", ..
                } => self.call_require(args),
                Function::Native { call, .. } => call(args),
                Function::User { params, body, env } => {
                    let call_env = child(env);
                    let mut arg_index = 0;
                    for param in params {
                        if param.variadic {
                            let values = Table {
                                array: args[arg_index..].to_vec(),
                                fields: HashMap::new(),
                                const_fields: HashSet::new(),
                                frozen: false,
                            };
                            call_env.borrow_mut().values.insert(
                                "__varargs".to_string(),
                                Value::Table(Rc::new(RefCell::new(values))),
                            );
                            arg_index = args.len();
                        } else {
                            call_env.borrow_mut().values.insert(
                                param.name.clone(),
                                args.get(arg_index).cloned().unwrap_or(Value::Nil),
                            );
                            arg_index += 1;
                        }
                    }
                    match self.exec_block(body, call_env)? {
                        Flow::Return(values) => Ok(values),
                        _ => Ok(vec![Value::Nil]),
                    }
                }
            },
            _ => Err("value is not callable".to_string()),
        }
    }

    fn call_try(&self, args: Vec<Value>) -> Result<Vec<Value>, String> {
        let Some(function) = args.first().cloned() else {
            return Err("try expects a function".to_string());
        };
        match self.call(function, args[1..].to_vec()) {
            Ok(mut values) => {
                values.insert(0, Value::Bool(true));
                Ok(values)
            }
            Err(error) => Ok(vec![Value::Bool(false), Value::String(error)]),
        }
    }

    fn call_require(&self, args: Vec<Value>) -> Result<Vec<Value>, String> {
        let Some(Value::String(package)) = args.first() else {
            return Err("require expects a string path".to_string());
        };
        let Some((_, source)) = BUNDLED_LIBRARIES.iter().find(|(name, _)| *name == package) else {
            return Err(format!(
                "package `{}` is not bundled; add it to the project manually",
                package
            ));
        };
        let program = crate::compiler::compile_source(source)?;
        let module_env = child(&self.global);
        match self.exec_block(&program, module_env)? {
            Flow::Return(values) => Ok(values),
            Flow::Normal => Ok(vec![Value::Nil]),
            Flow::Break | Flow::Continue => Err("loop control used outside a loop".to_string()),
        }
    }

    fn interpolate(&self, value: &str, env: EnvRef) -> Result<Value, String> {
        let mut output = String::new();
        let mut rest = value;
        while let Some(start) = rest.find('{') {
            output.push_str(&rest[..start]);
            let after_start = &rest[start + 1..];
            let end = after_start
                .find('}')
                .ok_or_else(|| "unfinished interpolation".to_string())?;
            let expression = &after_start[..end];
            let mut parser = crate::parser::Parser::new(expression);
            let expression =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parser.parse_program()))
                    .map_err(|_| "invalid interpolation expression".to_string())?;
            if expression.len() != 1 {
                return Err("interpolation must contain one expression".to_string());
            }
            let Stmt::Expr(expression) = &expression[0] else {
                return Err("interpolation must contain an expression".to_string());
            };
            output.push_str(&self.eval(expression, env.clone())?.to_string());
            rest = &after_start[end + 1..];
        }
        output.push_str(rest);
        Ok(Value::String(output))
    }

    fn binary(&self, left: &Expr, op: &str, right: &Expr, env: EnvRef) -> Result<Value, String> {
        let left = self.eval(left, env.clone())?;
        if op == "and" || op == "or" {
            let a = left.truthy_bool()?;
            if (op == "and" && !a) || (op == "or" && a) {
                return Ok(Value::Bool(a));
            }
            return Ok(Value::Bool(self.eval(right, env)?.truthy_bool()?));
        }
        let right = self.eval(right, env)?;
        match op {
            "??" => {
                if matches!(left, Value::Nil) {
                    Ok(right)
                } else {
                    Ok(left)
                }
            }
            ".." => Ok(Value::String(format!(
                "{}{}",
                require_string(left)?,
                require_string(right)?
            ))),
            "+" | "-" | "*" | "/" | "//" | "%" | "^" => self.numeric(left, op, right),
            "==" => Ok(Value::Bool(equal(&left, &right))),
            "!=" => Ok(Value::Bool(!equal(&left, &right))),
            "<" | "<=" | ">" | ">=" => compare(left, op, right),
            "&" | "|" | "~" | "<<" | ">>" => bitwise(left, op, right),
            _ => Err(format!("unsupported operator {}", op)),
        }
    }

    fn numeric(&self, left: Value, op: &str, right: Value) -> Result<Value, String> {
        if let (Value::Integer(a), Value::Integer(b)) = (&left, &right) {
            if (op == "//" || op == "%") && b.is_zero() {
                return Err("division by zero".to_string());
            }
            return match op {
                "+" => Ok(Value::Integer(a + b)),
                "-" => Ok(Value::Integer(a - b)),
                "*" => Ok(Value::Integer(a * b)),
                "/" => Ok(Value::Number(number(left)? / number(right)?)),
                "//" => Ok(Value::Integer(a.div_floor(b))),
                "%" => Ok(Value::Integer(a.mod_floor(b))),
                "^" if b.sign() != num_bigint::Sign::Minus => {
                    let exponent = b
                        .to_u32()
                        .ok_or_else(|| "integer exponent is too large".to_string())?;
                    Ok(Value::Integer(a.pow(exponent)))
                }
                _ => Ok(Value::Number(number(left)?.powf(number(right)?))),
            };
        }
        let a = number(left)?;
        let b = number(right)?;
        if (op == "//" || op == "%") && b == 0.0 {
            return Err("division by zero".to_string());
        }
        if op == "/" {
            return Ok(Value::Number(a / b));
        }
        let result = match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "//" => (a / b).floor(),
            "%" => a - (a / b).floor() * b,
            "^" => a.powf(b),
            _ => unreachable!(),
        };
        if result.fract() == 0.0 && result.is_finite() {
            Ok(Value::Integer(BigInt::from_f64(result).ok_or_else(
                || "integer result is out of range".to_string(),
            )?))
        } else {
            Ok(Value::Number(result))
        }
    }
    fn number_unary(&self, value: Value, negate: bool) -> Result<Value, String> {
        match value {
            Value::Integer(value) if negate => Ok(Value::Integer(-value)),
            Value::Integer(value) => Ok(Value::Integer(value)),
            Value::Number(value) if negate => Ok(Value::Number(-value)),
            Value::Number(value) => Ok(Value::Number(value)),
            _ => Err("expected a number".to_string()),
        }
    }
    fn length(&self, value: Value) -> Result<Value, String> {
        match value {
            Value::String(value) => Ok(Value::Integer(BigInt::from(value.len()))),
            Value::Table(value) => Ok(Value::Integer(BigInt::from(value.borrow().array.len()))),
            _ => Err("length expects a string or table".to_string()),
        }
    }
    fn index(&self, object: &Value, index: &Value) -> Result<Value, String> {
        match object {
            Value::Table(table) => {
                let table = table.borrow();
                match index {
                    Value::String(key) => Ok(table.fields.get(key).cloned().unwrap_or(Value::Nil)),
                    Value::Integer(index) if *index > BigInt::zero() => Ok(table
                        .array
                        .get(
                            index
                                .to_usize()
                                .ok_or_else(|| "table index is too large".to_string())?
                                - 1,
                        )
                        .cloned()
                        .unwrap_or(Value::Nil)),
                    _ => Err("table index must be a string or positive integer".to_string()),
                }
            }
            Value::String(_) => {
                Err("string members require a user-provided string package".to_string())
            }
            _ => Err("value is not indexable".to_string()),
        }
    }
    fn assign(&self, target: &Expr, value: Value, env: EnvRef) -> Result<(), String> {
        match target {
            Expr::Variable(name) => assign_env(&env, name, value),
            Expr::Member { object, field } => self.assign_index(
                &self.eval(object, env)?,
                Value::String(field.clone()),
                value,
            ),
            Expr::Index { object, index } => self.assign_index(
                &self.eval(object, env.clone())?,
                self.eval(index, env)?,
                value,
            ),
            _ => Err("invalid assignment target".to_string()),
        }
    }
    fn assign_index(&self, object: &Value, index: Value, value: Value) -> Result<(), String> {
        match object {
            Value::Table(table) => {
                let mut table = table.borrow_mut();
                if table.frozen {
                    return Err("assignment to frozen table".to_string());
                }
                match index {
                    Value::String(key) => {
                        if table.const_fields.contains(&key) {
                            return Err(format!("assignment to const field `{}`", key));
                        }
                        table.fields.insert(key, value);
                        Ok(())
                    }
                    Value::Integer(index) if index > BigInt::zero() => {
                        let index = index
                            .to_usize()
                            .ok_or_else(|| "table index is too large".to_string())?;
                        while table.array.len() < index {
                            table.array.push(Value::Nil);
                        }
                        table.array[index - 1] = value;
                        Ok(())
                    }
                    _ => Err("invalid table index".to_string()),
                }
            }
            _ => Err("assignment target is not a table".to_string()),
        }
    }

    fn protect_member(&self, target: &Expr, env: EnvRef) -> Result<(), String> {
        let Expr::Member { object, field } = target else {
            return Err("invalid const function target".to_string());
        };
        let value = self.eval(object, env)?;
        let Value::Table(table) = value else {
            return Err("const function target is not a table".to_string());
        };
        table.borrow_mut().const_fields.insert(field.clone());
        Ok(())
    }
}

fn native(name: &'static str, call: Native) -> Value {
    Value::Function(Rc::new(Function::Native { name, call }))
}
fn child(parent: &EnvRef) -> EnvRef {
    Rc::new(RefCell::new(Env {
        values: HashMap::new(),
        const_names: HashSet::new(),
        parent: Some(parent.clone()),
    }))
}
fn lookup(env: &EnvRef, name: &str) -> Option<Value> {
    env.borrow().values.get(name).cloned().or_else(|| {
        env.borrow()
            .parent
            .as_ref()
            .and_then(|parent| lookup(parent, name))
    })
}
fn assign_env(env: &EnvRef, name: &str, value: Value) -> Result<(), String> {
    if env.borrow().values.contains_key(name) {
        if env.borrow().const_names.contains(name) {
            return Err(format!("assignment to const name `{}`", name));
        }
        env.borrow_mut().values.insert(name.to_string(), value);
        Ok(())
    } else if let Some(parent) = env.borrow().parent.clone() {
        assign_env(&parent, name, value)
    } else {
        Err(format!("assignment to undefined name `{}`", name))
    }
}
fn parse_literal(value: &str) -> Result<Value, String> {
    match value {
        "nil" => Ok(Value::Nil),
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ => {
            let normalized = value.replace('_', "");
            let integer = if let Some(value) = normalized.strip_prefix("0x") {
                BigInt::parse_bytes(value.as_bytes(), 16)
            } else if let Some(value) = normalized.strip_prefix("0X") {
                BigInt::parse_bytes(value.as_bytes(), 16)
            } else if let Some(value) = normalized.strip_prefix("0b") {
                BigInt::parse_bytes(value.as_bytes(), 2)
            } else if let Some(value) = normalized.strip_prefix("0B") {
                BigInt::parse_bytes(value.as_bytes(), 2)
            } else {
                BigInt::parse_bytes(normalized.as_bytes(), 10)
            };
            if let Some(value) = integer {
                Ok(Value::Integer(value))
            } else if let Ok(value) = normalized.parse::<f64>() {
                Ok(Value::Number(value))
            } else {
                Ok(Value::String(value.to_string()))
            }
        }
    }
}
fn number(value: Value) -> Result<f64, String> {
    match value {
        Value::Integer(value) => value
            .to_f64()
            .ok_or_else(|| "integer is too large for floating-point conversion".to_string()),
        Value::Number(value) => Ok(value),
        _ => Err("expected a number".to_string()),
    }
}
fn require_string(value: Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value),
        _ => Err("expected a string".to_string()),
    }
}
fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Integer(a), Value::Number(b)) | (Value::Number(b), Value::Integer(a)) => {
            a.to_f64().is_some_and(|a| a == *b)
        }
        (Value::Number(a), Value::Number(b)) => a == b,
        _ => false,
    }
}
fn compare(left: Value, op: &str, right: Value) -> Result<Value, String> {
    let result = match (&left, &right) {
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => number(left)?
            .partial_cmp(&number(right)?)
            .ok_or_else(|| "values are not comparable".to_string())?,
    };
    Ok(Value::Bool(match op {
        "<" => result.is_lt(),
        "<=" => result.is_le(),
        ">" => result.is_gt(),
        ">=" => result.is_ge(),
        _ => false,
    }))
}
fn bitwise(left: Value, op: &str, right: Value) -> Result<Value, String> {
    let a = match left {
        Value::Integer(value) => value,
        value => BigInt::from_f64(number(value)?.trunc())
            .ok_or_else(|| "expected an integer".to_string())?,
    };
    let b = match right {
        Value::Integer(value) => value,
        value => BigInt::from_f64(number(value)?.trunc())
            .ok_or_else(|| "expected an integer".to_string())?,
    };
    Ok(Value::Integer(match op {
        "&" => a & b,
        "|" => a | b,
        "~" => a ^ b,
        "<<" => {
            a << b
                .to_usize()
                .ok_or_else(|| "shift is too large".to_string())?
        }
        ">>" => {
            a >> b
                .to_usize()
                .ok_or_else(|| "shift is too large".to_string())?
        }
        _ => return Err("unsupported bitwise operator".to_string()),
    }))
}
fn builtin_print(args: Vec<Value>) -> Result<Vec<Value>, String> {
    println!(
        "{}",
        args.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\t")
    );
    Ok(vec![Value::Nil])
}
fn builtin_tostring(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::String(
        args.first().cloned().unwrap_or(Value::Nil).to_string(),
    )])
}
fn builtin_type(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::String(
        match args.first().unwrap_or(&Value::Nil) {
            Value::Integer(_) | Value::Number(_) => "number",
            value => value.type_name(),
        }
        .to_string(),
    )])
}
fn builtin_typeof(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::String(
        args.first().unwrap_or(&Value::Nil).type_name().to_string(),
    )])
}
fn builtin_assert(args: Vec<Value>) -> Result<Vec<Value>, String> {
    if !args.first().unwrap_or(&Value::Nil).truthy_bool()? {
        return Err(args
            .get(1)
            .map(ToString::to_string)
            .unwrap_or_else(|| "assertion failed".to_string()));
    }
    Ok(args)
}
fn builtin_error(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Err(args
        .first()
        .map(ToString::to_string)
        .unwrap_or_else(|| "error".to_string()))
}
fn builtin_int(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = args.first().cloned().unwrap_or(Value::Nil);
    Ok(vec![match value {
        Value::Integer(value) => Value::Integer(value),
        value => Value::Integer(
            BigInt::from_f64(number(value)?.trunc())
                .ok_or_else(|| "value cannot be converted to an integer".to_string())?,
        ),
    }])
}
fn builtin_float(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Number(number(
        args.first().cloned().unwrap_or(Value::Nil),
    )?)])
}
fn builtin_floor(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = args.first().cloned().unwrap_or(Value::Nil);
    Ok(vec![match value {
        Value::Integer(value) => Value::Integer(value),
        value => Value::Integer(
            BigInt::from_f64(number(value)?.floor())
                .ok_or_else(|| "value cannot be converted to an integer".to_string())?,
        ),
    }])
}
fn builtin_sqrt(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = number(args.first().cloned().unwrap_or(Value::Nil))?;
    if !value.is_finite() {
        return Err("sqrt expects a finite number".to_string());
    }
    if value < 0.0 {
        return Err("sqrt expects a non-negative number".to_string());
    }
    Ok(vec![Value::Number(value.sqrt())])
}
fn builtin_random_int(args: Vec<Value>) -> Result<Vec<Value>, String> {
    if args
        .first()
        .is_some_and(|value| matches!(value, Value::Integer(_)))
        || args
            .get(1)
            .is_some_and(|value| matches!(value, Value::Integer(_)))
    {
        return builtin_random_bigint(args);
    }
    let min = integer_argument(args.first().cloned().unwrap_or(Value::Nil), "min")?;
    let max = integer_argument(args.get(1).cloned().unwrap_or(Value::Nil), "max")?;
    if min > max {
        return Err("random min must be less than or equal to max".to_string());
    }
    Ok(vec![Value::Integer(BigInt::from(
        rand::thread_rng().gen_range(min..=max),
    ))])
}

fn builtin_random_bigint(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let min = bigint_argument(args.first().cloned().unwrap_or(Value::Nil), "min")?;
    let max = bigint_argument(args.get(1).cloned().unwrap_or(Value::Nil), "max")?;
    if min > max {
        return Err("random min must be less than or equal to max".to_string());
    }

    let range = &max - &min;
    let bit_count = range.bits();
    let byte_count = bit_count.div_ceil(8) as usize;
    let excess_bits = (byte_count as u64 * 8).saturating_sub(bit_count);
    let mut bytes = vec![0; byte_count];
    let mut rng = rand::thread_rng();
    let offset = loop {
        rng.fill_bytes(&mut bytes);
        if excess_bits > 0 {
            bytes[0] &= u8::MAX >> excess_bits;
        }
        let offset = BigInt::from_bytes_be(Sign::Plus, &bytes);
        if offset <= range {
            break offset;
        }
    };

    Ok(vec![Value::Integer(min + offset)])
}

fn builtin_table_freeze(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = args.first().cloned().unwrap_or(Value::Nil);
    let Value::Table(table) = &value else {
        return Err("table.freeze expects a table".to_string());
    };
    table.borrow_mut().frozen = true;
    Ok(vec![value])
}

fn builtin_table_isfrozen(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Table(table)) = args.first() else {
        return Err("table.isfrozen expects a table".to_string());
    };
    Ok(vec![Value::Bool(table.borrow().frozen)])
}

fn builtin_table_unpack(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Table(table)) = args.first().cloned() else {
        return Err("table.unpack expects a table".to_string());
    };
    Ok(vec![Value::Varargs(table.borrow().array.clone())])
}
fn integer_argument(value: Value, name: &str) -> Result<i64, String> {
    let value = number(value)?;
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i64::MIN as f64
        || value > i64::MAX as f64
    {
        return Err(format!("random {} must be an integer", name));
    }
    Ok(value as i64)
}
fn bigint_argument(value: Value, name: &str) -> Result<BigInt, String> {
    match value {
        Value::Integer(value) => Ok(value),
        Value::Number(value) if value.is_finite() && value.fract() == 0.0 => {
            BigInt::from_f64(value).ok_or_else(|| format!("random {} must be an integer", name))
        }
        _ => Err(format!("random {} must be an integer", name)),
    }
}
fn builtin_try(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Function(function)) = args.first() else {
        return Err("try expects a function".to_string());
    };
    match &**function {
        Function::Native { call, .. } => match call(args[1..].to_vec()) {
            Ok(mut values) => {
                values.insert(0, Value::Bool(true));
                Ok(values)
            }
            Err(error) => Ok(vec![Value::Bool(false), Value::String(error)]),
        },
        Function::User { .. } => Ok(vec![
            Value::Bool(false),
            Value::String("user function try is unavailable in this base runtime".to_string()),
        ]),
    }
}
fn builtin_require(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    Err("require must be called through the runtime".to_string())
}
