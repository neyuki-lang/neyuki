# buffer

Fixed-size blocks of mutable bytes for binary data: file formats, network protocols, hashing input. Offsets are 0-based byte positions and every access is bounds-checked. Numbers are little-endian unless the optional last argument `bigEndian` is `true`. Writing an integer that does not fit keeps its low bits.

```lua
local b = buffer.create(8)
buffer.writeu16(b, 0, 0xCAFE, true) -- big-endian
buffer.writef32(b, 2, 1.5)
print(buffer.tohex(b)) --> cafe0000c03f0000
```

## buffer.create(size: number): buffer

A buffer of `size` zero bytes.

## buffer.fromstring(s: string): buffer

A buffer holding the bytes of `s`.

## buffer.tostring(b: buffer): string

The bytes as text. Bytes that are not valid UTF-8 become U+FFFD.

## buffer.fromhex(s: string): buffer

A buffer from hexadecimal text such as `"ff0080"`.

## buffer.tohex(b: buffer): string

The bytes as lower-case hexadecimal text.

## buffer.frombase64(s: string): buffer

A buffer from standard or URL-safe Base64 text.

## buffer.tobase64(b: buffer): string

The bytes as standard Base64 text.

## buffer.len(b: buffer): number

The size in bytes.

## buffer.resize(b: buffer, size: number): ()

Grows `b` with zero bytes or shrinks it to `size`.

## buffer.slice(b: buffer, offset: number?, count: number?): buffer

A new buffer with a copy of `count` bytes from `offset` (default: everything from `offset` on).

## buffer.copy(target: buffer, targetOffset: number, source: buffer, sourceOffset: number?, count: number?): ()

Copies `count` bytes (default: the rest of `source`) from `source` at `sourceOffset` into `target` at `targetOffset`. The two may be the same buffer with overlapping ranges.

## buffer.fill(b: buffer, offset: number, value: number, count: number?): ()

Sets `count` bytes (default: to the end) from `offset` to `value`.

## buffer.equal(a: buffer, b: buffer): boolean

Whether two buffers hold the same bytes.

## buffer.readstring(b: buffer, offset: number, count: number): string

`count` bytes from `offset` as a string.

## buffer.writestring(b: buffer, offset: number, s: string, count: number?): ()

Writes the first `count` bytes of `s` (default: all of them) at `offset`.

## buffer.readi8(b: buffer, offset: number): number

A signed 8-bit integer.

## buffer.readu8(b: buffer, offset: number): number

An unsigned 8-bit integer.

## buffer.readi16(b: buffer, offset: number, bigEndian: boolean?): number

A signed 16-bit integer.

## buffer.readu16(b: buffer, offset: number, bigEndian: boolean?): number

An unsigned 16-bit integer.

## buffer.readi32(b: buffer, offset: number, bigEndian: boolean?): number

A signed 32-bit integer.

## buffer.readu32(b: buffer, offset: number, bigEndian: boolean?): number

An unsigned 32-bit integer.

## buffer.readi64(b: buffer, offset: number, bigEndian: boolean?): number

A signed 64-bit integer.

## buffer.readu64(b: buffer, offset: number, bigEndian: boolean?): number

An unsigned 64-bit integer.

## buffer.readf32(b: buffer, offset: number, bigEndian: boolean?): number

A 32-bit float.

## buffer.readf64(b: buffer, offset: number, bigEndian: boolean?): number

A 64-bit float.

## buffer.writei8(b: buffer, offset: number, value: number): ()

Writes an 8-bit integer.

## buffer.writeu8(b: buffer, offset: number, value: number): ()

Writes an unsigned 8-bit integer.

## buffer.writei16(b: buffer, offset: number, value: number, bigEndian: boolean?): ()

Writes a 16-bit integer.

## buffer.writeu16(b: buffer, offset: number, value: number, bigEndian: boolean?): ()

Writes an unsigned 16-bit integer.

## buffer.writei32(b: buffer, offset: number, value: number, bigEndian: boolean?): ()

Writes a 32-bit integer.

## buffer.writeu32(b: buffer, offset: number, value: number, bigEndian: boolean?): ()

Writes an unsigned 32-bit integer.

## buffer.writei64(b: buffer, offset: number, value: number, bigEndian: boolean?): ()

Writes a 64-bit integer.

## buffer.writeu64(b: buffer, offset: number, value: number, bigEndian: boolean?): ()

Writes an unsigned 64-bit integer.

## buffer.writef32(b: buffer, offset: number, value: number, bigEndian: boolean?): ()

Writes a 32-bit float.

## buffer.writef64(b: buffer, offset: number, value: number, bigEndian: boolean?): ()

Writes a 64-bit float.

## buffer.readbits(b: buffer, bitOffset: number, bitCount: number): number

Up to 32 bits starting at any bit position, least significant bit first.

## buffer.writebits(b: buffer, bitOffset: number, bitCount: number, value: number): ()

Writes the low `bitCount` bits (up to 32) of `value` at any bit position.
