# utf8

Character-level text functions. Neyuki strings are always valid UTF-8; these functions count characters (Unicode code points) where `string` counts bytes. Byte positions are 1-based and negative ones count from the end.

## utf8.len(s: string, i: number?, j: number?): number

How many characters start between byte positions `i` (default 1) and `j` (default -1).

## utf8.char(...: number): string

The string made of the given code points.

```lua
print(utf8.char(72, 233)) --> Hé
```

## utf8.codepoint(s: string, i: number?, j: number?): ...number

The code points of the characters that start between byte positions `i` (default 1) and `j` (default `i`).

## utf8.offset(s: string, n: number, i: number?): number?

The byte position where the `n`th character counting from byte `i` starts, or `nil`. A negative `n` counts backwards and `n = 0` finds the start of the character containing byte `i`.

## utf8.sub(s: string, i: number, j: number?): string

The characters from the `i`th to the `j`th (default: the last), counted like `string.sub` counts bytes.

```lua
print(utf8.sub("héllo", 2, 3)) --> él
```

## utf8.codes(s: string): () -> (number, number)

An iterator over the byte position and code point of each character.

```lua
for position, code in utf8.codes("hé") do print(position, code) end
```

## utf8.chars(s: string): () -> string

An iterator over each character as a string.

```lua
for char in utf8.chars("héllo") do print(char) end
```
