# Phase B Acceptance — explicit `void`

Status: waiting for Phase A

Every item is required before Phase C production work begins.

## Syntax and checker acceptance

- [ ] `void` is reserved and is accepted only as a function return type.
- [ ] Void params, locals, ordinary fields, pointer/slice elements, and arguments
      are rejected with `CK2011` at source level.
- [ ] `return;`, natural fallthrough, and early void return are accepted.
- [ ] Missing/unexpected return values are rejected with stable diagnostics.
- [ ] Only a void-returning call is a legal standalone call statement.
- [ ] Non-void results cannot be discarded and void calls cannot enter any value
      context.
- [ ] Phase A reachability remains correct after empty returns.

## MIR and optimizer acceptance

- [ ] `MirType::Void` occurs only in function returns.
- [ ] Void call targets and void return values are `None`; no synthetic
      `MirValue`, local, temp, parameter, constant, or place exists.
- [ ] Natural fallthrough becomes an explicit valueless MIR return.
- [ ] Validator-negative tests cover every value/no-value mismatch.
- [ ] Printer, DCE, CSE, inliner, CFG walkers, use-def collectors, and temp
      collectors handle the new shapes at O0–O3 without panic.
- [ ] Targetless calls remain side effects and are never deleted.

## Backend/runtime matrix

| Contract | Unchecked C | Status C | WASM | LLVM |
| --- | --- | --- | --- | --- |
| source void signature | `void` | `CK_Status`, no result pointer | no result | `void` |
| explicit/natural return | `return;` | `return CK_OK;` | no value | `ret void` |
| internal void call | plain call | status propagate | call/no set | `call void` |
| buffer mutation runtime | [ ] | [ ] | [ ] | [ ] |

- [ ] Status C does not validate or append a nonexistent `ck_return` for void.
- [ ] Checked arithmetic failures propagate through a chain of void calls.
- [ ] Single-block, dispatcher, and structured WASM paths handle void functions.

## Documentation acceptance

- [ ] `examples/void.ck` demonstrates early return, fallthrough, call statement,
      and caller-owned mutation.
- [ ] English and Chinese language, MIR, ABI, checked arithmetic, WASM, LLVM,
      architecture, and roadmap documents agree.
- [ ] Existing public examples and ABIs remain unchanged.
- [ ] New syntax remains outside TypeScript-oracle parity fixtures.

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
| | | | |

## Exit decision

- [ ] Every checkbox is satisfied.
- [ ] Phase B changes and this evidence are ready for the dedicated phase commit.

Accepted by inline execution: pending
