# Neyuki
*/neˈjuːki/ — `neh-yoo-kee`. From 根雪, the base layer of snow that settles early and stays all winter.*

Neyuki is a small, Lua-like scripting language implemented in Rust. It is designed to be familiar to anyone who has used Lua or Luau, while keeping the surface area small and the runtime straightforward.

The project currently includes:

- a lexer and parser for `.nyk` source files
- a compiler pipeline for syntax validation
- a small runtime with builtins like `print`, `assert`, `try`, and `require`
- a bundled library example (`@neyuki/math`)
- an HTTP client, server and port forwarder (`@neyuki/http`)
- hashing, password hashing, authenticated encryption and signatures (`@neyuki/crypto`)
- a PostgreSQL and MySQL/MariaDB client with parameterized queries and transactions (`@neyuki/sql`)
- windows, canvas drawing and input events on Linux, macOS and Windows (`@neyuki/ui`)
- a VS Code extension for syntax highlighting in the sibling `editor-extensions/neyuki` folder

## Why Neyuki?

Neyuki aims to feel lightweight and readable without the usual Lua rough edges:

- variables are introduced with `local`, `const`, or `global`
- functions are first-class values
- control flow includes `if`, `elseif`, `else`, `while`, `repeat`, `for`, `break`, and `continue`
- arbitrary-precision integers, floating-point numbers, strings, and tables are the core runtime values
- module-style loading is supported through `require()` for bundled packages

## Project layout

```text
neyuki/
├── Cargo.toml
├── README.md
├── examples/
│   └── hello.nyk
├── lib/
│   └── math.nyk
├── src/
│   ├── compiler.rs
│   ├── lexer.rs
│   ├── lint.rs
│   ├── main.rs
│   ├── parser.rs
│   ├── runtime.rs
│   └── tests.rs
├── tests/
│   ├── control_flow_and_types.nyk
│   ├── number_stress.nyk
│   └── runtime_and_errors.nyk
└── target/
```

## Quick start

From the project root:

```bash
cargo run -- lint examples/hello.nyk
cargo run -- compile examples/hello.nyk
cargo run -- run examples/hello.nyk
cargo run -- test
```

### Example

```lua
const function fibonacci(n: int): int
    if n <= 1 then
        return n
    end

    return fibonacci(n - 1) + fibonacci(n - 2)
end

print(fibonacci(10))
```

Running it:

```bash
cargo run -- run examples/fibonacci.nyk
```

Output:

```text
55
```

## Supported language features

The current compiler/runtime supports a practical subset of a Lua-like language:

- variable declarations with `local` and `const`
- function declarations and calls
- typed variadic parameters (`...`) and vararg expansion in tables and calls
- postfix increment and decrement statements (`++` and `--`)
- arithmetic, comparisons, and boolean logic
- `if` / `elseif` / `else` blocks
- `while` and `repeat` loops
- `for` loops over table values
- strings and string interpolation using `{...}` inside quoted literals
- table literals and indexed access
- builtins: `print`, `tostring`, `type`, `typeof`, `assert`, `int`, `float`, `try`, and `require`

Integer literals and integer arithmetic use arbitrary precision. `type(value)` reports
`"number"` for both integer and floating-point values, while `typeof(value)` reports
`"bigint"` for integers and `"float"` for floating-point values.

## Builtins

A few runtime functions are available in the base environment:

```lua
print("hello")
assert(condition, "message")
tostring(value)
type(value)
typeof(value)
int(3.9)
float("7")
```

The `try` builtin is used for safe calls:

```lua
local ok, err = try(divide, 10, 0)
if not ok then
    print(err)
end
```

## Bundled libraries

Every `.nyk` file in `lib/` is bundled into the executable and exposed as
`@neyuki/<filename>` without its extension. For example:

```lua
local math = require("@neyuki/math")
print(math.random(1, 10))
```

Adding a new library only requires adding its source file to `lib/`.

## Editor support

The repository also includes a VS Code extension under `editor-extensions/neyuki` for `.nyk` syntax highlighting and editor tooling.

The detailed language and runtime references are in [`docs/`](docs/README.md).

## Status

This project is still in active development. The grammar and runtime are intentionally compact, and the test suite under `tests/` is the best way to validate behavior as features evolve.

## Community

Join the Discord server for questions, ideas and design discussion: https://discord.gg/mvCB3KnAT4

Contributions are welcome; see [`CONTRIBUTING.md`](CONTRIBUTING.md). To report a security vulnerability, please follow [`SECURITY.md`](SECURITY.md) instead of opening a public issue.

## License

This project is licensed under the MIT License. See `LICENSE` for details.
