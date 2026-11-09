//! Native HTML parsing, querying and writing backing `@neyuki/html`.
//!
//! Parsing goes through html5ever, the WHATWG parser from Servo (by way of
//! `kuchikiki`'s tree), so broken markup is repaired the way a browser
//! repairs it. The result is plain tables a script can read and change:
//!
//! ```text
//! { type = "document", children = { ... } }        also "fragment"
//! { type = "element", tag = "a", attrs = { href = "/" }, children = { ... } }
//! { type = "text", text = "..." }
//! { type = "comment", text = "..." }
//! { type = "doctype", name = "html" }
//! ```
//!
//! A plain string in `children` is a text node too. `select` matches CSS
//! selectors against such a tree, `text` gathers its text and `render`
//! writes it back as HTML. `lib/html.nyk` checks the arguments.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use html5ever::{LocalName, Namespace, QualName};
use kuchikiki::traits::TendrilSink;
use kuchikiki::{NodeData, NodeRef};

use crate::vm::libs::args::{array, bool_arg, str_arg, table_arg};
use crate::vm::machine::VM;
use crate::vm::value::{NativeDef, StrRef, Value, VmTable};

/// How deeply nodes may nest. Deeper documents are an error rather than a
/// stack overflow, and a node table that contains itself stops here too.
const MAX_DEPTH: usize = 512;

/// The longest text `render` produces, as for `json.encode`.
const MAX_OUTPUT: usize = 64 * 1024 * 1024;

/// Elements that never have children or an end tag.
const VOID: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "meta", "param", "source", "track", "wbr",
];

/// Elements whose text is written as is, without escaping.
const RAW_TEXT: &[&str] = &[
    "iframe",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
    "script",
    "style",
    "xmp",
];

pub(crate) const NATIVES: &[NativeDef] = &[
    NativeDef {
        name: "__html_parse",
        func: builtin_parse,
    },
    NativeDef {
        name: "__html_select",
        func: builtin_select,
    },
    NativeDef {
        name: "__html_text",
        func: builtin_text,
    },
    NativeDef {
        name: "__html_render",
        func: builtin_render,
    },
    NativeDef {
        name: "__html_escape",
        func: builtin_escape,
    },
];

fn new_table(table: VmTable) -> Value {
    Value::Table(Rc::new(RefCell::new(table)))
}

fn depth_error() -> String {
    format!("nesting is deeper than {} levels", MAX_DEPTH)
}

// ---------------------------------------------------------------------------
// parse
// ---------------------------------------------------------------------------

/// `__html_parse(text, fragment)`: a whole document, or with `fragment` the
/// nodes of a piece of markup as if it sat inside `<body>`.
fn builtin_parse(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let text = str_arg(args, 0, "text")?;
    let fragment = bool_arg(args, 1, "fragment")?;
    Ok(vec![parse(text, fragment)?])
}

fn parse(text: &str, fragment: bool) -> Result<Value, String> {
    let mut root = VmTable::new();
    if fragment {
        let context = QualName::new(
            None,
            Namespace::from("http://www.w3.org/1999/xhtml"),
            LocalName::from("body"),
        );
        let document = kuchikiki::parse_fragment(context, Vec::new()).one(text);
        // html5ever puts the nodes of a fragment under an `html` element.
        let holder = document.first_child().unwrap_or(document);
        root.set_str("type", Value::str("fragment"));
        root.set_str("children", convert_children(&holder, 0)?);
    } else {
        let document = kuchikiki::parse_html().one(text);
        root.set_str("type", Value::str("document"));
        root.set_str("children", convert_children(&document, 0)?);
    }
    Ok(new_table(root))
}

fn convert_children(node: &NodeRef, depth: usize) -> Result<Value, String> {
    if depth >= MAX_DEPTH {
        return Err(depth_error());
    }
    let mut items = Vec::new();
    for child in node.children() {
        if let Some(value) = convert(&child, depth)? {
            items.push(value);
        }
    }
    Ok(array(items))
}

