# Neyuki — Language Base Plan (v0.3)

> Status: base plan. Sections marked **[later]** are committed but deferred. Sections marked **[open]** still need a decision.

**Neyuki** (根雪) — the base layer of snow that settles early and stays all winter.

File extension: `.nyk`
Package namespace: `@neyuki/`
Implementation language: Rust

---

## 1. Design Goals

1. **Luau-like, without the papercuts.** Familiar syntax for anyone coming from Lua/Luau.
2. **Real integers.** `int` is a first-class type, not a float in a trenchcoat. Overflow widens to `bigint` instead of silently losing precision.
3. **No implicit globals.** Undeclared names are an error, not a new global.
4. **No truthiness.** Conditions take booleans. `if x then` on a `string?` does not compile.
5. **Always type-checked.** There is no lax mode to fall back into.
6. **Small surface.** One obvious way to do a thing. No aliases, no two names for one function.

---

## 2. Lexical Structure

### Comments

```neyuki
-- short comment
--[[
    long comment
]]
--[=[
    long comment that can contain ]] inside it
]=]
```

Long brackets take any number of `=` signs, matched between opener and closer (`[[ ]]`, `[=[ ]=]`, `[==[ ]==]`, …).

### Semicolons

Optional. A statement ends at a newline unless the line is syntactically incomplete. `;` is allowed as an explicit terminator and to separate multiple statements on one line.

### Identifiers

`[A-Za-z_][A-Za-z0-9_]*`. Case-sensitive. Leading `_` is fine; `_` alone is the conventional throwaway name.

### Keywords

```
and       break     const     continue  do        else      elseif
end       false     for       function  global    if        in
local     nil       not       or        repeat    return    then
true      type      until     while
```

`type` is a **contextual** keyword: it only starts a type alias when it appears at statement position followed by `Identifier =`. Everywhere else it is the ordinary `type()` builtin.

Reserved for future use (not usable as identifiers): `export`, `import`, `try`, `catch`, `throw`, `match`, `async`, `await`, `class`, `enum`, `struct`, `as`.

---

## 3. Types

### Hierarchy

```
any
├── nil
├── number
│   ├── int      -- 64-bit backed, widens automatically
│   ├── bigint   -- arbitrary precision
│   └── float    -- IEEE-754 binary64
├── boolean
├── string
├── table
├── function
└── thread       -- coroutines, see §13
```

`any` is the topmost type. It has no properties of its own — it exists so that "anything goes" can be spelled, and it is the one deliberate escape hatch from the type checker (§3.6).

`unknown` is **not** in the hierarchy. It is a top type you cannot use without narrowing first (§3.4).

### 3.1 `type()` vs `typeof()`

- `type(obj)` returns the **base type**: the nearest ancestor that is a direct child of `any`.
- `typeof(obj)` returns the **exact type** of the value.

```neyuki
print(type(5))        --> "number"
print(typeof(5))      --> "int"
print(type(5.5))      --> "number"
print(typeof(5.5))    --> "float"
print(type("hi"))     --> "string"
print(typeof("hi"))   --> "string"
print(type(2^128))    --> "number"
print(typeof(2^128))  --> "bigint"
```

### 3.2 Annotations

```neyuki
local verybignumber: bigint = 2^128
local name: string = "nyxyl"
```

- `?` marks a type as optional: `string?` is exactly `string | nil`.
- `|` builds a union: `local id: int | string = 42`.
- Unions distribute normally: `(int | string)?` is `int | string | nil`.

> `|` is also bitwise-or in expressions (§9). The parser distinguishes them by position — after a `:` or inside a `type` alias it is a union, otherwise it is an operator. Leave `&` free in type position in case intersection types land later.

### 3.3 Type aliases

```neyuki
type Vec2 = { x: float, y: float }
type Id = int | string
type Handler = (string, int) -> boolean
```

Aliases are block-scoped like `local` and may be exported from a module by returning them (§12).

### 3.4 `unknown` and assertions

A value of type `unknown` cannot be called, indexed, or used in arithmetic until it is narrowed:

```neyuki
local mod = require(userSuppliedPath)  -- unknown

if typeof(mod) == "table" then
    -- mod is table here
end

local cfg = require("./config") :: { debug: boolean }
```

`::` is a **static** assertion — it tells the checker what the type is and inserts a runtime shape check in debug builds only.

### 3.5 Inference

`local x = 5` infers `int`. Annotations are only required where inference cannot reach: uninitialised declarations, function parameters, and module boundaries.

An uninitialised declaration must have an optional type:

```neyuki
local cache: table?   -- ok, starts as nil
local count: int      -- error: non-optional declaration without initialiser
```

### 3.6 Strict mode

The type checker is **always on**. There is no `--!strict` / `--!nonstrict` directive and no per-file opt-out. Every type error is a compile error.

The intended escape hatch is `any`, which is explicit, local to the value that needs it, and greppable — unlike a file-level pragma that quietly disables checking for a thousand lines.

---

## 4. Numbers

### Literals

| Form | Example | Type |
|---|---|---|
| decimal integer | `42`, `1_000_000` | `int` |
| hexadecimal | `0xFF`, `0xdead_beef` | `int` |
| binary | `0b1011` | `int` |
| octal | `0o755` | `int` |
| decimal float | `4.2`, `1e-3`, `.5` | `float` |
| bigint literal | `42n`, `170141183460469231731687303715884105728n` | `bigint` |

Underscores are permitted between digits in any numeric literal and are ignored.

### `int` and `bigint`

`int` is backed by a signed 64-bit integer. When an operation's result leaves the 64-bit range, the value **widens to `bigint` automatically** — no wraparound, no precision loss, no error.

`int` and `bigint` are mutually assignable. `bigint` is best understood as an annotation meaning "this will be large, allocate wide from the start" rather than a separate world:

```neyuki
local a: int = 2^62
local b = a * 8          -- exceeds i64, silently becomes bigint
print(typeof(b))         --> "bigint"

local c: bigint = 7      -- fine, small value in a wide slot
```

`float` never widens. It is always binary64.

### Arithmetic result types

| Operands | `+ - * // %` | `/` | `^` |
|---|---|---|---|
| `int`, `int` | `int` (widens to `bigint` on overflow) | `float` | `int`/`bigint` if exp ≥ 0, else `float` |
| `int`, `bigint` | `bigint` | `float` | `bigint` if exp ≥ 0, else `float` |
| `bigint`, `bigint` | `bigint` | `float` | `bigint` if exp ≥ 0, else `float` |
| anything, `float` | `float` | `float` | `float` |

Rules:

- `/` is **always** true division and always yields `float`. `7 / 2 == 3.5`.
- `//` is floor division and preserves integerness. `7 // 2 == 3`.
- `%` is floor-modulo: the result carries the sign of the divisor (`-7 % 3 == 2`), matching Lua.
- `//` or `%` with an integer zero divisor raises a runtime error. Float division by zero yields `inf`/`nan`.
- `^` is right-associative. `2^3^2 == 2^9`.

### Conversions

```neyuki
int(3.9)         --> 3      (truncates toward zero)
int("42")        --> 42     (errors on malformed input)
float(7)         --> 7.0
bigint(9)        --> 9n
tostring(42)     --> "42"
tonumber("0x1F") --> 31     (returns int?, nil on failure)
```

There are **no implicit numeric conversions except int→bigint widening.** `float` never becomes `int` on its own.

---

## 5. Strings

Immutable UTF-8 byte sequences.

### Quoting

```neyuki
local a = 'single'
local b = "double"
local c = [[
    multi-line, no escape processing
]]
local d = [=[ can contain ]] safely ]=]
```

A newline immediately after the opening `[[` is skipped, so the example above starts at `    multi-line`.

### Escapes

`\n` `\t` `\r` `\\` `\"` `\'` `\0` `\xFF` `\u{1F600}` `\z` (skip following whitespace). Not processed inside long brackets.

### Interpolation

Backtick strings interpolate expressions:

```neyuki
local name = "world"
print(`hello {name}, {2 + 2} things`)
```

