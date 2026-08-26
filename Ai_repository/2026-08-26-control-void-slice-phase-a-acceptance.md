# Phase A Acceptance — `break` and `continue`

Status: not started

This document is a gate, not a progress estimate. Every required item must have
passing evidence before Phase B production work begins.

## Semantic acceptance

- [ ] `break;` and `continue;` are reserved, spanned statements requiring `;`.
- [ ] Each targets the innermost lexical `while` through nested blocks and `if`s.
- [ ] Either outside a loop is `CK2009` with a source span.
- [ ] A statement after unconditional return/break/continue is `CK2010`, never a
      MIR lowering error.
- [ ] Branch summaries retain fallthrough when either branch can continue.
- [ ] A `while`, including `while true`, remains conservatively fallthrough for
      missing-return analysis.
- [ ] Existing non-void missing-return behavior remains unchanged.

## MIR and optimizer acceptance

- [ ] MIR contains ordinary jumps to the correct condition/exit blocks and no
      break/continue-specific backend convention.
- [ ] Nested loops have distinct target pairs and all blocks validate.
- [ ] CFG simplification preserves effective loop-control targets at O0–O3.
- [ ] No accepted loop-control source causes an internal error or backend panic.

## Backend/runtime matrix

| Case | C O0–O3 | WASM O0–O3 | LLVM O0–O3 |
| --- | --- | --- | --- |
| early break | [ ] | [ ] | [ ] |
| continue skips work | [ ] | [ ] | [ ] |
| nested innermost loop | [ ] | [ ] | [ ] |
| mixed return/control | [ ] | [ ] | [ ] |

- [ ] O3 WASM proves dispatcher fallback for a non-simple control-flow graph.
- [ ] Existing simple O3 while still uses the structured path where applicable.

## Documentation acceptance

- [ ] `examples/control_flow.ck` compiles for C, WAT/WASM, and LLVM.
- [ ] English and Chinese language, architecture, MIR, and roadmap documents
      agree on syntax, reachability, and lowering.
- [ ] README links remain correct.
- [ ] The example is not added to TypeScript-oracle parity lists.

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
| | | | |

## Exit decision

- [ ] Every checkbox is satisfied.
- [ ] Phase A changes and this evidence are ready for the dedicated phase commit.

Accepted by inline execution: pending
