# os

Time, the environment, command-line arguments and the process.

## os.clock(): number

Seconds as a float, for measuring how long something takes.

```lua
local start = os.clock()
work()
print(("took %.3f s"):format(os.clock() - start))
```

## os.time(t: table?): number

Whole seconds since the Unix epoch (1970-01-01 UTC). Given a table with `year`, `month`, `day` and optionally `hour` (default 12), `min` and `sec`, returns that local time instead. Fields out of range carry over, so `month = 13` is January of the next year.

```lua
local deadline = os.time({ year = 2026, month = 12, day = 24, hour = 18 })
print(os.difftime(deadline, os.time()) // 86400, "days left")
```

## os.date(format: string?, time: number?): string | table

Formats `time` (default: now, in seconds since the epoch) in local time with strftime codes: `%Y` year, `%m` month, `%d` day, `%H` `%M` `%S` time, `%A`/`%a` weekday name, `%B`/`%b` month name, `%j` day of the year, `%Z` time zone and `%%` a percent sign, among others. The default format is `"%c"`. A leading `!` formats in UTC, and `"*t"` (or `"!*t"`) returns a table with `year`, `month`, `day`, `hour`, `min`, `sec`, `wday` (Sunday is 1) and `yday`.

```lua
print(os.date("%Y-%m-%d %H:%M"))          --> 2026-10-13 22:19
print(os.date("!%Y-%m-%dT%H:%M:%SZ", 0))  --> 1970-01-01T00:00:00Z
local today = os.date("*t")
print(today.year, today.month, today.day)
```

## os.difftime(t2: number, t1: number): number

`t2 - t1` in seconds, as a float.

## os.getenv(name: string): string?

The value of an environment variable, or `nil` when it is unset.

```lua
local db = sql.connect(os.getenv("DATABASE_URL") ?? "postgres://localhost/dev")
```

## os.args(): {string}

The command-line arguments that follow the script path.

```lua
-- neyuki greet.nyk Yuki
local name = os.args()[1] ?? "world"
print("hello, " .. name)
```

## os.run(command: string | {string}, options: table?): table

Runs a program, waits for it and returns `{ ok, code, stdout, stderr }`, where `ok` is true for exit code 0. A string runs through the system shell (`cmd /C` on Windows, `sh -c` elsewhere), so pipes and redirection work; a list runs its first element with the rest as arguments and no shell, so nothing needs quoting. `options` may hold `cwd` (the directory to run in), `env` (a table of variables to add or override), `stdin` (text to feed the program) and `capture` (default `true`; `false` lets the output go straight to the terminal). A program that cannot be started raises an error; one that fails just returns `ok = false`.

```lua
local result = os.run({ "git", "log", "--oneline", "-5" })
if result.ok then print(result.stdout) else print("git failed: " .. result.stderr) end

os.run("cargo build --release", { cwd = "../engine", capture = false })
```

## os.sleep(seconds: number): ()

Pauses the script for `seconds` (fractions allowed).

## os.exit(code: number | boolean?): never

Flushes output and ends the process with exit code 0 (nothing or `true`), 1 (`false`) or the given integer.
