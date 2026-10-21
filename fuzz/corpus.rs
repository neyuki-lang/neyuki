//! Seed inputs, mutations and a sandboxed VM shared by the random fuzzers.

use std::path::Path;

use rand::Rng;
use rand::rngs::StdRng;

use crate::bytecode::proto::Proto;
use crate::compiler::{Options, compile_program};
use crate::vm::machine::{DebugHook, VM};
use crate::vm::value::Value;

/// Small programs covering most of the language, used as seeds when the
/// repository's scripts aren't around and to give every opcode a chance.
pub const SNIPPETS: &[&str] = &[
    "local x = 1 + 2 * 3 - 4 // 2 % 3\nreturn x ^ 2, x / 0, -x, ~x, x << 3 >> 1, x & 6 | 1 ~ 3",
    "local s = \"a\" .. 1 .. 2.5 .. `x{1 + 1}y`\nreturn #s, s:upper(), string.sub(s, 2, -2)",
    "local t = {1, 2, 3, x = 4, [\"y\"] = 5}\nt[#t + 1] = 6\nlocal sum = 0\nfor i, v in t do sum += v end\nreturn sum",
    "local function fib(n) if n < 2 then return n end return fib(n - 1) + fib(n - 2) end\nreturn fib(10)",
    "local function counter()\n  local n = 0\n  return function() n += 1 return n end\nend\nlocal c = counter()\nc() c()\nreturn c()",
    "local function va(...)\n  local n = select(\"#\", ...)\n  return n, ...\nend\nreturn va(1, nil, 3)",
    "local mt = {__add = function(a, b) return a.v + b.v end, __index = function(t, k) return k end}\nlocal a = setmetatable({v = 1}, mt)\nlocal b = setmetatable({v = 2}, mt)\nreturn a + b, a.missing",
    "local co = coroutine.create(function(a)\n  local b = coroutine.yield(a + 1)\n  return b * 2\nend)\nlocal _, x = coroutine.resume(co, 1)\nlocal _, y = coroutine.resume(co, x)\nreturn y, coroutine.status(co)",
    "local ok, err = pcall(function() error({code = 1}) end)\nlocal ok2 = pcall(error)\nreturn ok, err.code, ok2",
    "local i = 0\nwhile true do\n  i += 1\n  if i > 5 then break end\n  if i % 2 == 0 then continue end\nend\nrepeat i -= 1 until i <= 0\nreturn i",
    "for i = 10, 1, -3 do\n  if i == 4 then goto done end\nend\n::done::\nreturn nil ?? 1",
    "local t = {}\nfor i = 1, 20 do t[i] = (i * 7) % 11 end\ntable.sort(t)\nreturn table.concat(t, \",\"), table.unpack(t, 1, 3)",
    "local s = json.encode({a = {1, 2, {b = true}}, c = \"x\\n\"})\nreturn json.decode(s).a[3].b",
    "local b = buffer.create(8)\nbuffer.writeu32(b, 0, 0xDEADBEEF)\nbuffer.writef32(b, 4, 1.5, true)\nreturn buffer.readu16(b, 2), buffer.tohex(b)",
    "local x <const> = 5\nlocal y: number = x * 2\nconst z = y + 1\nreturn z",
    "local s = \"héllo wörld\"\nlocal n = 0\nfor _, c in utf8.codes(s) do n += c end\nreturn utf8.len(s), n, string.format(\"%5.2f|%d|%s|%q|%x\", 1.5, 3, \"a\", \"b\\n\", 255)",
    "local big = 9223372036854775807\nreturn big + 1, big * big, -big - 1, math.maxinteger // -1, 2 ^ 63",
    "local weak = setmetatable({}, {__mode = \"k\"})\ndo local k = {} weak[k] = true end\ncollectgarbage(\"collect\")\nlocal n = 0\nfor _ in weak do n += 1 end\nreturn n",
    "local a, b, c = (function() return 1, 2, 3 end)()\nlocal t = {(function() return 1, 2 end)()}\nreturn a + b + c + #t",
    "local s = \"key=value; other=thing\"\nlocal out = {}\nfor k, v in string.gmatch(s, \"(%w+)=(%w+)\") do out[k] = v end\nreturn out.key, string.gsub(s, \"%s\", \"_\"), string.find(s, \"=\", 1, true)",
];

pub struct Corpus {
    /// Source programs: the snippets above plus the repository's tests,
    /// examples and bundled libraries when they can be found.
    pub sources: Vec<String>,
    /// Compiled bytecode of the sources that compile and use no modules
    /// with side effects.
    pub bytecode: Vec<Vec<u8>>,
}