`{{` and `}}` are literal braces. Any expression is allowed inside, and the result is passed through `tostring`.

### Length: bytes vs codepoints

**Decision: `#s` is byte length.** It is O(1), it matches Rust's `str` underneath, and it agrees with raw byte indexing. Everything in `@neyuki/string` is codepoint-aware, including `string.length`, `string.sub`, and `string.find`.

So `#"héllo" == 6` but `string.length("héllo") == 5`. That split is genuinely surprising the first time, and the documentation has to say so loudly. Two ways to soften it:

- **(a, recommended)** Keep the split, but name the operator's behaviour in the name: drop `string.length` and call the codepoint count `string.count(s)`. Then "`#` is bytes, `count` is characters" is learnable in one sentence and nothing looks like a duplicate of anything.
- **(b)** Make `#s` count codepoints too, so there is exactly one notion of length. Consistent and beginner-proof, but it turns `#s` into an O(n) scan, which makes `for i = 1, #s` quietly quadratic — a nasty trap in a language that otherwise doesn't have any.

I'd take (a). The cost model stays honest and the naming carries the difference.

### Operators

- `..` concatenates. Both sides must already be `string`; numbers are **not** auto-coerced (use `tostring` or interpolation).
- `#s` gives byte length.

---

## 6. Tables

One aggregate type, three syntactic flavours — arrays, maps, and structs are all `table` at runtime.

```neyuki
local list: {string} = {"a", "b", "c"}
local map: {[string]: int} = { one = 1, two = 2 }
local point: { x: float, y: float } = { x = 1.0, y = 2.0 }
local mixed = {"a", 2, "b"}           -- inferred {int | string}
```

- Array indices start at **1**, as in Lua.
- `t.key` is sugar for `t["key"]`.
- `#t` is the array length (the count of the contiguous 1..n part).
- Reading a missing key yields `nil`. Writing `nil` removes a key.
- Trailing commas are allowed. `,` and `;` both work as separators.

| Syntax | Meaning |
|---|---|
| `{T}` | array of `T` |
| `{[K]: V}` | map from `K` to `V` |
| `{ a: T, b: U }` | struct with known fields |

---

## 7. Declarations & Scoping

### Grammar

```
declaration := attribute* scope? 'function' Name funcbody          -- function sugar
             | attribute* scope? 'function'? Name (':' Type)? '=' expr
             | attribute* scope? Name ':' Type                     -- uninitialised, Type must be optional

attribute   := 'const'
scope       := 'local' | 'global'
```

- **Scope defaults to `local`** when omitted. There is no implicit global; assigning to an undeclared name is a compile error.
- `global` must always be written explicitly.
- The optional `function` marker in the assignment form asserts that the initialiser is a function. It is equivalent to writing a function type annotation and exists purely for readability.

### `local`

Block-scoped, same as Lua. Shadowing is allowed.

### `const`

Binding cannot be reassigned. Requires an initialiser. `const` is **shallow**: a `const` table can still have its contents mutated (use `table.freeze` for deep immutability).

```neyuki
const MAX: int = 100
MAX = 200          -- error: assignment to const
```

### `global`

Accessible anywhere in the **current script**, regardless of block. Globals are *not* shared across modules — each script gets its own global environment. Reading a global before it is assigned yields `nil`.

```neyuki
(function()
    global test = "hello!"
end)()

print(test)        --> hello!
```

### Blocks

`do ... end` introduces a scope. `if`, `while`, `for`, and function bodies each introduce their own.

---

## 8. Functions

### Declaration forms

```neyuki
-- statement sugar
const function greet(text: string)
    print(`hello {text}!`)
end

-- expression assigned to a binding
const global function printTime = function()
    print("the unix time passed is", time.tick())
end

-- anonymous, inline
local double = function(n: int): int return n * 2 end
```

### Signatures

```neyuki
function f(a: int, b: string): boolean        -- single return
function g(): (int, string)                   -- multiple returns
function h(a: int = 1, b: string = "x")       -- default parameters
function sum(...: int): int                   -- variadic
```

