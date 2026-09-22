# Contributing

We accept contributions via pull requests from forks. Fork the repository, make your changes on a branch, and open a pull request against `main`.

## Community

Questions, ideas and design discussion happen on the Discord server: https://discord.gg/mvCB3KnAT4

If you are planning a larger change, please bring it up there or in an issue first so we can agree on the approach before you invest the time.

## Before you open a pull request

CI runs the following on every pull request. Run them locally first so your PR is green on the first push:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

`cargo test` also runs every `.nyk` file under `tests/` and checks formatting, so a failing `cargo fmt` will fail the test suite too.

## Rules

### Code

- **Format with `rustfmt`.** No hand-formatting; if `cargo fmt` changes your code, commit the result.
- **No clippy warnings.** CI runs clippy with `-D warnings`. Fix the warning rather than adding `#[allow(...)]` unless there is a comment explaining why the lint is wrong in that spot.
- **Keep the build free of C toolchains.** Dependencies must build with a plain Rust toolchain on Linux, macOS and Windows (gnu and msvc). Prefer pure-Rust crates; do not add anything that needs OpenSSL, `ring`/`aws-lc`, or a system C compiler. See the comments in `Cargo.toml` for the existing choices.
- **Every new dependency needs a reason.** Add a short comment in `Cargo.toml` next to it explaining what it is for, as the existing entries do.
- **No `unsafe`** unless it is unavoidable, isolated, and documented with a `// SAFETY:` comment.
- **No `unwrap()` / `expect()` on user-controlled input.** Script errors must surface as Neyuki runtime errors, never as a Rust panic.
- **Match the surrounding code.** Follow the naming, comment density and module layout of the file you are editing.

### Language and runtime

- **The VM is the default engine.** New features go into the bytecode compiler and VM (`src/compiler`, `src/bytecode`, `src/vm`). The tree-walking interpreter in `src/runtime.rs` is legacy; only touch it for bug fixes.
- **Standard library changes must cover both implementations.** Native VM libraries live in `src/vm/libs` and `src/vm/builtins`; the bundled `.nyk` sources live in `lib/`. If you add or change a function in one, update the other (or document why it is VM-only).
- **Don't break existing scripts.** Changes to syntax, builtins or library behaviour must keep every file in `tests/` and `examples/` running. If a breaking change is genuinely needed, call it out in the PR description.
- **Optimizer changes must be sound.** Any change under the IR/optimizer must preserve observable behaviour, including error messages and evaluation order. Add a test that would fail without your change.

### Tests and docs

- **Every change ships with tests.** Language and library changes get a `.nyk` file in `tests/` (or additions to an existing one); Rust-level behaviour gets a unit test in `src/tests.rs` or alongside the code.
- **Bug fixes include a regression test** that reproduces the original bug.
- **Update the docs with the code.** Language changes go in `docs/language.md`, runtime/CLI changes in `docs/runtime.md`, new libraries in `README.md`. A feature is not done until it is documented.
- **Tests must be deterministic and self-contained.** No reliance on the network, on a running database, or on wall-clock timing unless the test skips itself cleanly when the resource is absent. Use ports in the 17xxx range for local HTTP tests.

### Commits and pull requests

- **Use conventional commit messages**: `feat(scope): ...`, `fix(scope): ...`, `docs: ...`, `refactor: ...`, `test: ...`, `chore: ...`. Scope is the area touched, e.g. `vm`, `parser`, `http`, `crypto`, `sql`, `ui`.
- **One logical change per PR.** Keep refactors, formatting-only changes and features in separate PRs so they can be reviewed and reverted independently.
- **Describe the why, not just the what.** The PR description should explain the motivation, the approach, and anything a reviewer should look at closely. Link the related issue if there is one.
- **Keep PRs green.** Do not open a PR with known failures; mark it as a draft instead.
- **Rebase, don't merge, when updating a branch.** Keep history linear on top of `main`.

### Reporting issues

- Include the Neyuki version or commit, your OS, and a minimal `.nyk` script that reproduces the problem.
- For crashes, paste the full output including any Rust panic message and backtrace (`RUST_BACKTRACE=1`).
- **Security vulnerabilities are not GitHub issues.** Report them privately as described in [SECURITY.md](SECURITY.md).

## License

By contributing you agree that your contributions are licensed under the MIT license in `LICENSE`.