fn convert(node: &NodeRef, depth: usize) -> Result<Option<Value>, String> {
    let mut table = VmTable::new();
    match node.data() {
        NodeData::Element(element) => {
            table.set_str("type", Value::str("element"));
            table.set_str("tag", Value::string(element.name.local.to_string()));
            let mut attrs = VmTable::new();
            for (name, attribute) in element.attributes.borrow().map.iter() {
                let key = match &attribute.prefix {
                    Some(prefix) => format!("{}:{}", prefix, name.local),
                    None => name.local.to_string(),
                };
                attrs
                    .fields
                    .insert(StrRef::from(key), Value::string(attribute.value.clone()));
            }
            table.set_str("attrs", new_table(attrs));
            // A `<template>` keeps its children in a separate fragment.
            let contents = element.template_contents.as_ref().unwrap_or(node);
            table.set_str("children", convert_children(contents, depth + 1)?);
        }
        NodeData::Text(text) => {
            table.set_str("type", Value::str("text"));
            table.set_str("text", Value::string(text.borrow().clone()));
        }
        NodeData::Comment(text) => {
            table.set_str("type", Value::str("comment"));
            table.set_str("text", Value::string(text.borrow().clone()));
        }
        NodeData::Doctype(doctype) => {
            table.set_str("type", Value::str("doctype"));
            table.set_str("name", Value::string(doctype.name.clone()));
        }
        NodeData::Document(_) | NodeData::DocumentFragment => {
            table.set_str("type", Value::str("document"));
            table.set_str("children", convert_children(node, depth + 1)?);
        }
        // HTML has no processing instructions; html5ever reads them as
        // comments.
        NodeData::ProcessingInstruction(_) => return Ok(None),
    }
    Ok(Some(new_table(table)))
}

// ---------------------------------------------------------------------------
// node tables
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A document or a fragment.
    Document,
    Element,
    Text,
    Comment,
    Doctype,
}

/// What a table is, from its `type`. A table without one is an element if
/// it has a `tag`, a text node if it has a `text`, and a fragment if it
/// only has `children`.
fn node_kind(table: &VmTable) -> Option<Kind> {
    match table.get_str("type") {
        Value::String(s) => match &*s {
            "document" | "fragment" => Some(Kind::Document),
            "element" => Some(Kind::Element),
            "text" => Some(Kind::Text),
            "comment" => Some(Kind::Comment),
            "doctype" => Some(Kind::Doctype),
            _ => None,
        },
        Value::Nil if matches!(table.get_str("tag"), Value::String(_)) => Some(Kind::Element),
        Value::Nil if matches!(table.get_str("text"), Value::String(_)) => Some(Kind::Text),
        Value::Nil if matches!(table.get_str("children"), Value::Table(_)) => Some(Kind::Document),
        _ => None,
    }
}

fn child_list(table: &VmTable) -> Vec<Value> {
    match table.get_str("children") {
        Value::Table(children) => children.borrow().array.clone(),
        _ => Vec::new(),
    }
}

