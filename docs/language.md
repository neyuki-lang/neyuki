# Neyuki language reference

## Declarations

Use `local`, `const`, or `global` to introduce a binding. Type annotations are optional and are currently retained as syntax metadata.

```lua
local count: int = 0
const greeting = "hello"
global shared = nil
```

Multiple local bindings are supported:

```lua
local ok, value = try(int, "12")
```

## Values and tables

Neyuki supports `nil`, booleans, arbitrary-precision integers, floating-point numbers, strings, functions, and tables. Array entries are one-based; named entries use `key = value`.

Integer literals may be decimal, hexadecimal (`0x...`), or binary (`0b...`), and may contain `_` separators. Integer arithmetic remains exact; `/` produces a floating-point result, while `//`, `%`, and bitwise operators preserve integer values.

```lua
local values = {10, 20, 30}
local config = {name = "neyuki", enabled = true}
values[2] = 25
print(config.name)
```

## Functions and varargs

Functions may have typed parameters and return annotations. A final `...` parameter collects every argument after the required parameters. Its optional type annotation describes each collected value.

```lua
function maximum(first: number, ...: number): number
    local values = {first, ...}
    local result = values[1]
    for _, value in values do
        if value > result then
            result = value
        end
    end
    return result
end

print(maximum(3, 8, 5))
```

Inside a variadic function, `...` can be used as a table entry or as a call argument list. It is an error to evaluate it outside a variadic function.

## Operators

Arithmetic operators are `+`, `-`, `*`, `/`, `//`, `%`, and `^`. Comparisons are `==`, `!=`, `<`, `<=`, `>`, and `>=`. Boolean operators are `and`, `or`, and `not`; `??` selects its right side only when the left side is `nil`. `..` concatenates strings and `#` returns string or array length.

Bitwise operators include `&`, `|`, `~`, `<<`, and `>>`.

Increment and decrement are statement operators for assignable targets:

```lua
count++
values[1]--
```

They add or subtract one and do not produce an expression value. `--` at the start of a line or after whitespace remains a line comment.

## Control flow

Neyuki supports `if` / `elseif` / `else`, `while`, `repeat ... until`, and table iteration with `for`:

```lua
for index, value in values do
    print(index, value)
end
```

Conditions must evaluate to booleans. `break` and `continue` control loops.

## Strings

Single-quoted, double-quoted, and long-bracket strings are supported. Backtick-style interpolation evaluates expressions inside braces:

```lua
print(`value: {count}`)
```