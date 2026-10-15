# Neyuki Technical Documentation

This directory contains the formal specifications and technical reference manuals for the Neyuki programming language, compiler toolchain, and runtime engine.

## Documentation Index

- **[Language Specification](spec.md)**: Formal EBNF grammar, lexical rules, expression precedence, and static type system.
- **[Bytecode v4 Specification](bytecode_v4.md)**: Binary format, header layout, constant pool encoding, prototype structure, and complete opcode instruction reference.
- **[Module System Reference](modules.md)**: Module isolation, export conventions, `require` resolution algorithm, caching semantics, and security rules.
- **[Versioning & Compatibility Policy](compatibility.md)**: Semantic Versioning enforcement, stability tiers, bytecode longevity, and deprecation lifecycle.
- **[Runtime & CLI Guide](runtime.md)**: Virtual machine architecture, coroutine model, garbage collector, CLI options, and diagnostics.
- **[Language Overview](language.md)**: Practical syntax guide and standard library usage examples.

## Verification and Testing

All examples and specifications are continuously validated against the test suite:

```bash
cargo test -- --test-threads=1
```