fn text_field(table: &VmTable, key: &str) -> String {
    match table.get_str(key) {
        Value::String(s) => s.to_string(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

/// `__html_text(node)`: the text of a node and everything under it, as the
/// DOM's `textContent` gives it.
fn builtin_text(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let node = match args.first() {
        Some(value @ (Value::Table(_) | Value::String(_))) => value,
        None | Some(Value::Nil) => return Err("node must be provided".to_string()),
        Some(other) => {
            return Err(format!("node must be a table, got {}", other.type_name()));
        }
    };
    let mut out = String::new();
    collect_text(node, &mut out, 0)?;
    Ok(vec![Value::string(out)])
}

fn collect_text(value: &Value, out: &mut String, depth: usize) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err(depth_error());
    }
    match value {
        Value::String(s) => out.push_str(s),
        Value::Table(t) => {
            let (kind, text, children) = {
                let t = t.borrow();
                (node_kind(&t), text_field(&t, "text"), child_list(&t))
            };
            match kind {
                Some(Kind::Text) => out.push_str(&text),
                Some(Kind::Element | Kind::Document) => {
                    for child in &children {
                        collect_text(child, out, depth + 1)?;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

/// `__html_render(node)`: a node and everything under it as HTML.
fn builtin_render(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let node = match args.first() {
        Some(value @ (Value::Table(_) | Value::String(_))) => value,
        None | Some(Value::Nil) => return Err("node must be provided".to_string()),
        Some(other) => {
            return Err(format!("node must be a table, got {}", other.type_name()));
        }
    };
    let mut out = String::new();
    render(node, &mut out, false, 0)?;
    Ok(vec![Value::string(out)])
}

/// `__html_escape(s)`: `s` with `& < > " '` written as character
/// references, safe inside text and quoted attribute values.
fn builtin_escape(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = str_arg(args, 0, "s")?;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    Ok(vec![Value::string(out)])
}

fn escape_text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}

fn escape_attribute(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}

/// Whether `name` can be written as a tag or attribute name without
/// changing the markup around it.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '<' | '>' | '/' | '=')
        })
}

fn render(value: &Value, out: &mut String, raw: bool, depth: usize) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err(depth_error());
    }
    if out.len() > MAX_OUTPUT {
        return Err(format!(
            "output exceeds the {} MiB limit",
            MAX_OUTPUT / (1024 * 1024)
        ));
    }
    let table = match value {
        Value::String(s) if raw => {
            out.push_str(s);
            return Ok(());
        }
        Value::String(s) => {
            escape_text(out, s);
            return Ok(());
        }
        Value::Table(t) => t.clone(),
        other => return Err(format!("cannot render a {} as HTML", other.type_name())),
    };
    let t = table.borrow();
    match node_kind(&t) {
        Some(Kind::Text) if raw => out.push_str(&text_field(&t, "text")),
        Some(Kind::Text) => escape_text(out, &text_field(&t, "text")),
        Some(Kind::Comment) => {
            out.push_str("<!--");
            out.push_str(&text_field(&t, "text"));
            out.push_str("-->");
        }
        Some(Kind::Doctype) => {
            let name = text_field(&t, "name");
            out.push_str("<!DOCTYPE ");
            out.push_str(if name.is_empty() {
                "html"
            } else {
                name.as_str()
            });
            out.push('>');
        }
        Some(Kind::Document) => {
            for child in &child_list(&t) {
                render(child, out, raw, depth + 1)?;
            }
        }
        Some(Kind::Element) => {
            let tag = text_field(&t, "tag");
            if !valid_name(&tag) {
                return Err(format!("invalid tag name `{}`", tag));
            }
            out.push('<');
            out.push_str(&tag);
            let mut attrs = Vec::new();
            if let Value::Table(a) = t.get_str("attrs") {
                for (key, value) in crate::vm::libs::table::entries(&a.borrow()) {
                    let Value::String(name) = key else {
                        return Err("attribute names must be strings".to_string());
                    };
                    if !valid_name(&name) {
                        return Err(format!("invalid attribute name `{}`", name));
                    }
                    let value = match value {
                        Value::String(s) => Some(s.to_string()),
                        // A true boolean attribute is written bare; false
                        // leaves it out.
                        Value::Bool(true) => None,
                        Value::Bool(false) => continue,
                        Value::Int(_) | Value::BigInt(_) | Value::Float(_) => {
                            Some(value.to_string())
                        }
                        other => {
                            return Err(format!(
                                "attribute `{}` must be a string, got {}",
                                name,
                                other.type_name()
                            ));
                        }
                    };
                    attrs.push((name.to_string(), value));
                }
            }
            attrs.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, value) in attrs {
                out.push(' ');
                out.push_str(&name);
                if let Some(value) = value {
                    out.push_str("=\"");
                    escape_attribute(out, &value);
                    out.push('"');
                }
            }
            out.push('>');
            let lower = tag.to_ascii_lowercase();
            if VOID.contains(&lower.as_str()) {
                return Ok(());
            }
            let raw = RAW_TEXT.contains(&lower.as_str());
            for child in &child_list(&t) {
                render(child, out, raw, depth + 1)?;
            }
            out.push_str("</");
            out.push_str(&tag);
            out.push('>');
        }
        None => return Err("cannot render a table that is not an HTML node".to_string()),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// select
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Combinator {
    /// `a b`
    Descendant,
    /// `a > b`
    Child,
    /// `a + b`
    Adjacent,
    /// `a ~ b`
    Sibling,
}

enum AttrOp {
    /// `[name]`
    Exists,
    /// `[name=value]`
    Equals,
    /// `[name~=value]`: one of the space-separated words.
    Includes,
    /// `[name|=value]`: the value or the value and a hyphen.
    DashMatch,
    /// `[name^=value]`
    Prefix,
    /// `[name$=value]`
    Suffix,
    /// `[name*=value]`
    Contains,
}

enum Condition {
    Id(String),
    Class(String),
    Attr {
        name: String,
        op: AttrOp,
        value: String,
        ignore_case: bool,
    },
    /// `:nth-child(an+b)` and its relatives, counting from the end when
    /// `last` and only elements of the same tag when `of_type`.
    Nth {
        a: i64,
        b: i64,
        last: bool,
        of_type: bool,
    },
    Only {
        of_type: bool,
    },
    Empty,
    Root,
    Not(Vec<Compound>),
}

/// A tag and conditions with nothing between them: `a.nav[href]`.
struct Compound {
    /// None for `*` or no tag.
    tag: Option<String>,
    conditions: Vec<Condition>,
}

/// Compounds joined by combinators. The combinator of each part links it
/// to the part before; the first part's is unused.
struct Complex {
    parts: Vec<(Combinator, Compound)>,
}

struct SelectorParser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

fn parse_selector(text: &str) -> Result<Vec<Complex>, String> {
    let mut parser = SelectorParser {
        text,
        bytes: text.as_bytes(),
        pos: 0,
    };
    let mut list = Vec::new();
    loop {
        parser.skip_whitespace();
        list.push(parser.complex()?);
        parser.skip_whitespace();
        match parser.peek() {
            None => return Ok(list),
            Some(b',') => parser.pos += 1,
            Some(_) => return Err(parser.error("unexpected character")),
        }
    }
}

fn parse_nth(s: &str) -> Option<(i64, i64)> {
    match s {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    match s.split_once('n') {
        Some((a, b)) => {
            let a = match a {
                "" | "+" => 1,
                "-" => -1,
                _ => a.parse().ok()?,
            };
            let b = if b.is_empty() { 0 } else { b.parse().ok()? };
            Some((a, b))
        }
        None => Some((0, s.parse().ok()?)),
    }
}

impl SelectorParser<'_> {
    fn error_at(&self, pos: usize, message: &str) -> String {
        format!(
            "invalid selector `{}` at character {}: {}",
            self.text,
            pos + 1,
            message
        )
    }

    fn error(&self, message: &str) -> String {
        self.error_at(self.pos, message)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')) {
            self.pos += 1;
        }
    }

    /// The character at `pos`, which always sits on a character boundary.
    fn next_char(&self) -> Option<char> {
        self.text
            .get(self.pos..)
            .and_then(|rest| rest.chars().next())
    }

    fn complex(&mut self) -> Result<Complex, String> {
        let mut parts = vec![(Combinator::Descendant, self.compound()?)];
        loop {
            let before = self.pos;
            self.skip_whitespace();
            let combinator = match self.peek() {
                None | Some(b',' | b')') => return Ok(Complex { parts }),
                Some(b'>') => Combinator::Child,
                Some(b'+') => Combinator::Adjacent,
                Some(b'~') => Combinator::Sibling,
                Some(_) if self.pos > before => Combinator::Descendant,
                Some(_) => return Err(self.error("unexpected character")),
            };
            if !matches!(combinator, Combinator::Descendant) {
                self.pos += 1;
                self.skip_whitespace();
            }
            parts.push((combinator, self.compound()?));
        }
    }

    fn compound(&mut self) -> Result<Compound, String> {
        let start = self.pos;
        let tag = if self.eat(b'*') {
            None
        } else if self.ident_start() {
            Some(self.ident()?.to_lowercase())
        } else {
            None
        };
        let mut conditions = Vec::new();
        loop {
            match self.peek() {
                Some(b'#') => {
                    self.pos += 1;
                    conditions.push(Condition::Id(self.ident()?));
                }
                Some(b'.') => {
                    self.pos += 1;
                    conditions.push(Condition::Class(self.ident()?));
                }
                Some(b'[') => conditions.push(self.attribute()?),
                Some(b':') => conditions.push(self.pseudo()?),
                _ => break,
            }
        }
        if self.pos == start {
            return Err(self.error("expected a selector"));
        }
        Ok(Compound { tag, conditions })
    }

    fn ident_start(&self) -> bool {
        matches!(self.peek(), Some(b) if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'\\') || b >= 0x80)
    }

    /// A name. A backslash makes the next character part of it, as in
    /// `.md\:flex`.
    fn ident(&mut self) -> Result<String, String> {
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(b'\\') => {
                    self.pos += 1;
                    let Some(c) = self.next_char() else {
                        return Err(self.error("unfinished escape"));
                    };
                    out.push(c);
                    self.pos += c.len_utf8();
                }
                Some(b) if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' => {
                    out.push(b as char);
                    self.pos += 1;
                }
                Some(b) if b >= 0x80 => {
                    let Some(c) = self.next_char() else {
                        break;
                    };
                    out.push(c);
                    self.pos += c.len_utf8();
                }
                _ => break,
            }
        }
        if out.is_empty() {
            Err(self.error("expected a name"))
        } else {
            Ok(out)
        }
    }

    fn quoted(&mut self, quote: char) -> Result<String, String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.next_char() else {
                return Err(self.error("unfinished string"));
            };
            self.pos += c.len_utf8();
            if c == quote {
                return Ok(out);
            }
            if c == '\\' {
                let Some(escaped) = self.next_char() else {
                    return Err(self.error("unfinished string"));
                };
                self.pos += escaped.len_utf8();
                out.push(escaped);
            } else {
                out.push(c);
            }
        }
    }

    fn attribute(&mut self) -> Result<Condition, String> {
        self.pos += 1;
        self.skip_whitespace();
        let name = self.ident()?.to_lowercase();
        self.skip_whitespace();
        let op = match self.peek() {
            Some(b']') => {
                self.pos += 1;
                return Ok(Condition::Attr {
                    name,
                    op: AttrOp::Exists,
                    value: String::new(),
                    ignore_case: false,
                });
            }
            Some(b'=') => {
                self.pos += 1;
                AttrOp::Equals
            }
            Some(c @ (b'~' | b'|' | b'^' | b'$' | b'*'))
                if self.bytes.get(self.pos + 1) == Some(&b'=') =>
            {
                self.pos += 2;
                match c {
                    b'~' => AttrOp::Includes,
                    b'|' => AttrOp::DashMatch,
                    b'^' => AttrOp::Prefix,
                    b'$' => AttrOp::Suffix,
                    _ => AttrOp::Contains,
                }
            }
            _ => return Err(self.error("expected ']' or an attribute operator")),
        };
        self.skip_whitespace();
        let value = match self.peek() {
            Some(b'"') => self.quoted('"')?,
            Some(b'\'') => self.quoted('\'')?,
            _ => self.ident()?,
        };
        self.skip_whitespace();
        let mut ignore_case = false;
        if let Some(flag @ (b'i' | b'I' | b's' | b'S')) = self.peek() {
            ignore_case = flag.eq_ignore_ascii_case(&b'i');
            self.pos += 1;
            self.skip_whitespace();
        }
        if !self.eat(b']') {
            return Err(self.error("expected ']'"));
        }
        Ok(Condition::Attr {
            name,
            op,
            value,
            ignore_case,
        })
    }

    fn pseudo(&mut self) -> Result<Condition, String> {
        let start = self.pos;
        self.pos += 1;
        let name = self.ident()?.to_lowercase();
        let nth = |a: i64, b: i64, last: bool, of_type: bool| Condition::Nth {
            a,
            b,
            last,
            of_type,
        };
        let condition = match name.as_str() {
            "first-child" => nth(0, 1, false, false),
            "last-child" => nth(0, 1, true, false),
            "only-child" => Condition::Only { of_type: false },
            "first-of-type" => nth(0, 1, false, true),
            "last-of-type" => nth(0, 1, true, true),
            "only-of-type" => Condition::Only { of_type: true },
            "empty" => Condition::Empty,
            "root" => Condition::Root,
            "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                let (a, b) = self.nth_argument()?;
                nth(a, b, name.contains("last"), name.ends_with("of-type"))
            }
            "not" => {
                if !self.eat(b'(') {
                    return Err(self.error("expected '('"));
                }
                let mut list = Vec::new();
                loop {
                    self.skip_whitespace();
                    list.push(self.compound()?);
                    self.skip_whitespace();
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b')') {
                        break;
                    }
                    return Err(self.error("expected ',' or ')'"));
                }
                Condition::Not(list)
            }
            _ => {
                return Err(self.error_at(start, &format!("unsupported pseudo-class `:{}`", name)));
            }
        };
        Ok(condition)
    }

    fn nth_argument(&mut self) -> Result<(i64, i64), String> {
        if !self.eat(b'(') {
            return Err(self.error("expected '('"));
        }
        let start = self.pos;
        while matches!(self.peek(), Some(b) if b != b')') {
            self.pos += 1;
        }
        let inner: String = self
            .text
            .get(start..self.pos)
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_lowercase();
        if !self.eat(b')') {
            return Err(self.error("expected ')'"));
        }
        parse_nth(&inner)
            .ok_or_else(|| self.error_at(start, &format!("invalid argument `{}`", inner)))
    }
}

