# Phase B Acceptance — explicit `void`

Status: accepted on 2026-08-26

Every item is required before Phase C production work begins.

## Syntax and checker acceptance

- [x] `void` is reserved and is accepted only as a function return type.
- [x] Void params, locals, ordinary fields, pointer/slice elements, and arguments
      are rejected with `CK2011` at source level.
- [x] `return;`, natural fallthrough, and early void return are accepted.
- [x] Missing/unexpected return values are rejected with stable diagnostics.
- [x] Only a void-returning call is a legal standalone call statement.
- [x] Non-void results cannot be discarded and void calls cannot enter any value
      context.
- [x] Phase A reachability remains correct after empty returns.

## MIR and optimizer acceptance

- [x] `MirType::Void` occurs only in function returns.
- [x] Void call targets and void return values are `None`; no synthetic
      `MirValue`, local, temp, parameter, constant, or place exists.
- [x] Natural fallthrough becomes an explicit valueless MIR return.
- [x] Validator-negative tests cover every value/no-value mismatch.
- [x] Printer, DCE, CSE, inliner, CFG walkers, use-def collectors, and temp
      collectors handle the new shapes at O0–O3 without panic.
- [x] Targetless calls remain side effects and are never deleted.

## Backend/runtime matrix

| Contract | Unchecked C | Status C | WASM | LLVM |
| --- | --- | --- | --- | --- |
| source void signature | `void` | `CK_Status`, no result pointer | no result | `void` |
| explicit/natural return | `return;` | `return CK_OK;` | no value | `ret void` |
| internal void call | plain call | status propagate | call/no set | `call void` |
| buffer mutation runtime | [x] | [x] | [x] | [x] |

- [x] Status C does not validate or append a nonexistent `ck_return` for void.
- [x] Checked arithmetic failures propagate through a chain of void calls.
- [x] Single-block, dispatcher, and structured WASM paths handle void functions.

## Documentation acceptance

- [x] `examples/void.ck` demonstrates early return, fallthrough, call statement,
      and caller-owned mutation.
- [x] English and Chinese language, MIR, ABI, checked arithmetic, WASM, LLVM,
      architecture, and roadmap documents agree.
- [x] Existing public examples and ABIs remain unchanged.
- [x] New syntax remains outside TypeScript-oracle parity fixtures.

## Required commands

```bash
cargo test --locked --test lexer_test --test parser_test --test checker_test --test mir_test
cargo test --locked --test optimizer_test
cargo test --locked --test c_backend_test --test wasm_backend_test --test llvm_backend_test
cargo test --locked --test cli_test --test docs_surface_test
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
CALCKERNEL_TS_ROOT=/Users/lynn/code/CalcKernel cargo test --locked
cargo build --release --locked
git diff --check
```

## Evidence record

| Date | Command / check | Result | Notes |
| --- | --- | --- | --- |
| 2026-08-26 | lexer/parser/checker/MIR target suites | pass | 59 tests; syntax, `CK2011`, optional MIR values, validation, and TypeScript MIR parity |
| 2026-08-26 | optimizer target suite | pass | 15 tests; targetless calls and valueless returns retained at O0–O3 |
| 2026-08-26 | C/WASM/LLVM target suites | pass | 33 tests; native and Node runtime coverage plus checked-status propagation |
| 2026-08-26 | initial CLI/docs target suite | fail, corrected | `return @;` was mistaken for empty return after lexer recovery; added a focused parser regression and preserved the prior CK0001/CK1001 diagnostic sequence |
| 2026-08-26 | CLI/docs target suites after repair | pass | 36 tests; CLI build runtime and bilingual ABI contracts covered |
| 2026-08-26 | direct `examples/void.ck` emission | pass | check plus optimized MIR, unchecked/status C, WAT, and LLVM emitted outside the repository |
| 2026-08-26 | initial `cargo fmt --check` | fail, corrected | rustfmt-only layout differences; ran `cargo fmt` without semantic edits |
| 2026-08-26 | fmt and strict Clippy | pass | `cargo fmt --check`; all targets/features with warnings denied |
| 2026-08-26 | full locked suite with TS oracle | pass | 157 tests, 0 failures; doctests pass |
| 2026-08-26 | locked release build | pass | optimized `ckc` built successfully |
| 2026-08-26 | `git diff --check` | pass | no whitespace errors |

## Exit decision

- [x] Every checkbox is satisfied.
- [x] Phase B changes and this evidence are ready for the dedicated phase commit.

Accepted by inline execution: 2026-08-26
