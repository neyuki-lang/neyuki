# @neyuki/crypto

Hashing, password storage, encryption, signatures and secure random values, built on the RustCrypto and dalek crates so keys do not leak through timing and bulk work runs at native speed.

Binary values are strings in an *encoding*: `"hex"`, `"base64"` or `"base64url"`. Digests default to hex and everything else (keys, ciphertexts, signatures, random bytes) to base64. Asymmetric keys are PEM strings (PKCS#8 private, SPKI public) that OpenSSL and other languages read; PKCS#1 `RSA PRIVATE KEY` files are accepted too.

```lua
local crypto = require("@neyuki/crypto")

-- storing and checking a password
local stored = crypto.hashPassword("correct horse battery staple")
assert(crypto.verifyPassword("correct horse battery staple", stored))

-- encrypting a record, bound to its id
local key = crypto.generateKey()
local sealed = crypto.encrypt("card ending 4242", key, { aad = "user:42" })
print(crypto.decrypt(sealed, key, { aad = "user:42" }))
```

## crypto.hash(s: string, algorithm: string?, encoding: string?): string

The digest of `s` with `"sha256"` (default), `"sha224"`, `"sha384"`, `"sha512"`, `"sha3-256"`, `"sha3-512"`, `"blake2b"`, `"blake2s"`, `"sha1"` or `"md5"`.

```lua
print(crypto.hash("hello")) --> 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
```

## crypto.hmac(s: string, key: string, algorithm: string?, encoding: string?): string

A keyed digest (HMAC) of `s`, with the same algorithms as `hash` except BLAKE2.

## crypto.equals(a: string, b: string): boolean

Compares two strings in constant time. Use it for MACs and tokens so the comparison does not reveal how much matched.

## crypto.hashPassword(password: string, options: table?): string

A salted, self-describing password hash. `options.algorithm` is `"argon2id"` (default; `cost`, `memory` in KiB and `parallelism` default to 2, 19456 and 1) or `"bcrypt"` (`cost` defaults to 12; passwords over 72 bytes are refused).

## crypto.verifyPassword(password: string, hash: string): boolean

Whether `password` matches a hash from `hashPassword` (either algorithm).

## crypto.pbkdf2(password: string, salt: string, options: table?): string

Derives a key from a password with PBKDF2. `options` may set `algorithm`, `iterations` (default 600000), `length` in bytes (default 32) and `encoding`.

## crypto.hkdf(key: string, options: table?): string

Derives a key from key material with HKDF. `options` may set `algorithm`, `salt`, `info`, `length` and `encoding` (which also applies to `key`).

## crypto.generateKey(algorithm: string?, options: table?): string

A random key for `"aes-256-gcm"` (default), `"aes-128-gcm"`, `"chacha20-poly1305"` or `"xchacha20-poly1305"`.

## crypto.encrypt(s: string, key: string, options: table?): string

Seals `s` with an authenticated cipher and a fresh random nonce, returning `nonce || ciphertext || tag`. `options` may hold `algorithm`, `aad` (data that is authenticated but not encrypted, which `decrypt` must be given again) and `encoding` (of the key and the result).

## crypto.decrypt(s: string, key: string, options: table?): string

Opens what `encrypt` sealed. Raises an error for a wrong key or tampered data.

## crypto.generateKeyPair(algorithm: string?, options: table?): table

`{ publicKey, privateKey }` for `"ed25519"` (default), `"x25519"` or `"rsa"` (`options.bits` defaults to 2048).

## crypto.publicKey(privateKey: string): string

The public half of a private key.

## crypto.sign(s: string, privateKey: string, options: table?): string

Signs `s` with an Ed25519 or RSA key. For RSA, `options` may set `hash` (default `"sha256"`) and `padding` (`"pss"`, the default, or `"pkcs1"`).

```lua
local keys = crypto.generateKeyPair()
local signature = crypto.sign("release-1.0.zip", keys.privateKey)
assert(crypto.verify("release-1.0.zip", signature, keys.publicKey))
```

## crypto.verify(s: string, signature: string, publicKey: string, options: table?): boolean

Whether `signature` is valid for `s`. Returns `false` for a bad or malformed signature. A private key is accepted in place of the public one.

## crypto.publicEncrypt(s: string, publicKey: string, options: table?): string

Encrypts a short message, such as a symmetric key, with RSA-OAEP.

## crypto.privateDecrypt(s: string, privateKey: string, options: table?): string

Reverses `publicEncrypt`.

## crypto.sharedSecret(privateKey: string, publicKey: string, options: table?): string

X25519 key agreement: both sides compute the same secret from their own private key and the other's public key. Run it through `hkdf` with an `info` before using it as a key.

## crypto.randomBytes(n: number, encoding: string?): string

`n` random bytes from the operating system's secure generator.

## crypto.randomInt(min: number, max: number?): number

A secure random integer from 1 to `min`, or from `min` to `max` (inclusive, like `math.random`).

## crypto.uuid(): string

A random (version 4) UUID such as `"3f2b9c1e-8a4d-4c6b-9e2f-1a7d5b3c8e90"`.

## crypto.encode(s: string, encoding: string): string

Encodes text as hex, Base64 or Base64url.

## crypto.decode(s: string, encoding: string): string

Decodes hex, Base64 or Base64url text. Raises an error if the bytes are not valid UTF-8, so keep binary values such as keys encoded.
