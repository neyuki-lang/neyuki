# table

Functions for arrays (tables with keys 1, 2, 3, ...) and other tables. `#t` is the length of the array part; `table.length` counts every entry.

## table.insert(t: {any}, value: any): ()

Appends `value` to the end of the array. `table.insert(t, pos, value)` inserts at `pos` and shifts the later elements up.

```lua
local list = { "b" }
table.insert(list, "c")
table.insert(list, 1, "a") --> { "a", "b", "c" }
```

## table.remove(t: {any}, pos: number?): any

Removes and returns the element at `pos` (default: the last one), shifting the later elements down.

## table.concat(t: {any}, sep: string?, i: number?, j: number?): string

Joins the strings and numbers `t[i]` to `t[j]` (default: the whole array) with `sep` between them.

```lua
print(table.concat({ 1, 2, 3 }, ", ")) --> 1, 2, 3
```

## table.sort(t: {any}, comp: ((any, any) -> boolean)?): ()

Sorts the array in place with `<`, or with `comp(a, b)` returning true when `a` must come before `b`. The sort is stable: equal elements keep their order.

```lua
table.sort(people, function(a, b) return a.age < b.age end)
```

## table.find(t: {any}, value: any, init: number?): any

The first index in the array part (from `init`) holding `value`, then any other key holding it, or `nil`.

## table.create(n: number, value: any?): {any}

An array of `n` copies of `value`.

## table.length(t: table): number

The number of non-nil entries, named keys included.

## table.keys(t: table): {any}

A new array of every key: array indices first, then string keys in sorted order, then the rest.

## table.values(t: table): {any}

A new array of every value, in the same order as `keys`.

## table.clone(t: table): table

A shallow copy that shares `t`'s metatable. The copy is never frozen.

## table.clear(t: table): ()

Removes every entry from `t`.

## table.freeze(t: table): table

Makes `t` read-only (writes raise an error) and returns it. Freezing is shallow: tables inside `t` stay writable.

## table.isfrozen(t: table): boolean

Whether `t` is frozen.

## table.pack(...: any): table

An array of the arguments with the field `n` set to their count, `nil`s included.

## table.unpack(t: {any}, i: number?, j: number?): ...any

Returns `t[i]` to `t[j]` (default: the whole array) as separate values.

```lua
local x, y = table.unpack({ 10, 20 })
```

## table.move(a1: {any}, f: number, e: number, t: number, a2: {any}?): {any}

Copies `a1[f..e]` to `a2[t..]` (or within `a1`), handling overlap, and returns the destination.