- Return type may be omitted and is then inferred. A function with no `return` returns `nil`.
- Parameters are required unless they have a default or an optional type.
- Defaults are evaluated at call time, once per call, left to right, and may reference earlier parameters.
- `...` collects remaining arguments; `{...}` packs them into an array, `#...` counts them.
- Multiple returns expand only in the last position of a call, an assignment, or a table constructor — same rule as Lua. Wrap in parentheses to truncate to one value.

### Methods: `:` and `.`

Both are supported, with exactly Lua's meaning:

```neyuki
function File:read(): string        -- implicit self parameter
    return self._handle:readAll()
end

function File.open(path: string): File   -- no self, a "static"
    ...
end

file:read()          -- passes file as self
File.read(file)      -- identical, self passed explicitly
File.open("a.txt")   -- no self involved
```

`obj:m(a)` is sugar for `obj.m(obj, a)`, evaluating `obj` once. `.` never binds `self`. Calling a `:`-declared function through `.` without passing self is a compile error when the receiver type is known.

### Function types

```neyuki
type Handler = (string, int) -> boolean
type Callback = () -> ()
type Variadic = (...int) -> int
```

---

## 9. Operators

### Table

| Category | Operators |
|---|---|
| Arithmetic | `+` `-` `*` `/` `//` `%` `^` |
| Bitwise | `&` `\|` `~` `<<` `>>` and unary `~` |
| Concatenation | `..` |
| Comparison | `==` `!=` `<` `<=` `>` `>=` |
| Logical | `and` `or` `not` |
| Nil-coalescing | `??` |
| Length | `#` |
| Type assertion | `::` |
| Assignment | `=` `+=` `-=` `*=` `/=` `//=` `%=` `^=` `..=` `??=` `&=` `\|=` `<<=` `>>=` |

### Precedence (loosest to tightest)

```
??
or
and
==  !=  <  <=  >  >=
|                       (bitwise or)
~                       (bitwise xor)
&                       (bitwise and)
<<  >>
..                      (right-associative)
+  -
*  /  //  %
not  #  -  ~  (unary)
^                       (right-associative)
```

### Notes

- **`~=` does not exist.** `!=` is the only inequality operator. There is deliberately no `~=` compound xor-assign either: if `~=` were valid, the Lua reflex `x ~= 1` would silently compile as `x = x ~ 1` instead of failing. Leaving it out means the compiler can catch every `~=` and emit *"`~=` is not an operator — did you mean `!=`?"*, which is worth more than saving four keystrokes on an operation nobody uses.
- `and` / `or` take booleans and return booleans. They are **not** value-selecting like in Lua. Short-circuit evaluation still applies.
- `a ?? b` returns `a` if it is non-nil, otherwise `b`. This covers the `x = y or default` idiom that strict booleans removed.
- `==` compares by value for `nil`, `boolean`, `string`, and `number` (across representations: `1 == 1.0` and `1 == 1n` are both true). Tables and functions compare by identity.
- Comparison of mismatched base types is a compile error, not `false`.

### Bitwise semantics

Bitwise operators accept `int` and `bigint` only. A `float` operand is a compile error — convert with `int(x)` first.

Integers are treated as **infinite-precision two's complement**, the same model Python uses. This is the only model consistent with automatic widening:

- `~x == -x - 1`. Negative numbers behave as if sign-extended forever.
- `<<` never drops bits; it widens to `bigint` as needed. `1 << 100` is a `bigint`.
- `>>` is arithmetic (sign-preserving), exactly equal to `x // 2^n`.
- Shift counts must be non-negative `int`; a negative count is a runtime error.
- The runtime caps shift counts at a configurable limit to stop `1 << 10^12` from trying to allocate a terabyte.

There is no `>>>`. A logical right shift on an unbounded integer has no meaningful definition; if a fixed-width one is ever needed it belongs in a `@neyuki/bit` package with explicit widths.

---

## 10. Control Flow

Every condition must be of type `boolean`. There is no truthiness.

```neyuki
if a > b then
    ...
elseif a == b then
    ...
else
    ...
end

while running do ... end

repeat ... until done

for i = 1, 10 do ... end
for i = 10, 1, -1 do ... end          -- optional step

for i, v in {"a", 2, "b"} do ... end  -- array: index, value
for k, v in someMap do ... end        -- map: key, value
for line in file:lines() do ... end   -- iterator function
```

