//! Programs run in the fuzz sandbox: half generated from a small grammar
//! (arithmetic on edge values, tables, closures, metatables, coroutines and
//! library calls), half mutated from real scripts. Runtime errors are fine;
//! panics are not.

use rand::Rng;
use rand::rngs::StdRng;

use super::super::corpus::{Corpus, compile, mutate_text, run_sandboxed};
use super::super::driver::Target;

pub const TARGET: Target = Target {
    name: "vm",
    extension: "nyk",
    default_seconds: 60,
    regressions,
    generate,
    run,
};

fn regressions() {
    use super::vm_fuzz::*;
    fuzz_arithmetic_div_by_zero_and_overflow();
    fuzz_coroutine_lifecycle_and_reentrancy();
    fuzz_metatable_recursion_and_cycles();
    fuzz_table_sort_inconsistent_comparators();
    fuzz_instruction_boundary_and_stack();
    budget_survives_pcall();
}

/// A loop that wraps another in `pcall` used to swallow the budget error
/// every time it fired and run forever.
fn budget_survives_pcall() {
    for source in [
        "while true do pcall(function() while true do end end) end",
        "while true do xpcall(function() while true do end end, print) end",
        "while true do coroutine.resume(coroutine.create(function() while true do end end)) end",
    ] {
        let proto = compile(source).expect("the regression program compiles");
        assert!(run_sandboxed(proto).is_err(), "{source} should run out of budget");
    }
}

fn generate(corpus: &Corpus, rng: &mut StdRng) -> Vec<u8> {
    if rng.gen_bool(0.5) {
        let source = corpus.source(rng).to_string();
        let other = corpus.source(rng).to_string();
        return mutate_text(rng, &source, &other).into_bytes();
    }
    Program::new(rng).generate().into_bytes()
}

fn run(input: &[u8]) {
    let source = String::from_utf8_lossy(input);
    if let Ok(proto) = compile(&source) {
        let _ = run_sandboxed(proto);
    }
}

const INTS: &[&str] = &[
    "0",
    "1",
    "-1",
    "2",
    "3",
    "7",
    "255",
    "256",
    "65536",
    "2147483647",
    "-2147483648",
    "4294967296",
    "9007199254740993",
    "9223372036854775807",
    "-9223372036854775808",
    "9223372036854775808",
    "0x7fffffffffffffff",
    "0xff",
];

const FLOATS: &[&str] = &[
    "0.0",
    "-0.0",
    "0.5",
    "1.5",
    "-2.75",
    "1e308",
    "-1e308",
    "1e-320",
    "math.huge",
    "-math.huge",
    "(0/0)",
    "math.pi",
    "2^53",
    "2^63",
    "2^64",
];

const STRINGS: &[&str] = &[
    "\"\"",
    "\"a\"",
    "\"abc\"",
    "\"10\"",
    "\" 0x1F \"",
    "\"1e5\"",
    "\"-0\"",
    "\"nan\"",
    "\"inf\"",
    "\"héllo\"",
    "\"\\0\"",
    "\"\\u{10FFFF}\"",
    "\"%d%s\"",
    "\"[\"",
    "\"{\\\"a\\\":[1,2]}\"",
    "\"9223372036854775808\"",
];

const BINOPS: &[&str] = &[
    "+", "-", "*", "/", "//", "%", "^", "==", "!=", "<", "<=", ">", ">=", "and", "or", "&", "|",
    "~", "<<", ">>", "??",
];

/// Patterns with bounded backtracking, for the string matching functions.
const PATTERNS: &[&str] = &[
    "\"a\"",
    "\"%d+\"",
    "\"(%w+)=(%w+)\"",
    "\"^%s*(.-)%s*$\"",
    "\"[%a_][%w_]*\"",
    "\"%b()\"",
    "\"%f[%w]%w+\"",
    "\".\"",
    "\"[^,]*\"",
    "\"(a)(b)?\"",
    "\"%\"",
    "\"[\"",
    "\"(()\"",
];

