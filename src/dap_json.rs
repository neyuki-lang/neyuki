// Minimal JSON value model plus DAP wire framing (`Content-Length`
// headers over a byte stream). No third-party crates: the parser is a
// small recursive-descent implementation covering exactly what the debug
// adapter speaks (objects, arrays, strings with escapes incl. `\u` +
// surrogate pairs, numbers, booleans, null). Malformed input is always a
// recoverable `Err`, never a panic.

use std::io::{self, BufRead, Write};

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_usize(&self) -> Option<usize> {
        match self {
            Json::Num(n) if *n >= 0.0 && n.fract() == 0.0 => Some(*n as usize),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "json parse error: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn err<T>(&self, msg: &str) -> Result<T, ParseError> {
        Err(ParseError(format!("{} at byte {}", msg, self.pos)))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn eat(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Json, ParseError> {
        self.skip_ws();
        match self.peek() {
            None => self.err("unexpected end of input"),
            Some(b'n') => self.parse_literal("null", Json::Null),
            Some(b't') => self.parse_literal("true", Json::Bool(true)),
            Some(b'f') => self.parse_literal("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(ParseError(format!("unexpected byte 0x{:02x}", c))),
        }
    }

    fn parse_literal(&mut self, word: &str, val: Json) -> Result<Json, ParseError> {
        if self.bytes.get(self.pos..self.pos + word.len()) == Some(word.as_bytes()) {
            self.pos += word.len();
            Ok(val)
        } else {
            self.err("invalid literal")
        }
    }

    fn hex_val(b: u8) -> Option<u32> {
        match b {
            b'0'..=b'9' => Some((b - b'0') as u32),
            b'a'..=b'f' => Some((b - b'a' + 10) as u32),
            b'A'..=b'F' => Some((b - b'A' + 10) as u32),
            _ => None,
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, ParseError> {
        let mut v = 0u32;
        for _ in 0..4 {
            match self.eat().and_then(Self::hex_val) {
                Some(d) => v = v * 16 + d,
                None => return self.err("bad \\u escape"),
            }
        }
        Ok(v)
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        // Opening quote already peeked.
        self.eat();
        // Raw bytes: literal (possibly multi-byte) UTF-8 passes through,
        // escapes append the encoded bytes of the resulting char.
        let mut out: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 4];
        // `emit` is a closure-free helper to keep borrows simple.
        macro_rules! emit {
            ($ch:expr) => {{
                let s = ($ch).encode_utf8(&mut tmp);
                out.extend_from_slice(s.as_bytes());
            }};
        }
        loop {
            match self.eat() {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    return String::from_utf8(out)
                        .map_err(|_| ParseError("invalid UTF-8".to_string()));
                }
                Some(b'\\') => match self.eat() {
                    None => return self.err("unterminated escape"),
                    Some(b'"') => out.push(b'"'),
                    Some(b'\\') => out.push(b'\\'),
                    Some(b'/') => out.push(b'/'),
                    Some(b'b') => emit!('\u{0008}'),
                    Some(b'f') => emit!('\u{000C}'),
                    Some(b'n') => out.push(b'\n'),
                    Some(b'r') => out.push(b'\r'),
                    Some(b't') => out.push(b'\t'),
                    Some(b'u') => {
                        let hi = self.parse_hex4()?;
                        // High surrogate: must be followed by \uDC00-\uDFFF.
                        if (0xD800..0xDC00).contains(&hi) {
                            match (self.eat(), self.eat()) {
                                (Some(b'\\'), Some(b'u')) => {
                                    let lo = self.parse_hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return self.err("bad low surrogate");
                                    }
                                    let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                    match char::from_u32(cp) {
                                        Some(ch) => emit!(ch),
                                        None => return self.err("bad codepoint"),
                                    }
                                }
                                _ => return self.err("lone high surrogate"),
                            }
                        } else if (0xDC00..0xE000).contains(&hi) {
                            return self.err("lone low surrogate");
                        } else {
                            match char::from_u32(hi) {
                                Some(ch) => emit!(ch),
                                None => return self.err("bad codepoint"),
                            }
                        }
                    }
                    Some(_) => return self.err("bad escape"),
                },
                Some(b) if b < 0x20 => return self.err("unescaped control character"),
                Some(b) => out.push(b),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Json, ParseError> {
        self.eat(); // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.eat();
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.eat();
                }
                Some(b']') => {
                    self.eat();
                    return Ok(Json::Arr(items));
                }
                None => return self.err("unterminated array"),
                _ => return self.err("expected ',' or ']'"),
            }
        }
    }

    fn parse_object(&mut self) -> Result<Json, ParseError> {
        self.eat(); // '{'
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.eat();
            return Ok(Json::Obj(pairs));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return self.err("expected string key");
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.eat() != Some(b':') {
                return self.err("expected ':'");
            }
            let val = self.parse_value()?;
            pairs.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.eat();
                }
                Some(b'}') => {
                    self.eat();
                    return Ok(Json::Obj(pairs));
                }
                None => return self.err("unterminated object"),
                _ => return self.err("expected ',' or '}'"),
            }
        }
    }

    fn parse_number(&mut self) -> Result<Json, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let mut digits = 0;
        while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
            self.pos += 1;
            digits += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.pos += 1;
                digits += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let mut exp_digits = 0;
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.pos += 1;
                exp_digits += 1;
            }
            if exp_digits == 0 {
                return self.err("bad exponent");
            }
        }
        if digits == 0 {
            return self.err("bad number");
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| ParseError("bad number".to_string()))?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| ParseError("bad number".to_string()))
    }

    fn finish(&mut self) -> Result<(), ParseError> {
        self.skip_ws();
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            self.err("trailing bytes")
        }
    }
}