impl Corpus {
    pub fn load() -> Corpus {
        let mut sources: Vec<String> = SNIPPETS.iter().map(|s| s.to_string()).collect();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for dir in ["tests", "examples", "lib"] {
            let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
                continue;
            };
            let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            paths.sort();
            for path in paths {
                if path.extension().is_some_and(|e| e == "nyk")
                    && let Ok(source) = std::fs::read_to_string(&path)
                {
                    sources.push(source);
                }
            }
        }
        let bytecode = sources
            .iter()
            .filter(|s| !s.contains("require("))
            .filter_map(|s| compile(s).ok())
            .map(|proto| crate::bytecode::serialize(&proto))
            .collect();
        Corpus { sources, bytecode }
    }

    pub fn source(&self, rng: &mut StdRng) -> &str {
        &self.sources[rng.gen_range(0..self.sources.len())]
    }

    pub fn bytecode(&self, rng: &mut StdRng) -> &[u8] {
        &self.bytecode[rng.gen_range(0..self.bytecode.len())]
    }
}

/// Compiles like `neyuki run`, falling back to the unchecked options the
/// bundled libraries are compiled with.
pub fn compile(source: &str) -> Result<Proto, String> {
    compile_program(source, "fuzz", Options::RUN)
        .or_else(|_| compile_program(source, "fuzz", Options::UNCHECKED))
}

/// Modules a fuzzed program may `require`: none of them touch files, the
/// network, other processes, the screen or the keyboard.
const ALLOWED_MODULES: &[&str] = &[
    "bit",
    "bit32",
    "buffer",
    "coroutine",
    "crypto",
    "json",
    "math",
    "string",
    "table",
    "utf8",
];

/// Instructions a sandboxed program may run, in thousands.
const INSTRUCTION_BUDGET_K: u32 = 300;

