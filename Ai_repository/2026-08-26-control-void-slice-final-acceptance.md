# Final Acceptance — CK control flow, void, and slices

Status: complete

This is the final branch-level gate. It does not replace phase acceptance and may
not weaken any phase requirement.

## Prerequisite evidence

- [x] Phase A acceptance is complete and its commit is present (`d36e1dd`).
- [x] Phase B acceptance is complete and its commit is present (`ffa7354`).
- [x] Phase C acceptance is complete and its commit is present (`dabfc16`).
- [x] Any post-phase repair has a recorded reproduction and does not lower an
      approved contract.

## End-to-end language scenario

One direct fixture must combine all new features:

- exported void entry accepting a `slice<Struct>`;
- internal function returning a sub-slice;
- `.len` loop condition;
- `continue` skipping selected elements;
- `break` ending traversal;
- checked write through slice index;
- natural and explicit void return paths.

Acceptance:

- [x] Unchecked C, WASM, and LLVM produce the same valid output at O0–O3.
- [x] Checked C produces the same valid output at O0–O3.
- [x] Checked C invalid index returns `CK_ERR_OUT_OF_BOUNDS` without mutation
      after failure.
- [x] Combined overflow/bounds case returns the earlier observable error.
- [x] WAT/WASM/LLVM checked-bounds commands reject with the documented messages.

The fixture may live in backend/CLI tests and reuse `examples/slices.ck`; it must
not be presented to the legacy TypeScript oracle.

## Contract audit

- [x] All four new words are reserved and migration notes identify compatibility
      impact.
- [x] No first-class unit value or general expression statement was introduced.
- [x] No labels, break values, omitted slice endpoints, direct nested slices,
      allocation, ownership, null validation, or raw-pointer bounds checks were
      introduced.
- [x] Exported slice returns remain rejected.
- [x] Slice params are flattened on every backend, including internal calls.
- [x] Checked bounds remains C-only and module-wide status ABI is activated by
      either checked mode.
- [x] MIR remains semantic and backend-independent.
- [x] Existing public example signatures remain stable.

## Static repository audit

```bash
rg -n "TODO|FIXME|unimplemented!|todo!" src tests examples docs
rg -n "panic!|expect\\(" src
rg -n "break|continue|void|slice" docs docs/zh-CN README.md README.zh-CN.md
git diff --check main...HEAD
git status --short
```

The panic/expect search is reviewed, not blindly required to be empty: existing
validated invariants may remain, but no accepted new source form may reach a new
panic path. Any new occurrence must have validator/checker coverage and a written
rationale in the evidence table.

## Final verification commands

Run from a clean worktree after all implementation commits:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
CALCKERNEL_TS_ROOT=/Users/lynn/code/CalcKernel cargo test --locked
cargo build --release --locked
cargo run --locked --bin ckc -- --help
git diff --check main...HEAD
git status --short
git log --oneline --decorate main..HEAD
```

Also verify the repository CI-equivalent command defined by the current workflow
if it differs from the commands above.

## Documentation parity audit

- [x] Every materially changed formal English document has an updated Simplified
      Chinese counterpart.
- [x] README links resolve to the intended language/ABI/backend documents.
- [x] Examples use `.ck` and commands use `ckc`.
- [x] Bounds, ownership, raw escape, status codes, and backend mode matrix agree
      across all documents.
- [x] Roadmap marks only actually delivered capabilities as complete.

## Git handoff audit

- [x] Branch is `feature/control-void-slice`.
- [x] Worktree is `/Users/lynn/code/Rust_CalcKernel/.worktrees/control-void-slice`.
- [x] All intended files are committed.
- [x] No unrelated user changes are included.
- [x] No merge to `main` was performed.
- [x] Final commit IDs and verification evidence are ready for user review.

## Evidence record

| Date | Command / audit | Result | Notes |
| --- | --- | --- | --- |
| 2026-08-26 | Phase commit audit | Pass | Planning `1f2a06f`; Phase A `d36e1dd`; Phase B `ffa7354`; Phase C `dabfc16`; combined acceptance fixture `b6d314a`. |
| 2026-08-26 | `cargo test --test control_void_slice_e2e_test -- --nocapture` | Pass, 3/3 | One direct fixture covers exported void plus `slice<Struct>`, internal sub-slice return, `.len`, `continue`, `break`, checked write, natural return, and explicit return. C/WASM/LLVM agree at O0–O3; checked C preserves memory on OOB and reports operand overflow before bounds; non-C checked modes reject. |
| 2026-08-26 | Combined fixture red/green record | Pass | Initial harness compilation exposed only an unused local in the new natural-void test fixture under `-Werror`; the local was used in an empty-slice early-return branch without changing product code or weakening the contract, then all three tests passed. |
| 2026-08-26 | Contract and invariant audit | Pass | Checker/parser/MIR negative suites reject unit values, non-call expression statements, labels/break values by grammar, omitted endpoints, nested/void slices, exported slice returns, and type mismatches. Backend slice parameters and calls are covered on C/WASM/LLVM. New backend `panic!`/`expect` sites are internal representation invariants fenced by checker plus `mir_validator_should_reject_each_malformed_slice_operation` and related validator tests; accepted source cannot reach them. |
| 2026-08-26 | `rg -n "TODO\|FIXME\|unimplemented!\|todo!" src tests examples docs` | Pass | No matches. Keyword/documentation search confirms all language and backend surfaces; docs tests verify English/Simplified Chinese parity and the shared slice example. |
| 2026-08-26 | `cargo fmt --check` | Pass | Clean implementation commit. |
| 2026-08-26 | `cargo clippy --all-targets --all-features --locked -- -D warnings` | Pass | No warnings. |
| 2026-08-26 | `CALCKERNEL_TS_ROOT=/Users/lynn/code/CalcKernel cargo test --locked` | Pass | Full suite passed, including all backend O0–O3 tests, CLI tests, documentation tests, combined acceptance, and legacy TypeScript-oracle compatibility. |
| 2026-08-26 | `cargo build --release --locked` | Pass | CI-equivalent release build succeeded. Current `ci.yml` and `native-release.yml` define the same fmt/clippy/test/release gates run here. |
| 2026-08-26 | `cargo run --locked --bin ckc -- --help` | Pass | Help documents `--bounds`, C-only checked support, and non-C unchecked-only modes. |
| 2026-08-26 | Git/static handoff audit | Pass | `git diff --check main...HEAD` passed; worktree was clean; `main..HEAD` contained only the five scoped planning/phase/acceptance commits before this evidence-only commit. Branch/worktree match this document and no merge was performed. |

## Final decision

- [x] Accepted for user review; do not merge.

Accepted by inline execution: 2026-08-26
