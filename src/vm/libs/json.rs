// JSON encoding and decoding library for Neyuki VM.

use num_bigint::BigInt;
use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

pub fn create_json_lib() -> Value {
    let t = Rc::new(RefCell::new(VmTable::new()));
    let mut b = t.borrow_mut();

    b.set_str("encode", Value::Native("json.encode", json_encode));
    b.set_str("decode", Value::Native("json.decode", json_decode));

    Value::Table(t.clone())
}

const MAX_JSON_DEPTH: usize = 256;

pub(crate) fn encode_to_string(val: &Value) -> Result<String, String> {
    let mut out = String::new();
    encode_value(val, &mut out, 0)?;
    Ok(out)
}

fn json_encode(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    let out = encode_to_string(val)?;
    Ok(vec![Value::String(out)])
}

fn encode_value(val: &Value, out: &mut String, depth: usize) -> Result<(), String> {
    if depth >= MAX_JSON_DEPTH {
        return Err("JSON nesting depth limit (256) exceeded during encode".to_string());
    }
    match val {
        Value::Nil => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                out.push_str("null");
            } else {
                out.push_str(&f.to_string());
            }
        }
        Value::String(s) => encode_string(s, out),
        Value::Table(t) => {
            let tbl = t.borrow();
            if !tbl.array.is_empty() && tbl.fields.is_empty() {
                // Encode as JSON array
                out.push('[');
                for (idx, item) in tbl.array.iter().enumerate() {
                    if idx > 0 {
                        out.push(',');
                    }
                    encode_value(item, out, depth + 1)?;
                }
                out.push(']');
            } else {
                // Encode as JSON object
                out.push('{');
                let mut first = true;
                for (key, item) in &tbl.fields {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    encode_string(key, out);
                    out.push(':');
                    encode_value(item, out, depth + 1)?;
                }
                // If array elements also exist in mixed table, include them as numeric string keys
                for (idx, item) in tbl.array.iter().enumerate() {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    encode_string(&(idx + 1).to_string(), out);
                    out.push(':');
                    encode_value(item, out, depth + 1)?;
                }
                out.push('}');
            }
        }
        _ => return Err(format!("cannot encode {} to JSON", val.type_name())),
    }
    Ok(())
}

fn encode_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            _ if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
}

pub(crate) fn decode_from_str(s: &str) -> Result<Value, String> {
    let mut parser = JsonParser::new(s);
    let val = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.cursor < parser.chars.len() {
        return Err(format!(
            "unexpected trailing data at position {}",
            parser.cursor
        ));
    }
    Ok(val)
}

fn json_decode(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = match args.first() {
        Some(Value::String(s)) => s.as_str(),
        _ => return Err("bad argument #1 to 'json.decode' (string expected)".to_string()),
    };

    let val = decode_from_str(s)?;
    Ok(vec![val])
}

struct JsonParser {
    chars: Vec<char>,
    cursor: usize,
    depth: usize,
}

