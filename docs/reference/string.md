# string

Text functions. Every string also has these as methods, so `s:upper()` is `string.upper(s)`. Strings are UTF-8 and indices are byte positions starting at 1; negative indices count from the end (`-1` is the last byte). `#s` and `string.len` count bytes, `string.count` counts characters.

Patterns follow Lua: `.` any character, `%a` letters, `%d` digits, `%s` whitespace, `%w` letters and digits, `%p` punctuation, `%l`/`%u` lower and upper case (capitalized classes such as `%S` negate), `[set]` and `[^set]`, the quantifiers `*`, `+`, `-` (lazy) and `?`, the anchors `^` and `$`, captures `( )`, `%b()` for balanced pairs and `%f[set]` for frontiers. `%` escapes a special character.

## string.len(s: string): number

The length in bytes, the same as `#s`.

## string.count(s: string): number

The number of characters (Unicode code points).

```lua
print(#"héllo", string.count("héllo")) --> 6	5
```

## string.sub(s: string, i: number, j: number?): string

The substring from byte `i` to byte `j` (default: the end), both inclusive.

```lua
print(("neyuki"):sub(1, 3), ("neyuki"):sub(-2)) --> ney	ki
```

## string.upper(s: string): string

`s` in upper case, including non-ASCII letters.

## string.lower(s: string): string

`s` in lower case, including non-ASCII letters.

## string.rep(s: string, n: number, sep: string?): string

`s` repeated `n` times, with `sep` between the copies.

## string.reverse(s: string): string

`s` with its bytes in reverse order.

## string.split(s: string, separator: string?): {string}

Splits `s` at every occurrence of `separator` (plain text, default `","`). An empty separator splits into characters.

```lua
local parts = ("a,b,,c"):split(",") --> { "a", "b", "", "c" }
```

## string.trim(s: string, chars: string?): string

`s` without leading and trailing whitespace, or without any of the characters in `chars`.

## string.ltrim(s: string, chars: string?): string

Like `trim`, but only at the start.

## string.rtrim(s: string, chars: string?): string

Like `trim`, but only at the end.

## string.startswith(s: string, prefix: string): boolean

Whether `s` begins with `prefix`.

## string.endswith(s: string, suffix: string): boolean

Whether `s` ends with `suffix`.

## string.find(s: string, pattern: string, init: number?, plain: boolean?): (number?, number?, ...string)

The start and end of the first match of `pattern` at or after `init`, followed by its captures, or `nil`. With `plain`, `pattern` is plain text.

```lua
print(("hello world"):find("o w")) --> 5	7
```

## string.match(s: string, pattern: string, init: number?): ...string?

The captures of the first match (or the whole match when the pattern has none), or `nil`.

```lua
local key, value = ("name=neyuki"):match("(%w+)=(%w+)")
```

## string.gmatch(s: string, pattern: string): () -> ...string

An iterator over every match, yielding its captures or the whole match.

```lua
for word in ("one two three"):gmatch("%a+") do print(word) end
```

## string.gsub(s: string, pattern: string, replacement: string | table | function, n: number?): (string, number)

`s` with every match (or the first `n`) replaced, and the number of replacements. A string replacement may refer to captures as `%1` to `%9` (`%0` is the whole match); a table is indexed with the first capture; a function is called with the captures. A `nil` or `false` result keeps the match.

```lua
print(("hello world"):gsub("o", "0"))            --> hell0 w0rld	2
print(("$name!"):gsub("%$(%w+)", { name = "Yuki" })) --> Yuki!	1
```

## string.format(format: string, ...: any): string

Builds a string from a C-style template: `%d` and `%i` integers, `%f`, `%e` and `%g` floats, `%x`/`%X` hexadecimal, `%o` octal, `%c` a character code, `%s` any value through `tostring`, `%q` a quoted literal and `%%` a percent sign. Flags, width and precision work as in C (`%-8s`, `%05.2f`).

```lua
print(string.format("%-6s|%5.1f|%x", "id", 3.14159, 255)) --> id    |  3.1|ff
```

## string.byte(s: string, i: number?, j: number?): ...number

The byte values from `i` (default 1) to `j` (default `i`).

## string.char(...: number): string

A string made of the given byte values.

## string.pack(format: string, ...: any): string

Packs values into a binary string, one character per byte. The format uses Lua's codes: `<` `>` `=` set byte order, `b`/`B` 8-bit, `h`/`H` 16-bit, `i[n]`/`I[n]` and `j`/`J` integers, `f`/`d`/`n` floats, `s[n]` length-prefixed strings, `z` zero-terminated strings and `x` padding.

## string.packsize(format: string): number

The size of the string `pack` produces for `format`, which may not contain variable-length options.

## string.unpack(format: string, s: string, init: number?): ...any

Reads the values `pack` wrote, followed by the position after the last byte read.
