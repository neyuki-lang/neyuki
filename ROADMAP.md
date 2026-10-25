# Neyuki Development Roadmap

This document outlines the architectural milestones and engineering trajectory for the Neyuki language, compiler, runtime, and ecosystem tools towards the 1.0 release.

---

## Milestone 1: Runtime Robustness and Type Precision (v0.1.1 - Current)
- [x] Raise VM recursion ceiling from hardcoded 512 frames to 16,384 frames (`DEFAULT_MAX_CALL_DEPTH`), configurable via `NEYUKI_MAX_CALL_DEPTH`.
- [x] Full tokenizer and recursive descent parser rewrite for `TypeExpr` supporting complex nested records (e.g. `{f: (int, int) -> int}`), function signatures, unions, and optional types.
- [x] Runtime type enforcement engine (`Value::matches_type_expr`) with global builtins `checktype` and `asserttype`.
- [x] Modernized compiler and runtime diagnostic formatting conforming to compiler industry standards (error codes `E0001` - `E0201`, caret underlines, gutter line numbers, source snippets, and structured stack backtraces).
- [x] Formal Language Specification (`docs/spec.md`) and Bytecode Version 4 specification (`docs/bytecode_v4.md`).
- [x] User module documentation (`docs/modules.md`) and Semantic Versioning compatibility policy (`docs/compatibility.md`).

---

## Milestone 2: Algebraic Data Modeling and Pattern Matching (v0.2.0)
- [ ] Algebraic Data Types (`enum` / sum types):
  - [ ] Lexer and parser support for `enum <Name> ... end` declarations with payload-bearing variants.
  - [ ] Runtime representation of enum instances with immutable tags and constructor functions.
- [ ] Pattern matching (`match` construct):
  - [ ] Pattern grammar: literal matching, variant destructuring, wildcard `_`, variable binding.
  - [ ] Exhaustiveness and reachability analysis in semantic checker.
  - [ ] Bytecode lowering to efficient jump tables and conditional branch cascades.

---

## Milestone 3: Developer Tooling Completeness (v0.3.0)
- [ ] Code Formatter (`neyuki fmt`):
  - [ ] AST and token-based code formatter preserving indentation, operators, and doc comments.
  - [ ] CLI flags: `--check` for CI verification and `--write` for in-place source formatting.
- [ ] Language Server Protocol (`neyuki lsp`):
  - [ ] Stdio JSON-RPC 2.0 language server implementation.
  - [ ] Diagnostics push on document open/edit.
  - [ ] Hover documentation for variables, functions, and standard libraries.
  - [ ] Goto definition and document symbols.

---

## Milestone 4: Self-Hosted Compiler and Bootstrap (v0.4.0)
- [ ] Compiler written in Neyuki (`bootstrap/compiler.nyk`):
  - [ ] Lexer and parser written in pure Neyuki leveraging deep recursion support.
  - [ ] AST representation using Neyuki `enum` / sum types.
  - [ ] IR construction and constant folder.
  - [ ] Bytecode v4 emitter.
- [ ] Two-stage bootstrap verification:
  - [ ] Stage 1: Rust compiler compiles `bootstrap/compiler.nyk` to `compiler.nykb`.
  - [ ] Stage 2: `compiler.nykb` compiles `bootstrap/compiler.nyk` to `compiler_stage2.nykb`.
  - [ ] Binary bit-for-bit equivalence check between Stage 1 and Stage 2 bytecode.

---

## Milestone 5: Version 1.0 Production Freeze (v1.0.0)
- [ ] Language syntax freeze: no further breaking grammar modifications.
- [ ] Bytecode Version 4 freeze: guarantee long-term stability and binary portability across all 1.x releases.
- [ ] Performance benchmarking suite validation against standard scripting workloads.
- [ ] Full test coverage of standard libraries, security sandboxing, and memory leak regression tests.

---

## Desktop app platform
- [x] Per-user storage in `@neyuki/os`: `dataDir`, `configDir` and `cacheDir` following each platform's conventions, and `protect`/`unprotect` for secrets (DPAPI on Windows, an OS keychain key elsewhere).
- [x] `@neyuki/ui` window chrome and input: frameless windows with app-drawn title bars (`startDrag`, `startResize`, minimize/maximize/restore, fullscreen, minimum size, icon, drop shadow), pointer cursors, IME text input (`textinput` and `ime` events), click counts for double-click, window state, file drop and DPI change events, `ui.clipboard` and `ui.openUrl`.