- The generic `for` accepts arrays, maps, and iterator functions directly. **There is no `pairs`/`ipairs`.**
- Numeric `for` bounds are evaluated once. The loop variable is `int` when all three parts are integers, `float` otherwise.
- `break` exits the innermost loop. `continue` skips to the next iteration.
- `return` must be the last statement in a block (wrap in `do return end` if you need an early exit mid-block).

---

## 11. Errors

```neyuki
error("something broke")              -- raises
error("bad input", 2)                 -- blames the caller's line
assert(x != nil, "x is required")     -- raises if false, returns x otherwise

local ok, result = try(riskyFn, arg1, arg2)
if not ok then
    print(`failed: {result}`)
end

local ok, result = try(riskyFn, arg1, { handler = traceback })
```

### One name, not two

`try(f, ...)` is a protected call: it returns `(true, results...)` or `(false, message)`. It is `pcall` with a name that says what it does.

**Don't ship both `try` and `pcall`.** Two names for one function is the exact kind of thing that makes a stdlib feel bloated: every code review argues about which to use, every tutorial picks a different one, and the docs have to explain that they're the same. It buys zero capability.

The migration problem is real though — Luau muscle memory will type `pcall`. Solve it in the compiler, not the stdlib: `pcall` stays an unbound name, and the checker special-cases it with *"`pcall` is called `try` in this language"*. Each person hits that once and never again, and the stdlib stays one function wide. Same trick as the `~=` → `!=` hint.

`xpcall`'s custom-handler role folds into `try`'s optional trailing options table, so that's a third name avoided too.

### No `try ... catch` statement

The function form covers the need and returns a value you can pattern the rest of your code around. A statement form would add two keywords and a second, differently-shaped error path for no new capability. `try` and `catch` stay reserved (§2) in case this is ever revisited.

---

## 12. Modules & Packages

### Requiring

```neyuki
local io = require("@neyuki/io")      -- built-in package, fully typed
local util = require("./util")     -- relative path, type from util.nyk's return
local dyn = require(somePath)      -- not statically resolvable -> unknown
```

`require` returns `unknown` **only** when the path is not a literal. Literal paths resolve at compile time and carry their real type, so built-in packages need no casts.

- Paths beginning with `@neyuki/` are built-ins.
- Paths beginning with `./` or `../` are relative to the requiring file.
- Each module is evaluated once; the result is cached per process.
- Circular requires are a compile-time error.

### Exporting

A module's exported value is whatever it returns:

```neyuki
-- util.nyk
type Options = { verbose: boolean }

const function run(opts: Options)
    ...
end

return { run = run, Options = Options }
```

Third-party package resolution is **[open]** — see §16.

---

## 13. Concurrency

Luau's model: coroutines underneath, `task` on top for anything with timing. Both ship.

The runtime owns a scheduler that drives the main script and every spawned thread, and runs until no thread is runnable and no timer is pending.

### `@neyuki/coroutine` — the primitive

```neyuki
create(f: function): thread
resume(co: thread, ...: any): (boolean, ...any)
yield(...: any): ...any
wrap(f: function): function          -- resumes and re-raises errors
status(co: thread): string           -- "running" | "suspended" | "normal" | "dead"
running(): thread
isYieldable(): boolean
close(co: thread): (boolean, string?)
```

### `@neyuki/task` — the scheduler

```neyuki
spawn(f: function, ...: any): thread     -- runs immediately, up to its first yield
defer(f: function, ...: any): thread     -- runs at the end of the current cycle
delay(seconds: float, f: function, ...: any): thread
wait(seconds: float = 0): float          -- yields, returns actual elapsed time
cancel(co: thread): ()
```

Notes:

- The main script body runs on a thread, so `task.wait` works at top level.
- `task.wait` **yields** — other threads keep running. `time.sleep` **blocks** the OS thread and stops everything. Reach for `task.wait` by default; `sleep` exists for single-threaded scripts where blocking is what you actually want.
- Function names are camelCase (`coroutine.isYieldable`), diverging from Luau's all-lowercase spelling, to match the rest of the stdlib.

