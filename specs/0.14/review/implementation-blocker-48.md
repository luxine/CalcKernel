# Implementation blocker 48: inherit fixed-bound SSA classification and repin V0.13 replay

## Evidence

Exact-SHA V0.14 workflow-dispatch run `34165564989`, commit
`255eb4b6992ab1beb37615ee481a8e4829972037`, failed x86-64 performance job
`101876627269` while rebuilding exact V0.13
`e869763366283e46cd76ffbf3bb85c6c3959c25c`. The required schema-7 checker measured stable
checked domain ratios of `1.0413218x` and `1.0415426x`, below the unchanged `1.05x`
geometric floor, and therefore correctly stopped schema-8/9 preparation.

The full diagnosis, historical machine-code comparison, and V0.13 repair are recorded in
`specs/0.13/review/implementation-blocker-39.md`. V0.13 commit
`aa757ca0f78664cbfa4f824d655d820c87368dd3` is published on
`design/v0.13-pgo-multiversion` and is the only accepted replay identity for this closure.

## Inherited repair and replay identity

V0.14 commit `df679fbe05810464e7d2d5a1206e35193ab0ca56` inherits the exact checked-loop repair:

- each pre-O3 production loop is classified through a mem2reg-promoted analysis clone;
- a checked scalar map whose recovered bound argument has an integer constant-equality
  `llvm.assume`, or is constant at every direct call, receives the existing bounded two-way
  schedule;
- unknown-length checked streaming maps retain `llvm.loop.unroll.disable`;
- ordinary and multiversion Native object cache identities advance to
  `x86-checked-memory-map-schedule-v3`.

The V0.13 replay manifest is repinned to exact commit
`aa757ca0f78664cbfa4f824d655d820c87368dd3` and compiler identity
`calckernel 0.13.0 (aa757ca0f78664cbfa4f824d655d820c87368dd3)`. Its new SHA-256 is
`d58fd2cbaaf35fb611bd666b0e027f4467291d06a27bb073ddfb54d431062898`; the preparer,
schema-9 contract regression, normative design, phase acceptance, and final acceptance all
bind that same immutable identity.

No language or public ABI rule, overflow/bounds behavior, target ISA, tuning decision schema,
performance/stability/artifact-size threshold, timed work, sample count, corpus, platform,
schema-9 tier, or required job changes.

## Acceptance

The inherited structural regression must fail before the bridge/cache repair and pass after it.
V0.14 formatting, lint, unit/performance/Python tests, Native bridge compilation, and applicable
local Native regressions are rerun. A replacement exact-SHA V0.14 ten-job run must rebuild the
new V0.13 commit and independently pass all unchanged historical and current performance gates;
V0.14 cannot replace V0.13's separate exact-SHA acceptance.
