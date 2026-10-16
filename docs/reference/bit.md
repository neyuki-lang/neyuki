# bit

Bitwise operations on 32-bit words, compatible with Lua's `bit32`. Arguments are reduced to their low 32 bits, so negative numbers act as their two's-complement words, and results are unsigned unless noted. For 64-bit and arbitrary-precision integers use the operators `&`, `|`, `~`, `<<`, `>>`, `<<<` and `>>>` instead.

## bit.band(...: number): number

The bitwise AND of all arguments.

## bit.bor(...: number): number

The bitwise OR of all arguments.

## bit.bxor(...: number): number

The bitwise exclusive OR of all arguments.

## bit.bnot(x: number): number

The bitwise NOT of `x`.

## bit.btest(...: number): boolean

Whether the AND of all arguments is non-zero.

```lua
if bit.btest(flags, 0x4) then print("flag 3 is set") end
```

## bit.lshift(x: number, n: number): number

`x` shifted left by `n` bits (modulo 32).

## bit.rshift(x: number, n: number): number

`x` shifted right by `n` bits, filling with zeros.

## bit.arshift(x: number, n: number): number

`x` shifted right by `n` bits, keeping the sign bit. The result is signed.

## bit.rol(x: number, n: number): number

`x` rotated left by `n` bits.

## bit.ror(x: number, n: number): number

`x` rotated right by `n` bits.

## bit.extract(x: number, field: number, width: number?): number

The `width` bits (default 1) of `x` starting at bit `field`, counted from the least significant bit.

```lua
print(bit.extract(0xf0, 4, 4)) --> 15
```

## bit.replace(x: number, value: number, field: number, width: number?): number

`x` with `width` bits (default 1) starting at bit `field` replaced by the low bits of `value`.

## bit.tobit(x: number): number

The low 32 bits of `x` as a signed integer.

## bit.tohex(x: number, n: number?): string

`x` as `n` hexadecimal digits (default 8). A negative `n` gives upper-case digits.

```lua
print(bit.tohex(255), bit.tohex(255, -4)) --> 000000ff	00FF
```