/// One element of the tree being searched, with what the selectors need
/// to know about its place in it.
struct Elem {
    table: Rc<RefCell<VmTable>>,
    /// Lower case, so tags match whatever case a selector uses.
    tag: String,
    /// Names in lower case.
    attrs: Vec<(String, String)>,
    parent: Option<usize>,
    /// The element sibling right before it.
    prev: Option<usize>,
    /// Its 1-based position among its element siblings, and their number.
    index: usize,
    count: usize,
    /// The same, among siblings with its tag.
    type_index: usize,
    type_count: usize,
    /// No children but comments.
    empty: bool,
    /// Its position in document order.
    order: usize,
}

struct Tree {
    elems: Vec<Elem>,
    next_order: usize,
}

fn nth_matches(a: i64, b: i64, position: usize) -> bool {
    let position = position as i64;
    if a == 0 {
        return position == b;
    }
    let steps = position - b;
    steps % a == 0 && steps / a >= 0
}

impl Tree {
    /// The elements under `root`, and the index of `root` itself when it is
    /// an element.
    fn build(root: &Rc<RefCell<VmTable>>) -> Result<(Tree, Option<usize>), String> {
        let mut tree = Tree {
            elems: Vec::new(),
            next_order: 0,
        };
        let kind = node_kind(&root.borrow());
        match kind {
            Some(Kind::Element) => {
                let id = tree.push(root, None);
                tree.elems[id].order = tree.next_order;
                tree.next_order += 1;
                tree.collect(root, Some(id), 0)?;
                Ok((tree, Some(id)))
            }
            Some(Kind::Document) => {
                tree.collect(root, None, 0)?;
                Ok((tree, None))
            }
            _ => Err("node must be an HTML document or element".to_string()),
        }
    }

