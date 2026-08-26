# CalcKernel Repository Conventions

[简体中文](../zh-CN/project/conventions.md)

The canonical language names are CK and CalcKernel. Source files use `.ck`; the
native compiler is `ckc`; the Rust package/library name is `calckernel`. Do not
introduce alternate `tk`, `tkc`, `.tk`, wrapper, or package-surface aliases.

Rust source is grouped by responsibility under `frontend`, `ir`, `optimizer`,
`backend`, and `cli`. Public compatibility comes through intentional `lib.rs`
re-exports, not broad visibility. Prefer small modules, typed data, borrowed
inputs, explicit error propagation, and deterministic ordered output.

Tests are grouped under matching responsibility directories with shared harness
code in `tests/support`. Examples are runnable CK source grouped by purpose;
benchmarks and their fixtures live under `benches`. Formal English documentation
under `docs` has an exact Simplified Chinese relative-path peer under
`docs/zh-CN`.

Durable documents describe current contracts. Temporary designs, execution
plans, review logs, generated output, local worktrees, and historical process
narratives are not committed to the release tree. Git commits and published
release notes provide history.