---

## 14. Standard Library

### Base (always in scope, no require)

| Name | Signature | Notes |
|---|---|---|
| `print` | `(...: any) -> ()` | space-separated, newline-terminated |
| `require` | `(path: string) -> unknown` | statically typed for literal paths, §12 |
| `type` | `(obj: any) -> string` | base type |
| `typeof` | `(obj: any) -> string` | exact type |
| `tostring` | `(obj: any) -> string` | |
| `tonumber` | `(s: string, base: int = 10) -> number?` | |
| `int` / `float` / `bigint` | `(v: number \| string) -> …` | conversions, §4 |
| `error` | `(msg: string, level: int = 1) -> ()` | never returns |
| `assert` | `(cond: boolean, msg: string?) -> ()` | |
| `try` | `(f: function, ...: any) -> (boolean, ...any)` | protected call, §11 |

### `@neyuki/io` — console input/output

```neyuki
read(prefix: string?, suffix: string?): string   -- prints prefix, reads a line, prints suffix
write(...: any): ()                              -- no trailing newline
writeLine(...: any): ()
clear(): ()
```

### `@neyuki/fs` — file operations

```neyuki
type File
    path: string          -- read-only
    File:read(): string
    File:write(text: string, append: boolean = false): ()
    File:lines(): () -> string?     -- iterator, one line at a time
    File:close(): ()

open(path: string, mode: string = "r"): File     -- errors if unopenable
exists(path: string): boolean
remove(path: string): ()
rename(from: string, to: string): ()
mkdir(path: string, recursive: boolean = false): ()
list(path: string): {string}
isDir(path: string): boolean
```

### `@neyuki/math`

```neyuki
pi: float      inf: float      nan: float
floor(x: number): int          ceil(x: number): int
abs(x: number): number         sign(x: number): int
sqrt(x: number): float         round(x: number): int
min(...: number): number       max(...: number): number
clamp(x: number, lo: number, hi: number): number
random(): float                randomInt(lo: int, hi: int): int
```

### `@neyuki/string`

Codepoint-aware throughout. Byte-level access lives in `bytes`.

```neyuki
count(s: string): int                     -- codepoints (see §5)
sub(s: string, i: int, j: int?): string
upper(s: string): string                  lower(s: string): string
trim(s: string): string
split(s: string, sep: string): {string}
find(s: string, needle: string): int?
replace(s: string, from: string, to: string): string
startsWith(s: string, p: string): boolean
endsWith(s: string, p: string): boolean
rep(s: string, n: int, sep: string = ""): string
format(fmt: string, ...: any): string
bytes(s: string): {int}                   chars(s: string): {string}
```

### `@neyuki/regex` — pattern matching **[proposed]**

You want the language to be an all-rounder, and pattern matching is the one place where "small" and "powerful" genuinely conflict. The resolution: keep plain-string operations in `@neyuki/string` (most code never needs more than `split`/`find`/`replace`), and put real regex in its own package that nobody pays for unless they import it.

Take **regex, not Lua patterns.** Lua patterns are small and fast, but they're a dialect that exists nowhere else — everyone has to learn them from scratch and they can't express alternation. Regex is knowledge people already have. And Rust's `regex` crate gives you linear-time guarantees and no catastrophic backtracking for free, which is a better engine than most languages ship.

```neyuki
type Match
    text: string
    start: int
    finish: int
    groups: {string}

compile(pattern: string): Regex           -- errors at compile time for literal patterns
Regex:test(s: string): boolean
Regex:find(s: string): Match?
Regex:findAll(s: string): {Match}
Regex:replace(s: string, to: string): string
Regex:split(s: string): {string}
```

Literal patterns passed to `compile` are validated and cached at compile time.

### `@neyuki/time`

```neyuki
tick(): float          -- seconds since the unix epoch, fractional
now(): int             -- whole seconds since the unix epoch
clock(): float         -- monotonic, for measuring durations
sleep(seconds: float): ()   -- blocks; prefer task.wait, §13
date(fmt: string = "%Y-%m-%d %H:%M:%S", t: int?): string
```