impl JsonParser {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            cursor: 0,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.cursor).copied()
    }

    fn advance(&mut self) -> Option<char> {
        if self.cursor < self.chars.len() {
            let ch = self.chars[self.cursor];
            self.cursor += 1;
            Some(ch)
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.cursor += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_whitespace();
        let ch = self
            .peek()
            .ok_or_else(|| "unexpected end of JSON input".to_string())?;
        match ch {
            'n' => self.parse_null(),
            't' | 'f' => self.parse_bool(),
            '"' => self.parse_string().map(Value::String),
            '[' => self.parse_array(),
            '{' => self.parse_object(),
            '-' | '0'..='9' => self.parse_number(),
            _ => Err(format!(
                "unexpected character '{}' at position {}",
                ch, self.cursor
            )),
        }
    }

    fn parse_null(&mut self) -> Result<Value, String> {
        if self.chars[self.cursor..].starts_with(&['n', 'u', 'l', 'l']) {
            self.cursor += 4;
            Ok(Value::Nil)
        } else {
            Err(format!("invalid literal at position {}", self.cursor))
        }
    }

    fn parse_bool(&mut self) -> Result<Value, String> {
        if self.chars[self.cursor..].starts_with(&['t', 'r', 'u', 'e']) {
            self.cursor += 4;
            Ok(Value::Bool(true))
        } else if self.chars[self.cursor..].starts_with(&['f', 'a', 'l', 's', 'e']) {
            self.cursor += 5;
            Ok(Value::Bool(false))
        } else {
            Err(format!("invalid boolean at position {}", self.cursor))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.advance(); // consume opening '"'
        let mut out = String::new();
        while let Some(c) = self.advance() {
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let esc = self
                        .advance()
                        .ok_or_else(|| "unexpected end of escape".to_string())?;
                    match esc {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\x08'),
                        'f' => out.push('\x0c'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let mut hex = String::new();
                            for _ in 0..4 {
                                hex.push(
                                    self.advance()
                                        .ok_or_else(|| "unclosed unicode escape".to_string())?,
                                );
                            }
                            let cp = u32::from_str_radix(&hex, 16)
                                .map_err(|_| "invalid unicode escape".to_string())?;
                            let ch = char::from_u32(cp).ok_or_else(|| {
                                format!("invalid unicode codepoint \\u{:04x}", cp)
                            })?;
                            out.push(ch);
                        }
                        _ => return Err(format!("invalid escape character '\\{}'", esc)),
                    }
                }
                _ => out.push(c),
            }
        }
        Err("unclosed string literal".to_string())
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let mut num_str = String::new();
        let mut has_dot_or_exp = false;

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E' {
                if c == '.' || c == 'e' || c == 'E' {
                    has_dot_or_exp = true;
                }
                num_str.push(c);
                self.cursor += 1;
            } else {
                break;
            }
        }

        if has_dot_or_exp {
            let f = f64::from_str(&num_str)
                .map_err(|e| format!("invalid float '{}': {}", num_str, e))?;
            Ok(Value::Float(f))
        } else {
            let i = BigInt::from_str(&num_str)
                .map_err(|e| format!("invalid int '{}': {}", num_str, e))?;
            Ok(Value::Int(i))
        }
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        if self.depth >= MAX_JSON_DEPTH {
            return Err("JSON nesting depth limit (256) exceeded during decode".to_string());
        }
        self.depth += 1;
        let res = self.parse_array_inner();
        self.depth -= 1;
        res
    }

    fn parse_array_inner(&mut self) -> Result<Value, String> {
        self.advance(); // consume '['
        self.skip_whitespace();

        let tbl = Rc::new(RefCell::new(VmTable::new()));
        if self.peek() == Some(']') {
            self.advance();
            return Ok(Value::Table(tbl));
        }

        loop {
            let val = self.parse_value()?;
            tbl.borrow_mut().array.push(val);
            self.skip_whitespace();

            match self.peek() {
                Some(',') => {
                    self.advance();
                    self.skip_whitespace();
                }
                Some(']') => {
                    self.advance();
                    break;
                }
                _ => return Err(format!("expected ',' or ']' at position {}", self.cursor)),
            }
        }

        Ok(Value::Table(tbl))
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        if self.depth >= MAX_JSON_DEPTH {
            return Err("JSON nesting depth limit (256) exceeded during decode".to_string());
        }
        self.depth += 1;
        let res = self.parse_object_inner();
        self.depth -= 1;
        res
    }

    fn parse_object_inner(&mut self) -> Result<Value, String> {
        self.advance(); // consume '{'
        self.skip_whitespace();

        let tbl = Rc::new(RefCell::new(VmTable::new()));
        if self.peek() == Some('}') {
            self.advance();
            return Ok(Value::Table(tbl));
        }

        loop {
            self.skip_whitespace();
            if self.peek() != Some('"') {
                return Err(format!(
                    "expected string key in object at position {}",
                    self.cursor
                ));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();

            if self.peek() != Some(':') {
                return Err(format!(
                    "expected ':' after key at position {}",
                    self.cursor
                ));
            }
            self.advance(); // consume ':'

            let val = self.parse_value()?;
            tbl.borrow_mut().fields.insert(key, val);
            self.skip_whitespace();

            match self.peek() {
                Some(',') => {
                    self.advance();
                    self.skip_whitespace();
                }
                Some('}') => {
                    self.advance();
                    break;
                }
                _ => return Err(format!("expected ',' or '}}' at position {}", self.cursor)),
            }
        }

        Ok(Value::Table(tbl))
    }
}
