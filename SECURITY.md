# Security policy

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues, pull requests, or the Discord server.**

Report them privately through GitHub's private vulnerability reporting:

https://github.com/neyuki-lang/neyuki/security/advisories/new

Include as much of the following as you can:

- the Neyuki version or commit hash
- your operating system and Rust toolchain version
- a minimal `.nyk` script (or Rust snippet) that reproduces the issue
- what an attacker could do with it (crash, memory corruption, data disclosure, code execution, ...)
- any suggested fix, if you have one

We will acknowledge the report as soon as we can, keep you informed while we work on a fix, and credit you in the advisory unless you prefer to stay anonymous. Please give us a reasonable amount of time to release a fix before disclosing publicly.

## What counts as a vulnerability

Neyuki is pre-1.0 and things are still moving quickly, but the following are always treated as security issues:

- memory unsafety (crashes, use-after-free, out-of-bounds reads/writes) in the VM, garbage collector or any native library reachable from a `.nyk` script
- bugs in `@neyuki/crypto` that weaken hashing, password hashing, encryption, signatures or random number generation
- bugs in `@neyuki/sql` that allow parameterized queries to be turned into SQL injection
- bugs in `@neyuki/http` that allow request smuggling, header injection, TLS verification bypass or path traversal in the server
- any way for a script to gain capabilities that the runtime is documented as not granting it

### Out of scope

- **Running untrusted scripts.** Neyuki is not a sandbox. Scripts have access to the filesystem, network and OS through the bundled libraries, so a malicious `.nyk` file can do anything the user running it can do. Do not run scripts you do not trust.
- Denial of service through scripts that intentionally allocate unbounded memory or loop forever.
- Vulnerabilities in third-party dependencies that are already reported upstream. Please report those to the crate's maintainers; a PR bumping the dependency is welcome.

## Supported versions

Only the latest release and the `main` branch receive security fixes.

## Questions

If you are unsure whether something is a security issue, ask privately through the advisory link above rather than in public. For everything else, the Discord server is the place: https://discord.gg/mvCB3KnAT4
