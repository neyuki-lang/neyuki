# @neyuki/toml

Reading and writing TOML, the configuration format of `Cargo.toml` and `pyproject.toml`. `decode` accepts TOML 1.1, which includes every TOML 1.0 document; `encode` writes TOML that both versions read.

```lua
local toml = require("@neyuki/toml")

local config = toml.decode([[
name = "game"

[window]
width = 1280
fullscreen = false
]])
print(config.window.width) --> 1280

config.window.fullscreen = true
print(toml.encode(config))
```

## toml.decode(text: string): table

Parses a TOML document into a table. Tables, inline tables and arrays of tables (`[[name]]`) all become tables; integers stay exact, floats include `inf` and `nan`, and dates and times become RFC 3339 strings such as `"1979-05-27T07:32:00Z"`, `"1979-05-27"` or `"07:32:00"` (`T` and `Z` upper case, seconds always present). Raises an error with the line and column of malformed input, a key defined twice, or an integer outside the 64-bit range.

```lua
local data = toml.decode('when = 2026-11-09\npoint = { x = 1, y = 2 }')
print(data.when, data.point.y) --> 2026-11-09	2
```

## toml.encode(value: table): string

Writes a table as a TOML document. Keys are sorted; plain values come first, then one `[section]` per subtable and one `[[section]]` per entry of an array whose entries are all tables with keys. Other arrays and the tables inside them are written inline. A section that only holds other sections gets no header of its own. Raises an error for an array at the top level, nil inside an array, functions, tables that contain themselves, integers outside the 64-bit range, and output over 64 MiB.

Dates and times decode to strings, so they encode back as strings.

```lua
print(toml.encode({ name = "x", server = { port = 8080 } }))
-- name = "x"
--
-- [server]
-- port = 8080
```