### `@neyuki/os`

```neyuki
args: {string}                -- command-line arguments
platform: string              -- "linux" | "windows" | "macos"
env(name: string): string?
setEnv(name: string, value: string): ()
exec(cmd: string): (int, string)   -- exit code, combined output
exit(code: int = 0): ()
```

### `@neyuki/table`

```neyuki
insert(t: {T}, v: T, pos: int?): ()
remove(t: {T}, pos: int?): T?
concat(t: {string}, sep: string = ""): string
sort(t: {T}, cmp: ((T, T) -> boolean)?): ()
find(t: {T}, v: T): int?
keys(t: table): {any}        values(t: table): {any}
clone(t: table, deep: boolean = false): table
freeze(t: table): table      isFrozen(t: table): boolean
count(t: table): int         -- counts all keys, not just the array part
```

---

## 15. Metatables **[later]**

Committed to, deferred past v1. Operator overloading, prototype-style OOP, `__index` fallbacks and custom iteration all hang off this, so it's the single biggest thing the design has to stay compatible with.

Planned surface:

```neyuki
setmetatable(t: table, mt: table?): table
getmetatable(t: table): table?
```

Metamethods to reserve now: `__index`, `__newindex`, `__call`, `__len`, `__eq`, `__lt`, `__le`, `__concat`, `__tostring`, `__iter`, `__add`, `__sub`, `__mul`, `__div`, `__idiv`, `__mod`, `__pow`, `__unm`, `__band`, `__bor`, `__bxor`, `__shl`, `__shr`, `__bnot`, `__metatable`, `__gc`.

**What to build into the Rust runtime now, even with metatables switched off:**

1. Every table value carries an `Option<Rc<Table>>` metatable slot from day one. Adding a field to the table representation later means touching every allocation site and every GC path — do it while there's nothing to break.
2. Index reads/writes, arithmetic, comparison, and `#` go through a single dispatch point each, even if that point currently does nothing but the fast path. Inlining these everywhere and un-inlining them later is the expensive version.
3. `table.freeze` sets a flag that a future `__metatable` / `__newindex` implementation will also need to respect. Design the flag as a small bitfield, not a bool.
4. Decide early whether metamethod lookup is itself metatable-aware (Lua says no — `__index` on a metatable is not consulted when looking up metamethods). Say no; it's faster and less surprising.

Type-checker interaction is the open half: how `__index` affects inferred shapes is a real design problem and is explicitly out of scope until metatables are actually being implemented.

---

## 16. Open Questions

1. **Third-party packages.** `@user/pkg` namespacing vs bare names, where they resolve from, whether there's a manifest/lockfile, and whether `@neyuki/` stays reserved for builtins. Blocks nothing in v1 since builtins resolve statically.
2. **Generics.** `function first<T>(t: {T}): T?` — needed for a non-embarrassing `@neyuki/table`, but real inference is a large chunk of work. Interim option: special-case the handful of generic stdlib signatures in the checker without exposing user-facing type parameters, then open it up in v2.
3. **`#s` naming.** §5 recommends `#` = bytes and `string.count` = codepoints. Confirm, or take option (b) there.
4. **Regex vs Lua patterns.** §14 recommends regex in `@neyuki/regex`. Confirm.
5. **Float formatting.** How `tostring(1.0)` prints — `"1"` or `"1.0"`. Matters more than it sounds, since it's how `int` and `float` are told apart in output.
6. **Integer keys and float keys.** Whether `t[1]` and `t[1.0]` are the same slot. Lua 5.3 normalises integral floats to integers; recommend the same.

---

## 17. Style Conventions

- Types: `PascalCase`. Everything else: `camelCase`.
- Constants may be `SCREAMING_SNAKE_CASE`.
- Indent with 4 spaces.
- Prefer `const` over `local` where the binding never changes.
- Prefer interpolation over `..` chains.
- Prefer `task.wait` over `time.sleep`.
- `_` for intentionally unused bindings.