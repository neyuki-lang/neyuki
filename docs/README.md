# Neyuki documentation

Neyuki is a small, Lua-like scripting language implemented in Rust. The language reference is in [language.md](language.md), and the current runtime and command-line behavior is in [runtime.md](runtime.md).

The examples in these documents are `.nyk` files. Run them from the `neyuki/` project directory with:

```text
cargo run -- run path/to/file.nyk
```

The executable is intentionally compact. Syntax parsing is shared by `compile`, `lint`, `run`, and the test command, while runtime behavior is exercised by `run`.