    fn push(&mut self, table: &Rc<RefCell<VmTable>>, parent: Option<usize>) -> usize {
        let t = table.borrow();
        let tag = text_field(&t, "tag").to_lowercase();
        let mut attrs = Vec::new();
        if let Value::Table(a) = t.get_str("attrs") {
            for (name, value) in a.borrow().fields.iter() {
                let value = match value {
                    Value::String(s) => s.to_string(),
                    Value::Bool(true) => String::new(),
                    Value::Int(_) | Value::BigInt(_) | Value::Float(_) => value.to_string(),
                    _ => continue,
                };
                attrs.push((name.to_lowercase(), value));
            }
        }
        let empty = child_list(&t).iter().all(|child| match child {
            Value::String(s) => s.is_empty(),
            Value::Table(c) => {
                let c = c.borrow();
                match node_kind(&c) {
                    Some(Kind::Comment) => true,
                    Some(Kind::Text) => text_field(&c, "text").is_empty(),
                    _ => false,
                }
            }
            _ => true,
        });
        self.elems.push(Elem {
            table: table.clone(),
            tag,
            attrs,
            parent,
            prev: None,
            index: 1,
            count: 1,
            type_index: 1,
            type_count: 1,
            empty,
            order: 0,
        });
        self.elems.len() - 1
    }

