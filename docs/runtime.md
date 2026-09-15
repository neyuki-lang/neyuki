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

`try(function, ...)` returns a leading boolean followed by the function result or an error message. The bundled `@neyuki/fs`, `@neyuki/math`, `@neyuki/string` and `@neyuki/table` modules can be loaded with `require`.

```lua
local math = require("@neyuki/math")
print(math.max(2, 7, 4))
```

`@neyuki/string` follows Lua's string library: `byte`, `char`, `count` (codepoints; `#s` counts bytes), `find`, `format`, `gmatch`, `gsub`, `len`, `lower`, `match`, `pack`, `packsize`, `rep`, `reverse`, `split`, `sub`, `unpack` and `upper`. Indices are byte positions and patterns use Lua pattern syntax. `pack` returns one character per byte (codes 0-255) so binary data fits in a string; `unpack` expects the same encoding.

```lua
local string = require("@neyuki/string")
for key, value in string.gmatch("a=1, b=2", "(%w+)=(%w+)") do
    print(string.format("%s -> %d", key, value))
end
```

`@neyuki/fs` exposes the filesystem: `open(path, mode?)` returns a `File` (modes `"r"`, `"w"` and `"a"`; the default is `"r"` and a missing file is an error), plus `exists`, `remove` (files and empty directories), `rename`, `mkdir(path, recursive?)`, `list` (sorted entry names) and `isDir`. A `File` has a read-only `path` and the methods `read()`, `write(text, append?)` (replaces the contents unless `append` is true; files opened with `"a"` always append), `lines()` (an iterator yielding one line at a time) and `close()`.

```lua
local fs = require("@neyuki/fs")
local file = fs.open("notes.txt", "w")
file:write("one
two
")
file:close()
for line in fs.open("notes.txt"):lines() do
    print(line)
end
```

## Current boundaries

The compiler currently validates syntax rather than enforcing the declared type annotations. Runtime modules are resolved from the project checkout, and the base `require` implementation recognizes only the bundled modules listed above. User-defined functions return their explicit `return` values; a function without a return produces `nil` when called.