const FORMATS: &[&str] = &[
    "\"%d\"",
    "\"%5.2f\"",
    "\"%s|%s\"",
    "\"%q\"",
    "\"%x\"",
    "\"%X\"",
    "\"%o\"",
    "\"%e\"",
    "\"%g\"",
    "\"%c\"",
    "\"%-10s|\"",
    "\"%+.3d\"",
    "\"%%\"",
    "\"%\"",
    "\"%i\"",
    "\"%a\"",
    "\"%99d\"",
];

/// Library functions of one or two value arguments. Every result is
/// bounded by its arguments, so nothing here can allocate without limit.
const CALLS: &[(&str, usize)] = &[
    ("tostring", 1),
    ("tonumber", 1),
    ("tonumber", 2),
    ("type", 1),
    ("typeof", 1),
    ("int", 1),
    ("float", 1),
    ("rawlen", 1),
    ("rawequal", 2),
    ("rawget", 2),
    ("select", 2),
    ("math.floor", 1),
    ("math.ceil", 1),
    ("math.abs", 1),
    ("math.sqrt", 1),
    ("math.max", 2),
    ("math.min", 2),
    ("math.fmod", 2),
    ("math.modf", 1),
    ("math.frexp", 1),
    ("math.ldexp", 2),
    ("math.round", 1),
    ("math.trunc", 1),
    ("math.sign", 1),
    ("math.log", 2),
    ("math.pow", 2),
    ("math.clamp", 2),
    ("math.isnan", 1),
    ("string.len", 1),
    ("string.upper", 1),
    ("string.lower", 1),
    ("string.reverse", 1),
    ("string.byte", 2),
    ("string.char", 1),
    ("string.sub", 2),
    ("string.trim", 1),
    ("string.split", 2),
    ("string.startswith", 2),
    ("string.count", 2),
    ("table.concat", 2),
    ("table.unpack", 1),
    ("table.insert", 2),
    ("table.remove", 2),
    ("table.find", 2),
    ("table.keys", 1),
    ("table.values", 1),
    ("table.clone", 1),
    ("table.freeze", 1),
    ("table.length", 1),
    ("json.encode", 1),
    ("json.decode", 1),
    ("utf8.char", 1),
    ("utf8.len", 1),
    ("utf8.codepoint", 2),
    ("utf8.offset", 2),
    ("bit.band", 2),
    ("bit.lshift", 2),
    ("bit.arshift", 2),
    ("bit.rol", 2),
    ("bit.tohex", 2),
    ("bit.extract", 2),
    ("coroutine.isyieldable", 0),
    ("coroutine.running", 0),
    ("getmetatable", 1),
];

const METAMETHODS: &[&str] = &[
    "__index",
    "__newindex",
    "__add",
    "__sub",
    "__mul",
    "__div",
    "__mod",
    "__unm",
    "__concat",
    "__eq",
    "__lt",
    "__le",
    "__len",
    "__call",
    "__tostring",
    "__close",
    "__idiv",
    "__band",
];

struct Program<'a> {
    rng: &'a mut StdRng,
    out: String,
    /// Locals in scope, as their names' numbers.
    vars: Vec<usize>,
    next_var: usize,
    /// Local functions in scope, with their parameter counts.
    funcs: Vec<(usize, usize)>,
    next_func: usize,
    /// Nesting of blocks, to bound the program's size.
    depth: usize,
    /// Inside a loop, where statements must not grow values every
    /// iteration.
    in_loop: bool,
}

impl<'a> Program<'a> {
    fn new(rng: &'a mut StdRng) -> Self {
        Program {
            rng,
            out: String::new(),
            vars: Vec::new(),
            next_var: 0,
            funcs: Vec::new(),
            next_func: 0,
            depth: 0,
            in_loop: false,
        }
    }