    /// Adds the element children of `node` and, depth first, theirs.
    fn collect(
        &mut self,
        node: &Rc<RefCell<VmTable>>,
        parent: Option<usize>,
        depth: usize,
    ) -> Result<(), String> {
        if depth >= MAX_DEPTH {
            return Err(depth_error());
        }
        let children = child_list(&node.borrow());
        let mut ids = Vec::new();
        for child in &children {
            if let Value::Table(c) = child
                && node_kind(&c.borrow()) == Some(Kind::Element)
            {
                ids.push(self.push(c, parent));
            }
        }
        let mut type_counts: HashMap<String, usize> = HashMap::new();
        for &id in &ids {
            *type_counts.entry(self.elems[id].tag.clone()).or_default() += 1;
        }
        let mut type_seen: HashMap<String, usize> = HashMap::new();
        for (i, &id) in ids.iter().enumerate() {
            let tag = self.elems[id].tag.clone();
            let seen = type_seen.entry(tag.clone()).or_default();
            *seen += 1;
            let type_index = *seen;
            let elem = &mut self.elems[id];
            elem.prev = i.checked_sub(1).map(|j| ids[j]);
            elem.index = i + 1;
            elem.count = ids.len();
            elem.type_index = type_index;
            elem.type_count = type_counts.get(&tag).copied().unwrap_or(1);
        }
        for &id in &ids {
            self.elems[id].order = self.next_order;
            self.next_order += 1;
            let table = self.elems[id].table.clone();
            self.collect(&table, Some(id), depth + 1)?;
        }
        Ok(())
    }

