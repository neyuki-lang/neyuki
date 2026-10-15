# math

Numbers, rounding, trigonometry and random values. Integers are exact and unbounded; functions that round (`floor`, `ceil`, `round`, `trunc`) return integers, and functions that compute (`sqrt`, `sin`, `log`, ...) return floats.

## math.pi: number

π, 3.14159...

## math.tau: number

2π, a full turn in radians.

## math.e: number

Euler's number, 2.71828...

## math.phi: number

The golden ratio, 1.61803...

## math.sqrt2: number

The square root of 2.

## math.huge: number

Positive infinity. `-math.huge` is negative infinity.

## math.nan: number

A floating-point NaN, the one value not equal to itself.

## math.abs(n: number): number

The absolute value. Integers stay exact.

## math.sign(n: number): number

`-1`, `0` or `1` by the sign of `n` (NaN stays NaN).

## math.floor(n: number): number

The largest integer not greater than `n`.

## math.ceil(n: number): number

The smallest integer not less than `n`.

## math.round(n: number): number

The nearest integer, with halves rounded away from zero.

## math.trunc(n: number): number

`n` with its fractional part dropped (rounded toward zero).

## math.max(...: number): number

The largest argument. Also accepts a single array: `math.max(scores)`.

## math.min(...: number): number

The smallest argument. Also accepts a single array.

## math.clamp(n: number, min: number, max: number): number

`n` limited to the range `min..max`.

```lua
print(math.clamp(15, 0, 10)) --> 10
```

## math.lerp(a: number, b: number, t: number): number

The value `t` of the way from `a` to `b`: `a + (b - a) * t`.

## math.map(n: number, inMin: number, inMax: number, outMin: number, outMax: number): number

Rescales `n` from the range `inMin..inMax` to `outMin..outMax`.

```lua
print(math.map(5, 0, 10, 0, 100)) --> 50
```

## math.sqrt(n: number): number

The square root. Raises an error for negative `n`.

## math.cbrt(n: number): number

The cube root.

## math.pow(base: number, exponent: number): number

`base` raised to `exponent` as a float. The `^` operator keeps integer powers exact.

## math.exp(n: number): number

e raised to `n`.

## math.log(n: number, base: number?): number

The natural logarithm, or the logarithm in `base`.

## math.log10(n: number): number

The base-10 logarithm.

## math.log2(n: number): number

The base-2 logarithm.

## math.sin(radians: number): number

The sine.

## math.cos(radians: number): number

The cosine.

## math.tan(radians: number): number

The tangent.

## math.asin(n: number): number

The arc sine, in radians.

## math.acos(n: number): number

The arc cosine, in radians.

## math.atan(y: number, x: number?): number

The arc tangent of `y`, or of `y / x` using both signs to find the quadrant.

## math.atan2(y: number, x: number): number

The angle of the point `(x, y)` from the positive x axis, in radians.

## math.sinh(n: number): number

The hyperbolic sine.

## math.cosh(n: number): number

The hyperbolic cosine.

## math.tanh(n: number): number

The hyperbolic tangent.

## math.deg(radians: number): number

Converts radians to degrees.

## math.rad(degrees: number): number

Converts degrees to radians.

## math.fmod(a: number, b: number): number

The remainder of `a / b` with the sign of `a` (unlike `%`, which takes the sign of `b`).

## math.modf(n: number): (number, number)

The integral and fractional parts of `n`, both floats.

## math.frexp(n: number): (number, number)

Splits `n` into a mantissa in `[0.5, 1)` and an exponent so that `n = m * 2^e`.

## math.ldexp(m: number, e: number): number

`m * 2^e`.

## math.isfinite(n: number): boolean

Whether `n` is neither infinite nor NaN.

## math.isinf(n: number): boolean

Whether `n` is positive or negative infinity.

## math.isnan(n: number): boolean

Whether `n` is NaN.

## math.random(m: number?, n: number?): number

With no arguments, a float in `[0, 1)`. With `m`, an integer from 1 to `m`. With `m` and `n`, an integer from `m` to `n`. Bounds may be integers of any size.

```lua
local die = math.random(6)
local roll = math.random(10, 20)
```

## math.randomseed(seed: number?): ()

Seeds the generator so `random` repeats the same sequence; without a seed it goes back to random entropy.

## math.noise(x: number, y: number?, z: number?): number

Perlin noise at a point, a smooth value between about -1 and 1 that changes gradually as the coordinates do. Useful for terrain, textures and natural-looking motion.
