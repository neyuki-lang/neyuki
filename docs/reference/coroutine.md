# coroutine

Functions that can pause in the middle and continue later. A coroutine runs until it calls `coroutine.yield`, which hands values back to whoever resumed it; the next `resume` continues right after the `yield`. They are handy for generators, state machines and cooperative tasks.

```lua
local counter = coroutine.wrap(function()
    for i = 1, 3 do coroutine.yield(i) end
end)
print(counter(), counter(), counter()) --> 1	2	3
```

A coroutine can only yield from its own Neyuki frames: yielding from inside a function that a native called back (a `try` body, a `table.sort` comparator, a metamethod) raises an error.

## coroutine.create(f: function): thread

A new suspended coroutine that will run `f`. The handle is frozen, and only it can resume the coroutine.

## coroutine.resume(co: thread, ...: any): (boolean, ...any)

Starts or continues `co`, passing the arguments to `f` (the first time) or as the results of `yield`. Returns `true` and the values passed to `yield` or returned by `f`, or `false` and the error message if it failed.

## coroutine.yield(...: any): ...any

Suspends the running coroutine, handing the arguments to `resume`. Returns the arguments of the next `resume`.

## coroutine.wrap(f: function): (...any) -> ...any

Creates a coroutine and returns a function that resumes it, returning what it yields and raising its errors.

## coroutine.status(co: thread): string

`"suspended"`, `"running"`, `"normal"` (it resumed another coroutine) or `"dead"`.

## coroutine.running(): (thread?, boolean)

The running coroutine (or `nil` in the main program) and whether that is the main program.

## coroutine.isyieldable(): boolean

Whether `yield` would succeed here.

## coroutine.close(co: thread): (boolean, string?)

Kills a suspended or finished coroutine and frees its stack. Returns `true`, or `false` and a message for one that is still running.
