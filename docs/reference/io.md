# @neyuki/io

Reading from and writing to the terminal (standard input, output and error) for interactive scripts and command-line filters.

```lua
local io = require("@neyuki/io")

local name = io.prompt("What is your name? ")
io.write("Hello, ", name, "!\n")

-- a filter: number every line piped in
local n = 0
for line in io.lines() do
    n += 1
    io.write(n, ": ", line, "\n")
end
```

## io.read(...: string | number): ...any

Reads from standard input, one value per format: `"l"` a line without its newline (the default), `"L"` a line with its newline, `"a"` everything remaining, `"n"` a whitespace-delimited number, or a number of characters. Each format yields `nil` once input runs out.

```lua
local width, height = io.read("n", "n")
```

## io.lines(format: string | number?): () -> any

An iterator that reads with `format` (default `"l"`) until input runs out.

## io.write(...: string | number): table

Writes the arguments to standard output with no separators or trailing newline, and returns `io.stdout` so calls chain.

## io.flush(): ()

Flushes standard output, so text without a newline appears now.

## io.prompt(text: string?, format: string | number?): any

Writes `text`, flushes and reads one value with `format` (default `"l"`).

## io.stdin: table

Standard input, with the methods `read(...)`, `lines(format?)` and `isTerminal()`.

## io.stdout: table

Standard output, with the methods `write(...)`, `flush()` and `isTerminal()`.

## io.stderr: table

Standard error, with the methods `write(...)`, `flush()` and `isTerminal()`. Use it for messages that should not mix with a filter's output.

```lua
if not io.stdin:isTerminal() then io.stderr:write("reading from a pipe\n") end
```
