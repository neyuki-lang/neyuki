# Runtime and tools

## Commands

Run these commands from the `neyuki/` directory:

```text
cargo run -- lint examples/hello.nyk
cargo run -- compile examples/hello.nyk
cargo run -- run examples/hello.nyk
cargo run -- test
```

`lint` prints tokens and checks parsing. `compile` parses a file and reports the statement count. `run` executes the file. `test` lints every `.nyk` file in `tests/`.

## Builtins

The base runtime provides `print`, `tostring`, `type`, `typeof`, `assert`, `int`, `float`, `try`, and `require`. `type` groups integers and floats as `number`; `typeof` reports `bigint` for arbitrary-precision integers and `float` for floating-point values.

`try(function, ...)` returns a leading boolean followed by the function result or an error message. The bundled `@neyuki/math` and `@neyuki/table` modules can be loaded with `require`.

```lua
local math = require("@neyuki/math")
print(math.max(2, 7, 4))
```

## Current boundaries

The compiler currently validates syntax rather than enforcing the declared type annotations. Runtime modules are resolved from the project checkout, and the base `require` implementation recognizes only the bundled modules listed above. User-defined functions return their explicit `return` values; a function without a return produces `nil` when called.