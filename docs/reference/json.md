# json

Reading and writing JSON. Tables with only an array part become JSON arrays and every other table an object; integers stay exact and other numbers become floats.

```lua
local text = json.encode({ name = "neyuki", tags = { "small", "fast" } })
print(text) --> {"name":"neyuki","tags":["small","fast"]}
local data = json.decode(text)
print(data.tags[2]) --> fast
```

## json.encode(value: any, options: table?): string

Turns `nil`, booleans, numbers, strings and tables into JSON text. `options.indent` (a number of spaces or a string) pretty-prints the output; `options.sortKeys` (default `true`) sorts object keys. A table whose metatable has `__tojson` is encoded as whatever that function returns. Raises an error for functions, cycles, NaN or infinity, and output over 64 MiB.

```lua
print(json.encode({ x = 1 }, { indent = 2 }))
```

## json.decode(text: string, options: table?): any

Parses strict JSON (RFC 8259). JSON `null` becomes `nil`, so the key disappears from its object; with `options.keepNull` it becomes `json.null` instead and the key stays. Raises an error with the position of malformed input.

## json.array(t: table?): table

Marks `t` (or a new table) so it always encodes as an array, even when empty.

```lua
print(json.encode({ items = json.array() })) --> {"items":[]}
```

## json.null: any

A value that encodes as `null`, and what `decode` produces for `null` with `keepNull`.
