# Contributing to CalcKernel

[简体中文](CONTRIBUTING.zh-CN.md)

Thanks for your interest in CalcKernel. Contributions to the CK language,
compiler, documentation, examples, and benchmarks are welcome.

## Before opening a change

- Search existing issues and pull requests for related work. For a substantial
  language or architecture change, open an issue first so the expected behavior
  can be discussed before implementation.
- Keep changes focused and explain the user-visible reason for them.
- Use the canonical names **CK / CalcKernel**, `.ck` source files, and the
  `ckc` compiler command.
- Do not include credentials, personal data, or machine-specific build outputs
  in commits or issue reports.

## Where things live

- Compiler implementation: `src/frontend/`, `src/ir/`, `src/optimizer/`,
  `src/backend/`, and `src/cli/`.
- Runnable language examples: the matching directory under `examples/`.
- Integration tests: the responsibility-based layout described in
  [`tests/README.md`](tests/README.md); shared helpers belong in `tests/support/`.
- User documentation: `docs/` and its Simplified Chinese mirror at
  `docs/zh-CN/`.
When changing language, CLI, MIR, ABI, compatibility, or release behavior,
update its tests and user-facing documentation. Formal documentation changes
under `docs/` must include matching English and Simplified Chinese files at the
same relative paths.

## Build and check

The default feature set can be built and tested without the pinned native LLVM
toolchain:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

Native compiler features require the LLVM prefix specified by
`native/llvm/manifest.toml`. Follow the source-build instructions in
[`README.md`](README.md), then run:

```sh
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
```

Include the commands you ran and their results in your pull request. If a
required native dependency is unavailable, say so and run the checks that do
not need it.

## Pull requests

1. Create a branch from `main` and make a focused change.
2. Add or update tests for changed behavior. Update the bilingual docs when a
   documented contract changes.
3. Run the relevant checks above and review your diff for generated files,
   accidental secrets, and unrelated edits.
4. Open a pull request against `main`. Describe the problem, the approach, any
   compatibility impact, and the verification performed. Link related issues.
5. Respond to review feedback and keep required checks passing. Maintainers
   review and merge changes; opening a pull request does not guarantee
   acceptance.

Do not put vulnerability details in a public issue or pull request. Use the
private process in [`SECURITY.md`](SECURITY.md).
