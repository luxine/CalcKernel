# Final Acceptance — CK control flow, void, and slices

Status: not started

This is the final branch-level gate. It does not replace phase acceptance and may
not weaken any phase requirement.

## Prerequisite evidence

- [ ] Phase A acceptance is complete and its commit is present.
- [ ] Phase B acceptance is complete and its commit is present.
- [ ] Phase C acceptance is complete and its commit is present.
- [ ] Any post-phase repair has a recorded reproduction and does not lower an
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

- [ ] Unchecked C, WASM, and LLVM produce the same valid output at O0–O3.
- [ ] Checked C produces the same valid output at O0–O3.
- [ ] Checked C invalid index returns `CK_ERR_OUT_OF_BOUNDS` without mutation
      after failure.
- [ ] Combined overflow/bounds case returns the earlier observable error.
- [ ] WAT/WASM/LLVM checked-bounds commands reject with the documented messages.

The fixture may live in backend/CLI tests and reuse `examples/slices.ck`; it must
not be presented to the legacy TypeScript oracle.

## Contract audit

- [ ] All four new words are reserved and migration notes identify compatibility
      impact.
- [ ] No first-class unit value or general expression statement was introduced.
- [ ] No labels, break values, omitted slice endpoints, direct nested slices,
      allocation, ownership, null validation, or raw-pointer bounds checks were
      introduced.
- [ ] Exported slice returns remain rejected.
- [ ] Slice params are flattened on every backend, including internal calls.
- [ ] Checked bounds remains C-only and module-wide status ABI is activated by
      either checked mode.
- [ ] MIR remains semantic and backend-independent.
- [ ] Existing public example signatures remain stable.

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

- [ ] Every materially changed formal English document has an updated Simplified
      Chinese counterpart.
- [ ] README links resolve to the intended language/ABI/backend documents.
- [ ] Examples use `.ck` and commands use `ckc`.
- [ ] Bounds, ownership, raw escape, status codes, and backend mode matrix agree
      across all documents.
- [ ] Roadmap marks only actually delivered capabilities as complete.

## Git handoff audit

- [ ] Branch is `feature/control-void-slice`.
- [ ] Worktree is `/Users/lynn/code/Rust_CalcKernel/.worktrees/control-void-slice`.
- [ ] All intended files are committed.
- [ ] No unrelated user changes are included.
- [ ] No merge to `main` was performed.
- [ ] Final commit IDs and verification evidence are ready for user review.

## Evidence record

| Date | Command / audit | Result | Notes |
| --- | --- | --- | --- |
| | | | |

## Final decision

- [ ] Accepted for user review; do not merge.

Accepted by inline execution: pending
