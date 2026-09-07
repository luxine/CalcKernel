# Implementation blocker 34: resolver-entry fact-audit lineage

Date: 2026-09-07

## Finding

Replacement exact V0.13 run `34116187720` exposed the same failure in native
integration, x86-64 Linux, and AArch64 Darwin. The contracted multiversion tests
reported four LLVM `readonly` and four `writeonly` parameter attributes while
the CK fact ledger expected three of each.

The new resolver entry correctly inherits the baseline function's parameter and
function attributes so that its indirect must-tail call preserves the target
contract. The earlier repair registered inherited fact lineage only for the
steady dispatcher, leaving the resolver entry's valid strengthenings untracked.

## Resolution

`NativeModule::add_multiversion_dispatch` now records the same source-linked
inherited attribute set once for each of the two generated exact-ABI call
layers: the cold resolver entry and the steady dispatcher. The existing closed
filter remains unchanged: only alignment, readonly, writeonly, memory effects,
and parameter noalias are inherited. Body-owned assume, range, no-wrap, and
alias-scope evidence is not duplicated.

A structural RED/GREEN contract requires both ledger copies. The existing
native audit remains the behavioral authority and still fails on every
unregistered LLVM strengthening.

## Frozen boundaries

No LLVM attribute was removed, no fact-audit comparison was weakened, and no
language, ABI, profile, target, performance/stability threshold, timed work,
sample, corpus, platform, or required job changed.

## Verdict

Accepted implementation blocker. A replacement exact-SHA ten-job run must
validate the complete native matrix and independent performance jobs.