    fn attr(&self, id: usize, name: &str) -> Option<&str> {
        self.elems[id]
            .attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn matches_compound(&self, compound: &Compound, id: usize) -> bool {
        if let Some(tag) = &compound.tag
            && *tag != self.elems[id].tag
        {
            return false;
        }
        compound
            .conditions
            .iter()
            .all(|condition| self.matches_condition(condition, id))
    }

    fn matches_condition(&self, condition: &Condition, id: usize) -> bool {
        let elem = &self.elems[id];
        match condition {
            Condition::Id(wanted) => self.attr(id, "id") == Some(wanted.as_str()),
            Condition::Class(class) => self
                .attr(id, "class")
                .is_some_and(|classes| classes.split_ascii_whitespace().any(|c| c == class)),
            Condition::Attr {
                name,
                op,
                value,
                ignore_case,
            } => {
                let Some(actual) = self.attr(id, name) else {
                    return false;
                };
                let (actual, value) = if *ignore_case {
                    (actual.to_lowercase(), value.to_lowercase())
                } else {
                    (actual.to_string(), value.clone())
                };
                match op {
                    AttrOp::Exists => true,
                    AttrOp::Equals => actual == value,
                    AttrOp::Includes => actual.split_ascii_whitespace().any(|word| word == value),
                    AttrOp::DashMatch => {
                        actual == value || actual.starts_with(&format!("{}-", value))
                    }
                    AttrOp::Prefix => !value.is_empty() && actual.starts_with(&value),
                    AttrOp::Suffix => !value.is_empty() && actual.ends_with(&value),
                    AttrOp::Contains => !value.is_empty() && actual.contains(&value),
                }
            }
            Condition::Nth {
                a,
                b,
                last,
                of_type,
            } => {
                let (index, count) = if *of_type {
                    (elem.type_index, elem.type_count)
                } else {
                    (elem.index, elem.count)
                };
                let position = if *last { count + 1 - index } else { index };
                nth_matches(*a, *b, position)
            }
            Condition::Only { of_type: true } => elem.type_count == 1,
            Condition::Only { of_type: false } => elem.count == 1,
            Condition::Empty => elem.empty,
            Condition::Root => elem.parent.is_none(),
            Condition::Not(list) => !list
                .iter()
                .any(|compound| self.matches_compound(compound, id)),
        }
    }

    /// Whether `id` matches `complex` up to and including `part`. Results
    /// are remembered, so long chains of descendant combinators over deep
    /// trees stay linear instead of retrying every ancestor combination.
    fn matches(
        &self,
        memo: &mut HashMap<(usize, usize), bool>,
        complex: &Complex,
        part: usize,
        id: usize,
    ) -> bool {
        if let Some(&known) = memo.get(&(part, id)) {
            return known;
        }
        let result = self.matches_uncached(memo, complex, part, id);
        memo.insert((part, id), result);
        result
    }

    fn matches_uncached(
        &self,
        memo: &mut HashMap<(usize, usize), bool>,
        complex: &Complex,
        part: usize,
        id: usize,
    ) -> bool {
        let (combinator, compound) = &complex.parts[part];
        if !self.matches_compound(compound, id) {
            return false;
        }
        if part == 0 {
            return true;
        }
        let elem = &self.elems[id];
        match combinator {
            Combinator::Child => elem
                .parent
                .is_some_and(|parent| self.matches(memo, complex, part - 1, parent)),
            Combinator::Adjacent => elem
                .prev
                .is_some_and(|prev| self.matches(memo, complex, part - 1, prev)),
            Combinator::Descendant => {
                let mut current = elem.parent;
                while let Some(ancestor) = current {
                    if self.matches(memo, complex, part - 1, ancestor) {
                        return true;
                    }
                    current = self.elems[ancestor].parent;
                }
                false
            }
            Combinator::Sibling => {
                let mut current = elem.prev;
                while let Some(sibling) = current {
                    if self.matches(memo, complex, part - 1, sibling) {
                        return true;
                    }
                    current = self.elems[sibling].prev;
                }
                false
            }
        }
    }
}

/// The elements under `node` that match `selector`, in document order.
fn select(node: &Rc<RefCell<VmTable>>, selector: &str, first: bool) -> Result<Vec<Value>, String> {
    let list = parse_selector(selector)?;
    let (tree, scope) = Tree::build(node)?;
    let mut ids: Vec<usize> = (0..tree.elems.len())
        .filter(|id| Some(*id) != scope)
        .collect();
    ids.sort_by_key(|id| tree.elems[*id].order);
    let mut memos: Vec<HashMap<(usize, usize), bool>> =
        list.iter().map(|_| HashMap::new()).collect();
    let mut found = Vec::new();
    for id in ids {
        let hit = list
            .iter()
            .zip(memos.iter_mut())
            .any(|(complex, memo)| tree.matches(memo, complex, complex.parts.len() - 1, id));
        if hit {
            found.push(Value::Table(tree.elems[id].table.clone()));
            if first {
                break;
            }
        }
    }
    Ok(found)
}

/// `__html_select(node, selector, first)`: the matching elements under
/// `node` (not `node` itself) in document order, or with `first` the first
/// of them or nil.
fn builtin_select(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let node = table_arg(args, 0, "node")?.clone();
    let selector = str_arg(args, 1, "selector")?;
    let first = bool_arg(args, 2, "first")?;
    let found = select(&node, selector, first)?;
    if first {
        return Ok(vec![found.into_iter().next().unwrap_or(Value::Nil)]);
    }
    Ok(vec![array(found)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(text: &str) -> Rc<RefCell<VmTable>> {
        match parse(text, false).unwrap() {
            Value::Table(t) => t,
            _ => panic!("parse returns a table"),
        }
    }

    fn tags(found: &[Value]) -> Vec<String> {
        found
            .iter()
            .map(|value| match value {
                Value::Table(t) => {
                    let t = t.borrow();
                    format!("{}#{}", text_field(&t, "tag"), {
                        match t.get_str("attrs") {
                            Value::Table(a) => text_field(&a.borrow(), "id"),
                            _ => String::new(),
                        }
                    })
                }
                _ => String::new(),
            })
            .collect()
    }

    const PAGE: &str = "<!DOCTYPE html><title>T</title><ul id=list><li id=a class='x y'>1<li id=b>2<li id=c class=y>3</ul><p id=p>after";

    #[test]
    fn selectors_match_in_document_order() {
        let doc = document(PAGE);
        let check = |selector: &str, expected: &[&str]| {
            let found = select(&doc, selector, false).unwrap();
            assert_eq!(tags(&found), expected, "{}", selector);
        };
        check("li", &["li#a", "li#b", "li#c"]);
        check(".y", &["li#a", "li#c"]);
        check("ul > li:nth-child(2)", &["li#b"]);
        check("li:not(.x):last-child", &["li#c"]);
        check("li + li", &["li#b", "li#c"]);
        check("#a ~ li", &["li#b", "li#c"]);
        check("ul + p, title", &["title#", "p#p"]);
        check("[class~=x]", &["li#a"]);
        check("html body li:nth-of-type(odd)", &["li#a", "li#c"]);
        check("LI#b", &["li#b"]);
    }

    #[test]
    fn invalid_selectors_are_errors() {
        let doc = document(PAGE);
        for bad in ["", "li >", "li:hover", "[x", "a,,b", ":nth-child(x)"] {
            assert!(select(&doc, bad, false).is_err(), "accepted {:?}", bad);
        }
    }

    #[test]
    fn text_and_render_round_trip() {
        let doc = Value::Table(document(PAGE));
        let mut text = String::new();
        collect_text(&doc, &mut text, 0).unwrap();
        assert_eq!(text, "T123after");
        let mut html = String::new();
        render(&doc, &mut html, false, 0).unwrap();
        assert!(html.starts_with("<!DOCTYPE html><html><head><title>T</title></head><body>"));
        assert!(
            html.contains("<li class=\"x y\" id=\"a\">1</li>"),
            "{}",
            html
        );
    }

    #[test]
    fn fragments_and_deep_nesting() {
        let fragment = parse("<b>x</b> & y", true).unwrap();
        let mut html = String::new();
        render(&fragment, &mut html, false, 0).unwrap();
        assert_eq!(html, "<b>x</b> &amp; y");
        let deep = "<div>".repeat(5000);
        assert!(parse(&deep, false).unwrap_err().contains("nesting"));
    }
}
