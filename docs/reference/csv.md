# @neyuki/csv

Reading and writing CSV (RFC 4180). Quoted fields may hold the delimiter, quotes (written twice) and line breaks; LF, CRLF and CR line endings are all read, a leading byte order mark is dropped and blank lines are skipped. Fields are read as strings; use `tonumber` for numbers.

```lua
local csv = require("@neyuki/csv")
local fs = require("@neyuki/fs")

for _, row in csv.decode(fs.read("scores.csv"), { header = true }) do
    print(row.name, tonumber(row.score))
end

fs.write("out.csv", csv.encode({ { name = "Ada", score = 9 } }))
```

## csv.decode(text: string, options: table?): table

Parses CSV text into an array of records. By default each record is an array of strings. `options.delimiter` is the field separator, one ASCII character other than a quote or a line break (default `","`). With `options.header`, the first record names the columns and every later record becomes a table keyed by those names; a record with fewer fields leaves the missing keys nil. Raises an error with the line of an unclosed quote, text after a closing quote, a column named twice in the header, or a record with more fields than the header.

```lua
local rows = csv.decode('id;label\n1;"a; b"', { delimiter = ";" })
print(rows[2][2]) --> a; b
```

## csv.encode(rows: table, options: table?): string

Writes an array of records as CSV text, one line per record. A record is an array of fields or a table keyed by column name. Fields may be strings, numbers, booleans or nil (an empty field); a field holding the delimiter, a quote or a line break is quoted. For keyed records, `options.columns` gives the column names in order, defaulting to every key of every keyed record, sorted. A header line naming the columns comes first unless `options.header` is false. `options.delimiter` defaults to `","` and `options.newline` is `"\n"` (default) or `"\r\n"`.

```lua
print(csv.encode({ { name = "Ada", age = 36 } }, { columns = { "name", "age" } }))
-- name,age
-- Ada,36
```