    fn generate(mut self) -> String {
        let n = self.rng.gen_range(3..16);
        self.block(n);
        let results: Vec<String> = (0..self.rng.gen_range(0..4))
            .map(|_| self.expr(1))
            .collect();
        self.out
            .push_str(&format!("return {}\n", results.join(", ")));
        self.out
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.rng.gen_range(0..items.len())]
    }

    fn var(&mut self) -> Option<String> {
        if self.vars.is_empty() {
            None
        } else {
            let v = self.pick(&self.vars.clone());
            Some(format!("v{}", v))
        }
    }

    fn block(&mut self, statements: usize) {
        let (vars, funcs) = (self.vars.len(), self.funcs.len());
        self.depth += 1;
        for _ in 0..statements {
            self.statement();
        }
        self.depth -= 1;
        self.vars.truncate(vars);
        self.funcs.truncate(funcs);
    }

    fn statement(&mut self) {
        // Most statements run under `pcall`, so one runtime error doesn't
        // end the program.
        let protect = self.rng.gen_bool(0.6);
        if protect {
            self.out.push_str("pcall(function()\n");
        }
        let nested = self.depth < 3;
        match self.rng.gen_range(0..14) {
            0..=3 => {
                let e = self.expr(0);
                let v = self.next_var;
                self.next_var += 1;
                self.out.push_str(&format!("local v{} = {}\n", v, e));
                if !protect {
                    self.vars.push(v);
                }
            }
            4 if !self.in_loop => {
                if let Some(v) = self.var() {
                    let e = self.expr(0);
                    self.out.push_str(&format!("{} = {}\n", v, e));
                }
            }
            5 => {
                if let Some(v) = self.var() {
                    let (k, e) = (self.expr(2), self.expr(1));
                    self.out.push_str(&format!("{}[{}] = {}\n", v, k, e));
                }
            }
            6 if nested => {
                let c = self.expr(1);
                self.out.push_str(&format!("if {} then\n", c));
                let n = self.rng.gen_range(1..4);
                self.block(n);
                self.out.push_str("else\n");
                let n = self.rng.gen_range(0..3);
                self.block(n);
                self.out.push_str("end\n");
            }
            7 if nested => {
                let (a, b) = (self.rng.gen_range(-3..5), self.rng.gen_range(-3..12));
                let step = self.pick(&["", ", 1", ", -1", ", 2", ", 0", ", 0.5"]);
                self.out
                    .push_str(&format!("for i{} = {}, {}{} do\n", self.depth, a, b, step));
                self.loop_body();
            }
            8 if nested => {
                if let Some(v) = self.var() {
                    self.out
                        .push_str(&format!("for k{0}, x{0} in {1} do\n", self.depth, v));
                    self.loop_body();
                }
            }
            9 if nested => {
                let f = self.next_func;
                self.next_func += 1;
                let arity = self.rng.gen_range(0..3);
                let mut params: Vec<String> = (0..arity).map(|i| format!("p{}_{}", f, i)).collect();
                if self.rng.gen_bool(0.3) {
                    params.push("...".to_string());
                }
                self.out
                    .push_str(&format!("local function f{}({})\n", f, params.join(", ")));
                let n = self.rng.gen_range(0..4);
                self.block(n);
                let e = self.expr(1);
                self.out.push_str(&format!("return {}\nend\n", e));
                if !protect {
                    self.funcs.push((f, arity));
                }
            }
            10 => {
                if let Some(v) = self.var() {
                    let mut fields = Vec::new();
                    for _ in 0..self.rng.gen_range(1..4) {
                        let name = self.pick(METAMETHODS);
                        let body = self.expr(2);
                        fields.push(format!("{} = function(a, b) return {} end", name, body));
                    }
                    if self.rng.gen_bool(0.2) {
                        fields.push(format!("__index = {}", v));
                    }
                    self.out
                        .push_str(&format!("setmetatable({}, {{{}}})\n", v, fields.join(", ")));
                }
            }
            11 if nested => {
                let (a, b) = (self.expr(1), self.expr(1));
                self.out.push_str(&format!(
                    "local co = coroutine.wrap(function(a, b)\n\
                     local x = coroutine.yield({})\n\
                     coroutine.yield(x, a)\n\
                     return {}\nend)\nco(1, 2) co(3) co() co()\n",
                    a, b
                ));
            }
            12 => {
                let mode = self.pick(&["\"collect\"", "\"count\"", "\"step\"", ""]);
                self.out.push_str(&format!("collectgarbage({})\n", mode));
            }
            _ => {
                let e = self.expr(0);
                self.out.push_str(&format!("local _ = {}\n", e));
            }
        }
        if protect {
            self.out.push_str("end)\n");
        }
    }

    fn loop_body(&mut self) {
        let was = self.in_loop;
        self.in_loop = true;
        let n = self.rng.gen_range(1..4);
        self.block(n);
        if self.rng.gen_bool(0.2) {
            self.out.push_str("if true then break end\n");
        }
        self.in_loop = was;
        self.out.push_str("end\n");
    }

    fn leaf(&mut self) -> String {
        match self.rng.gen_range(0..10) {
            0..=1 => self.pick(INTS).to_string(),
            2 => self.pick(FLOATS).to_string(),
            3..=4 => self.pick(STRINGS).to_string(),
            5 => self
                .pick(&["nil", "true", "false", "{}", "json.null"])
                .to_string(),
            _ => self.var().unwrap_or_else(|| "nil".to_string()),
        }
    }

    /// A string at most a few dozen bytes long, so concatenation can't
    /// double a value every statement.
    fn short(&mut self, e: String) -> String {
        format!("string.sub(tostring({}), 1, 40)", e)
    }

    fn expr(&mut self, depth: usize) -> String {
        if depth >= 3 || self.rng.gen_bool(0.3) {
            return self.leaf();
        }
        let d = depth + 1;
        match self.rng.gen_range(0..13) {
            0..=2 => {
                let op = self.pick(BINOPS);
                let (a, b) = (self.expr(d), self.expr(d));
                format!("({} {} {})", a, op, b)
            }
            3 => {
                let op = self.pick(&["-", "not ", "#", "~"]);
                format!("({}{})", op, self.expr(d))
            }
            4 => {
                let (a, b) = (self.expr(d), self.expr(d));
                let (a, b) = (self.short(a), self.short(b));
                format!("({} .. {})", a, b)
            }
            5 => {
                let n = self.rng.gen_range(0..4);
                let items: Vec<String> = (0..n)
                    .map(|i| {
                        let e = self.expr(d);
                        if i % 2 == 0 {
                            e
                        } else {
                            format!("k{} = {}", i, e)
                        }
                    })
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            6..=7 => {
                let (name, arity) = self.pick(CALLS);
                let args: Vec<String> = (0..arity).map(|_| self.expr(d)).collect();
                format!("{}({})", name, args.join(", "))
            }
            8 => {
                let s = self.expr(d);
                let s = self.short(s);
                match self.rng.gen_range(0..5) {
                    0 => format!("string.rep({}, {})", s, self.rng.gen_range(-1..20)),
                    1 => {
                        let f = self.pick(FORMATS);
                        format!("string.format({}, {})", f, self.leaf())
                    }
                    2 => format!("string.find({}, {})", s, self.pick(PATTERNS)),
                    3 => format!("string.match({}, {})", s, self.pick(PATTERNS)),
                    _ => format!(
                        "string.gsub({}, {}, {})",
                        s,
                        self.pick(PATTERNS),
                        self.pick(&["\"%0%0\"", "\"x\"", "\"%1\"", "string.upper", "{a = 1}"])
                    ),
                }
            }
            9 => {
                if self.funcs.is_empty() {
                    return self.leaf();
                }
                let (f, arity) = self.pick(&self.funcs.clone());
                let n = arity + self.rng.gen_range(0..2);
                let args: Vec<String> = (0..n).map(|_| self.expr(d)).collect();
                format!("f{}({})", f, args.join(", "))
            }
            10 => {
                let (o, k) = (self.expr(d), self.expr(d));
                format!("({})[{}]", o, k)
            }
            11 => {
                let body = self.expr(d);
                format!(
                    "(function(a, ...) return {}, ... end)({})",
                    body,
                    self.leaf()
                )
            }
            _ => format!("pcall(error, {})", self.expr(d)),
        }
    }
}