/// A VM for running programs the fuzzer made up or mangled: no globals with
/// side effects (files, network, processes, windows, input devices, stdout,
/// the scheduler, debug hooks), a `require` limited to `ALLOWED_MODULES`,
/// and a count hook that stops the program after its instruction budget.
pub fn sandbox_vm() -> VM {
    let mut vm = VM::new();
    let names: Vec<String> = vm.globals.keys().map(|k| k.to_string()).collect();
    for name in names {
        let native_with_effects = name.starts_with("__") && !name.starts_with("__crypto_");
        if native_with_effects
            || matches!(name.as_str(), "os" | "task" | "debug" | "print" | "warn")
        {
            vm.globals.remove(name.as_str());
        }
    }
    vm.globals_epoch = vm.globals_epoch.wrapping_add(1);

    let allowed = ALLOWED_MODULES
        .iter()
        .map(|m| format!("[\"{0}\"] = true, [\"@neyuki/{0}\"] = true", m))
        .collect::<Vec<_>>()
        .join(", ");
    let setup = format!(
        "local real = require
        local allowed = {{{allowed}}}
        require = function(name)
            if allowed[name] then return real(name) end
            error(\"not available while fuzzing: \" .. tostring(name))
        end
        print = function(...) end
        warn = print
        local count = 0
        return function()
            count += 1
            if count > {budget} then error(\"fuzz: instruction budget exhausted\") end
        end",
        allowed = allowed,
        budget = INSTRUCTION_BUDGET_K,
    );
    let proto = compile_program(&setup, "fuzz-setup", Options::UNCHECKED)
        .expect("the fuzz sandbox setup compiles");
    let hook = vm.execute(proto).expect("the fuzz sandbox setup runs");
    vm.hook = Some(DebugHook {
        func: hook,
        on_call: false,
        on_return: false,
        on_line: false,
        count: 1000,
    });
    vm.update_trap();
    vm
}

/// Runs `proto` in a fresh sandbox. Errors are expected; only panics count.
pub fn run_sandboxed(proto: Proto) -> Result<Value, String> {
    sandbox_vm().execute(proto)
}

/// Values worth trying wherever a number goes.
pub const INTERESTING_U64: &[u64] = &[
    0,
    1,
    2,
    0x7f,
    0x80,
    0xff,
    0x100,
    0x7fff,
    0x8000,
    0xffff,
    0x7fff_ffff,
    0x8000_0000,
    0xffff_ffff,
    0x7fff_ffff_ffff_ffff,
    0x8000_0000_0000_0000,
    u64::MAX,
    u64::MAX - 1,
];

/// Applies one to four random byte-level mutations, sometimes splicing in
/// part of `other`.
pub fn mutate_bytes(rng: &mut StdRng, data: &mut Vec<u8>, other: &[u8]) {
    for _ in 0..rng.gen_range(1..=4) {
        let len = data.len();
        match rng.gen_range(0..9) {
            0 if len > 0 => {
                let i = rng.gen_range(0..len);
                data[i] ^= 1 << rng.gen_range(0..8);
            }
            1 if len > 0 => {
                let i = rng.gen_range(0..len);
                data[i] = INTERESTING_U64[rng.gen_range(0..INTERESTING_U64.len())] as u8;
            }
            2 if len > 0 => {
                let i = rng.gen_range(0..len);
                data[i] = rng.r#gen();
            }
            3 => {
                let i = rng.gen_range(0..=len);
                let n = rng.gen_range(1..=16);
                let bytes: Vec<u8> = (0..n).map(|_| rng.r#gen()).collect();
                data.splice(i..i, bytes);
            }
            4 if len > 0 => {
                let i = rng.gen_range(0..len);
                let n = rng.gen_range(1..=(len - i).min(32));
                data.drain(i..i + n);
            }
            5 if len > 0 => data.truncate(rng.gen_range(0..len)),
            6 if len >= 4 => {
                // Overwrite a little-endian u16/u32/u64 with an edge value.
                let width = [2usize, 4, 8][rng.gen_range(0..3)].min(len);
                let i = rng.gen_range(0..=len - width);
                let v = INTERESTING_U64[rng.gen_range(0..INTERESTING_U64.len())];
                data[i..i + width].copy_from_slice(&v.to_le_bytes()[..width]);
            }
            7 if len > 0 => {
                // Duplicate a chunk somewhere else.
                let i = rng.gen_range(0..len);
                let n = rng.gen_range(1..=(len - i).min(64));
                let chunk = data[i..i + n].to_vec();
                let at = rng.gen_range(0..=len);
                data.splice(at..at, chunk);
            }
            8 if !other.is_empty() => {
                let i = rng.gen_range(0..other.len());
                let n = rng.gen_range(1..=(other.len() - i).min(128));
                let at = rng.gen_range(0..=len);
                let end = (at + n).min(len);
                data.splice(at..end, other[i..i + n].iter().copied());
            }
            _ => {}
        }
    }
}

/// Fragments that tend to reach unusual parser and lexer states.
const TOKENS: &[&str] = &[
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "[[",
    "]]",
    "[==[",
    "--[[",
    "--",
    "\"",
    "'",
    "`",
    "{",
    "\\",
    "\\u{",
    "\\x",
    "\\z",
    "\n",
    " ",
    ",",
    ";",
    ":",
    "::",
    ".",
    "..",
    "...",
    "=",
    "==",
    "!=",
    "~=",
    "<",
    "<=",
    "<<",
    ">>",
    "+",
    "-",
    "*",
    "/",
    "//",
    "%",
    "^",
    "#",
    "&",
    "|",
    "~",
    "??",
    "+=",
    "..=",
    "->",
    "?",
    "@",
    "$",
    "0",
    "1",
    "-1",
    "0x",
    "0xffffffffffffffffff",
    "1e309",
    "1e-400",
    "0.5",
    ".5",
    "1.",
    "9223372036854775808",
    "nil",
    "true",
    "false",
    "local",
    "const",
    "function",
    "return",
    "end",
    "if",
    "then",
    "elseif",
    "else",
    "while",
    "do",
    "for",
    "in",
    "repeat",
    "until",
    "break",
    "continue",
    "goto",
    "and",
    "or",
    "not",
    "match",
    "enum",
    "type",
    "x",
    "self",
    "_",
    "a.b",
    "a:b()",
    "f()",
    "<const>",
    "<close>",
    ": number",
    ": string?",
    ": {[string]: any}",
    "(...: any) -> any",
    "\u{0}",
    "\u{feff}",
    "é",
    "\u{1F600}",
];

/// Applies one to four token-level mutations to source text, sometimes
/// splicing in part of `other`.
pub fn mutate_text(rng: &mut StdRng, source: &str, other: &str) -> String {
    let mut text: Vec<char> = source.chars().collect();
    let other: Vec<char> = other.chars().collect();
    for _ in 0..rng.gen_range(1..=4) {
        let len = text.len();
        match rng.gen_range(0..7) {
            0 => {
                let at = rng.gen_range(0..=len);
                let token = TOKENS[rng.gen_range(0..TOKENS.len())];
                text.splice(at..at, token.chars());
            }
            1 if len > 0 => {
                let i = rng.gen_range(0..len);
                let n = rng.gen_range(1..=(len - i).min(40));
                text.drain(i..i + n);
            }
            2 if len > 0 => {
                let i = rng.gen_range(0..len);
                let n = rng.gen_range(1..=(len - i).min(80));
                let chunk: Vec<char> = text[i..i + n].to_vec();
                let at = rng.gen_range(0..=len);
                text.splice(at..at, chunk);
            }
            3 if !other.is_empty() => {
                let i = rng.gen_range(0..other.len());
                let n = rng.gen_range(1..=(other.len() - i).min(200));
                let at = rng.gen_range(0..=len);
                let end = (at + rng.gen_range(0..=n)).min(len);
                text.splice(at..end, other[i..i + n].iter().copied());
            }
            4 => {
                // Deep nesting around a random span.
                let depth = rng.gen_range(1..=300);
                let (open, close) = [
                    ("(", ")"),
                    ("{", "}"),
                    ("[", "]"),
                    ("-(", ")"),
                    ("not ", ""),
                ][rng.gen_range(0..5)];
                let at = rng.gen_range(0..=len);
                let end = rng.gen_range(at..=len.min(at + 20));
                text.splice(end..end, close.repeat(depth).chars());
                text.splice(at..at, open.repeat(depth).chars());
            }
            5 if len > 0 => {
                let i = rng.gen_range(0..len);
                text[i] = char::from_u32(rng.gen_range(0..0x80)).unwrap_or('?');
            }
            6 if len > 1 => {
                let a = rng.gen_range(0..len);
                let b = rng.gen_range(0..len);
                text.swap(a, b);
            }
            _ => {}
        }
    }
    text.truncate(64 * 1024);
    text.into_iter().collect()
}