/// Parses one complete JSON document; trailing garbage is an error.
pub fn parse_json(bytes: &[u8]) -> Result<Json, ParseError> {
    let mut p = Parser::new(bytes);
    let v = p.parse_value()?;
    p.finish()?;
    Ok(v)
}

fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}

/// Serializes compactly (no whitespace).
pub fn render_json(val: &Json) -> String {
    let mut out = String::new();
    render_into(&mut out, val);
    out
}

fn render_into(out: &mut String, val: &Json) {
    match val {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Num(n) => {
            if n.fract() == 0.0 && n.is_finite() && n.abs() < 1e21 {
                out.push_str(&format!("{}", *n as i64));
            } else {
                out.push_str(&format!("{}", n));
            }
        }
        Json::Str(s) => {
            out.push('"');
            escape_into(out, s);
            out.push('"');
        }
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                render_into(out, item);
            }
            out.push(']');
        }
        Json::Obj(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                escape_into(out, k);
                out.push_str("\":");
                render_into(out, v);
            }
            out.push('}');
        }
    }
}

/// Reads one DAP-framed message (`Content-Length: N\r\n\r\n` + N bytes).
/// Returns `Ok(None)` on clean EOF before any header byte.
pub fn read_message<R: BufRead>(input: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = input.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse::<usize>().ok();
        }
    }
    let len = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Writes one DAP-framed message.
pub fn write_message<W: Write>(out: &mut W, body: &[u8]) -> io::Result<()> {
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(body)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(val: &Json) {
        let s = render_json(val);
        let back = parse_json(s.as_bytes()).expect("roundtrip");
        assert_eq!(&back, val);
    }

    #[test]
    fn json_roundtrip_nested() {
        let v = Json::Obj(vec![
            ("seq".to_string(), Json::Num(3.0)),
            ("type".to_string(), Json::Str("request".to_string())),
            (
                "arguments".to_string(),
                Json::Obj(vec![
                    ("threadId".to_string(), Json::Num(1.0)),
                    ("path".to_string(), Json::Str("a\"b\\c\né✓".to_string())),
                    (
                        "list".to_string(),
                        Json::Arr(vec![Json::Null, Json::Bool(true)]),
                    ),
                ]),
            ),
        ]);
        rt(&v);
        // Integer-valued floats render without a decimal point.
        assert!(render_json(&Json::Num(3.0)).contains('3'));
    }

    #[test]
    fn json_surrogates_and_escapes() {
        // U+1F600 via surrogate pair; é direct; classic escapes.
        let v = parse_json("\"\\uD83D\\uDE00é\\n\\t\\\"\\\\\"".as_bytes()).expect("parse");
        assert_eq!(v, Json::Str("\u{1F600}é\n\t\"\\".to_string()));
        rt(&v);
    }

    #[test]
    fn json_accepts_whitespace() {
        // Real clients (and Python's json.dumps) emit spaces after separators.
        let v = parse_json(
            "{ \"seq\" : 1 , \"args\" : { \"a\" : [1 , 2] } , \"t\" : true }".as_bytes(),
        )
        .expect("spaced json");
        assert_eq!(v.get("seq"), Some(&Json::Num(1.0)), "spaced object parses");
        rt(&v);
    }

    #[test]
    fn json_rejects_garbage() {
        for bad in [
            "",
            "{",
            "{\"a\":}",
            "[1,]",
            "\"unterminated",
            "nul",
            "01a",
            "{\"a\" 1}",
            "[1 2]",
            "\"bad \\x escape\"",
            "\"lone \\uD83D end\"",
        ] {
            assert!(parse_json(bad.as_bytes()).is_err(), "input: {}", bad);
        }
    }

    #[test]
    fn framing_two_messages_back_to_back() {
        let m1 = br#"{"seq":1}"#;
        let m2 = br#"{"seq":2}"#;
        let mut raw = Vec::new();
        raw.extend_from_slice(format!("Content-Length: {}\r\n\r\n", m1.len()).as_bytes());
        raw.extend_from_slice(m1);
        raw.extend_from_slice(format!("Content-Length: {}\r\n\r\n", m2.len()).as_bytes());
        raw.extend_from_slice(m2);
        let mut cursor = io::Cursor::new(raw);
        let r1 = read_message(&mut cursor).expect("read1").expect("some1");
        let r2 = read_message(&mut cursor).expect("read2").expect("some2");
        assert_eq!(r1, m1);
        assert_eq!(r2, m2);
        assert!(read_message(&mut cursor).expect("eof").is_none());
    }

    #[test]
    fn framing_missing_length_errors() {
        let raw = b"X-Other: 1\r\n\r\n{}";
        let mut cursor = io::Cursor::new(raw.to_vec());
        assert!(read_message(&mut cursor).is_err());
    }
}
