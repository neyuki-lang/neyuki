# debug

Introspection of the running program: stack traces, frame and function information, locals, upvalues and hooks. Levels count outward from the function calling the debug function: level 1 is that function, level 2 its caller, and so on.

## debug.traceback(message: string?, level: number?): string

`message` followed by the call stack, one `file:line in function` entry per frame. Pass it to `xpcall` to keep the stack of an error.

```lua
local ok, err = xpcall(risky, debug.traceback)
```

## debug.getinfo(target: number | function): table?

A table describing the frame at a level or a function: `name`, `what` (`"Neyuki"`, `"main"` or `"native"`), `source`, `short_src`, `linedefined`, `lastlinedefined`, `currentline` (frames only), `numparams`, `isvararg`, `nups` and `func`. Returns `nil` for a level past the stack.

## debug.getlocal(level: number | function, n: number): (string?, any)

The name and value of the `n`th local in scope at a frame; a negative `n` reads the `-n`th extra argument. Given a function, returns the name of its `n`th parameter.

## debug.setlocal(level: number, n: number, value: any): string?

Assigns the `n`th local in scope at a frame and returns its name, or `nil` when there is no such local.

## debug.getupvalue(f: function, n: number): (string?, any)

The name and value of the `n`th variable `f` captures from enclosing functions.

## debug.setupvalue(f: function, n: number, value: any): string?

Assigns `f`'s `n`th captured variable and returns its name.

## debug.upvalueid(f: function, n: number): number

A number identifying the variable behind `f`'s `n`th upvalue, equal for closures that share it.

## debug.getmetatable(value: any): table?

The metatable of `value`, ignoring any `__metatable` field.

## debug.setmetatable(value: any, mt: table?): any

Sets the metatable of `value`, ignoring any `__metatable` field, and returns `value`.

## debug.sethook(hook: function?, mask: string?, count: number?): ()

Calls `hook(event, line)` as the program runs. `mask` holds `c` for calls, `r` for returns and `l` for each new line; with a `count`, the hook also gets a `"count"` event every `count` instructions. `debug.sethook()` removes the hook. Hooks do not trigger hooks.

```lua
debug.sethook(function(event, line) print(event, line) end, "l")
```

## debug.gethook(): (function?, string, number)

The current hook function, its mask and its count.
