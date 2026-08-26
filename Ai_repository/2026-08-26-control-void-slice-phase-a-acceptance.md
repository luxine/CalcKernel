# Phase A Acceptance — `break` and `continue`

Status: accepted on 2026-08-26

This document is a gate, not a progress estimate. Every required item must have
passing evidence before Phase B production work begins.

## Semantic acceptance

- [x] `break;` and `continue;` are reserved, spanned statements requiring `;`.
- [x] Each targets the innermost lexical `while` through nested blocks and `if`s.
- [x] Either outside a loop is `CK2009` with a source span.
- [x] A statement after unconditional return/break/continue is `CK2010`, never a
      MIR lowering error.
- [x] Branch summaries retain fallthrough when either branch can continue.
- [x] A `while`, including `while true`, remains conservatively fallthrough for
      missing-return analysis.
- [x] Existing non-void missing-return behavior remains unchanged.

## MIR and optimizer acceptance

- [x] MIR contains ordinary jumps to the correct condition/exit blocks and no
      break/continue-specific backend convention.
- [x] Nested loops have distinct target pairs and all blocks validate.
- [x] CFG simplification preserves effective loop-control targets at O0–O3.
- [x] No accepted loop-control source causes an internal error or backend panic.

## Backend/runtime matrix

| Case | C O0–O3 | WASM O0–O3 | LLVM O0–O3 |
| --- | --- | --- | --- |
| early break | [x] | [x] | [x] |
| continue skips work | [x] | [x] | [x] |
| nested innermost loop | [x] | [x] | [x] |
| mixed return/control | [x] | [x] | [x] |

- [x] O3 WASM proves dispatcher fallback for a non-simple control-flow graph.
- [x] Existing simple O3 while still uses the structured path where applicable.

## Documentation acceptance

- [x] `examples/control_flow.ck` compiles for C, WAT/WASM, and LLVM.
- [x] English and Chinese language, architecture, MIR, and roadmap documents
      agree on syntax, reachability, and lowering.
- [x] README links remain correct.
- [x] The example is not added to TypeScript-oracle parity lists.

## Required commands

Run from the worktree root and record exact results below.

```bash
cargo test --locked --test lexer_test --test parser_test --test checker_test --test mir_test
cargo test --locked --test optimizer_test
cargo test --locked --test c_backend_test --test wasm_backend_test --test llvm_backend_test
cargo test --locked --test docs_surface_test
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
CALCKERNEL_TS_ROOT=/Users/lynn/code/CalcKernel cargo test --locked
cargo build --release --locked
git diff --check
```

## Evidence record

| Date | Command / check | Result | Notes |
| --- | --- | --- | --- |
| 2026-08-26 | lexer/parser/checker/MIR target suite | pass | 43 tests; includes conservative `while true` and TypeScript MIR parity |
| 2026-08-26 | optimizer target suite | pass | 11 tests; O0–O3 loop-control CFG retained |
| 2026-08-26 | C/WASM/LLVM target suites | pass | 28 tests; real O0–O3 runtimes cover early break, skipped work, nesting, and loop return |
| 2026-08-26 | docs surface suite | pass | 7 tests; bilingual contracts and example link checked |
| 2026-08-26 | direct example emission | pass | check plus MIR, C/header, WASM, and LLVM artifacts emitted outside repository |
| 2026-08-26 | initial `cargo fmt --check` | fail, corrected | rustfmt-only layout differences; ran `cargo fmt` without semantic edits |
| 2026-08-26 | fmt and strict Clippy | pass | `cargo fmt --check`; all targets/features with warnings denied |
| 2026-08-26 | full locked suite with TS oracle | pass | 117 tests, 0 failures; doctests pass |
| 2026-08-26 | locked release build | pass | optimized `ckc` built successfully |
| 2026-08-26 | `git diff --check` | pass | no whitespace errors |

## Exit decision

- [x] Every checkbox is satisfied.
- [x] Phase A changes and this evidence are ready for the dedicated phase commit.

Accepted by inline execution: 2026-08-26
