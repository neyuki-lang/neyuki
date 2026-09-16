# Neyuki language reference

## Declarations

Use `local`, `const`, or `global` to introduce a binding. Type annotations are optional and are currently retained as syntax metadata.

```lua
local count: int = 0
const greeting = "hello"
global shared = nil
```

Const bindings cannot be reassigned. A `const function` also protects the
declared function field, while the containing table remains mutable:

```lua
local math = {}
const function math.exp(n: number): number
    return n
end

math.exp = 2 -- runtime error
```

Multiple local bindings are supported:

```lua
local ok, value = try(int, "12")
```

## Values and tables

Neyuki supports `nil`, booleans, arbitrary-precision integers, floating-point numbers, strings, functions, and tables. Array entries are one-based; named entries use `key = value`, or `["key"] = value` when the key is not a valid name (such as `content-type`).

Integer literals may be decimal, hexadecimal (`0x...`), or binary (`0b...`), and may contain `_` separators. Integer arithmetic remains exact; `/` produces a floating-point result, while `//`, `%`, and bitwise operators preserve integer values.

```lua
local values = {10, 20, 30}
local config = {name = "neyuki", enabled = true}
local headers = {["content-type"] = "text/plain"}
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

`object:name(args)` calls the function stored in `object.name` with `object` passed as the first argument. It is shorthand for `object.name(object, args)` and is how the file objects from `@neyuki/fs` are used.

```lua
local file = fs.open("notes.txt")
print(file:read())
```

## Operators

Arithmetic operators are `+`, `-`, `*`, `/`, `//`, `%`, and `^`. Comparisons are `==`, `!=`, `<`, `<=`, `>`, and `>=`; tables and functions compare by identity, so a table is equal only to itself. Boolean operators are `and`, `or`, and `not`; `??` selects its right side only when the left side is `nil`. `..` concatenates strings and `#` returns string or array length.

Bitwise operators include `&`, `|`, `~`, `<<`, and `>>`. `>>` is an arithmetic shift, so it keeps the sign of negative numbers. The logical shifts `<<<` and `>>>` treat the left operand as an unsigned 64-bit word: bits shifted past the 64th are dropped, zeros are shifted in, and the result is never negative.

```lua
print(-8 >> 1)      -- -4
print(-1 >>> 60)    -- 15
print(1 <<< 63)     -- 9223372036854775808
print(1 <<< 64)     -- 0
```

Increment and decrement are statement operators for assignable targets:

```lua
count++
values[1]--
```

They add or subtract one and do not produce an expression value. `--` at the start of a line or after whitespace remains a line comment.

Compound assignment rewrites `target op= value` as `target = target op value` for `+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=`, `..=`, `<<=`, `>>=`, `&=`, `|=` and `??=`. Several targets can be assigned at once; the right-hand side is evaluated before any target changes, and a call's extra results spread across the remaining targets:

```lua
total += price
a, b = b, a
x, y, z = pair()
```

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