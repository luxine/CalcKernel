# Implementation blocker 39: fixed checked-map bounds must survive the pre-O3 handoff

## Evidence

V0.14 exact-SHA workflow-dispatch run `34165564989`, commit
`255eb4b6992ab1beb37615ee481a8e4829972037`, rebuilt exact V0.13
`e869763366283e46cd76ffbf3bb85c6c3959c25c` and failed x86-64 performance job
`101876627269` while preparing the required historical schema-7 replay. With the complete
`20,000,000` timed batch iterations, three warmups, twenty rotating samples, and unchanged
corpus, the two checked domain cases were stable but missed the unchanged 5% geometric floor:

- `contract_noalias`: CK `9,333,104 ns`, faster Rust oracle `9,718,765 ns`, ratio `1.0413218`;
- `contract_fixed_length`: CK `9,331,452 ns`, faster Rust oracle `9,719,105 ns`, ratio
  `1.0415426`.

The retained candidate objects used a single checked scalar element per iteration. By
comparison, the earlier exact artifact from run `34133617442`, before the over-broad checked
streaming-map repair, used the bounded two-way schedule and measured the fixed-length case at
`6,584,248 ns` versus `10,179,688 ns` for the faster oracle (`1.5461x`). This is machine-code
evidence that the existing bounded schedule is sufficient when the fixed bound is recognized;
it is not grounds to restore that schedule for unknown-length streaming maps.

## Root cause and rejected shortcuts

The checked-loop scheduler classified production IR before LLVM O3 and before mem2reg. At
that point the fixed-length contract is represented by `llvm.assume(n == 16)`, while the loop
condition still loads `n` from an entry alloca. The scheduler therefore could not recover the
bound argument, classified the loop as an unknown-length checked streaming map, and attached
`llvm.loop.unroll.disable`. LLVM later promoted the alloca and folded the assume, but the
already-attached metadata prohibited the profitable bounded schedule.

Restoring two-way expansion for every checked map would reintroduce blocker 37's measured
regression on long `map_u32` and `zip_u32` loops. Naming fixtures, changing the domain floor,
reducing timed work or samples, or weakening checked overflow/bounds behavior are rejected.

## Repair

The x86 checked-loop handoff now clones each production function solely for analysis, promotes
entry allocas in the clone, and maps each production loop to its SSA analysis counterpart. A
scalar checked memory map receives the existing bounded two-way schedule when its recovered
bound argument is either constant at every direct call or constrained equal to an integer
constant by `llvm.assume`. Unknown-length checked streaming maps continue to receive
`llvm.loop.unroll.disable`; metadata is still attached only to the production loop. Ordinary
and multiversion Native object cache identities advance to
`x86-checked-memory-map-schedule-v3`.

No language or public ABI rule, overflow/bounds behavior, target ISA, inline or growth budget,
performance/stability/artifact-size threshold, timed work, sample count, corpus, platform, or
required job changes.

## Acceptance

The focused structural regression must fail before the repair and pass afterward, proving the
analysis clone, mem2reg handoff, constant-equality assume recognition, and distinct bounded and
streaming schedules remain wired. Formatting, lint, unit/performance tests, Native bridge
compilation, and applicable local Native regressions are rerun. Replacement exact-SHA ten-job
runs remain authoritative for x86-64 machine-code performance and the full six-host matrix.
