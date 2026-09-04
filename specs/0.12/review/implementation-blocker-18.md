# Implementation blocker 18: upper-band selection and UF-scaled x86 admission are unsound

Date: 2026-09-04

## Finding

Exact V0.12 run `33833225186` failed both authoritative performance jobs:

- AArch64 job `100900399564` exhausted all 256 unchecked `slp_quad` settling
  probes without re-entering the channel's calibrated upper-duration band. The
  benchmark then produced no report, so the diagnostic step also failed while
  trying to open the absent `target/ckc-perf/results.json`.
- x86-64 job `100900399817` measured unchecked `strict_f64` at 89.47% of its
  faster SIMD oracle, below the unchanged 90% individual floor. The retained
  domain evidence also showed that `contract_noalias` and
  `contract_fixed_length` did not establish the required 5% geometric-mean
  advantage over generic C/Rust.

Exact V0.13 run `33833224700` independently reproduced both classes of
failure. AArch64 job `100900397192` rejected the Rust SIMD `slp_quad` stream as
unstable, while x86-64 job `100900397115` rejected the cumulative schema-7/8
domain-fact gate. Exact V0.14 run `33833224718` then failed both performance
jobs while replaying the pinned V0.13 schema-8 evidence. V0.14 is downstream
of the same defects; it is not an independent threshold failure.

## Rediagnosis

### Short-kernel measurement

The fixed 4-, 32-, and 64-batch ramps and `bounded-upper-band-v1` have now
failed in four successive designs. The current upper-band selector assumes
that the slower AArch64 duration band is both reproducibly reachable and more
authoritative than the faster band. Neither assumption follows from the
evidence:

- the upper band can disappear for more than 256 full-batch probes;
- one successful probe does not predict the following seven calls;
- CK, C, and Rust move between approximately 4.42 ms and 8.84 ms together,
  which identifies a shared multiplicative host-frequency state rather than a
  channel-specific regression.

Increasing another probe count would preserve the invalid architecture. The
gate must compare channels within the same frequency state and reject
channel-specific noise without requiring one arbitrarily selected absolute
band.

### x86 short-loop admission

The x86 profitability floor currently requires `4 * VF * UF` scalar
iterations. One vector-loop chunk already contains `UF` vector operations, so
the formula charges the interleave factor twice when expressing the intended
four-vector-operation amortization floor. At runtime `n = 16`, the selected
`VF4/UF4` plan therefore requires 64 elements and immediately takes its scalar
epilogue. For the exact-length fixture, the competing scalar unroll path
materializes sixteen scalar operations instead of SIMD. Both outcomes retain
extra CK control or scalar work while generic Clang/Rust emit a compact SIMD
loop.

## Approved repair

### Interleaved upper-median protocol

Replace `bounded-upper-band-v1` with
`interleaved-upper-median-three-channel-v2` for every CK/C/Rust oracle case.
For retained row `r` and raw repetition `k` in `0..7`, execute all three
channels once in the rotation beginning at `(r + k) mod 3`. Each channel still
contributes exactly seven timed full batches to the row; its retained duration
is still the upper median of those seven values. No timed value may be
discarded, retried, substituted, or moved into an unmeasured region.

Warmup remains three rows with one call per channel. Release evidence remains
twenty retained rows. The report continues to retain every per-channel row
duration and its exact upper median. The oracle manifest and source-level
contract tests bind the new raw-call schedule.

For cases other than `slp_quad`, the existing absolute stability rule remains
unchanged. For `slp_quad`, the checker derives a common-mode-normalized stream
from the three retained durations in each row:

`q[c,r] = d[c,r] / geometric_mean(d[0,r], d[1,r], d[2,r])`.

Each channel must still have at least 16 of 20 normalized values within
75%..125% of that channel's normalized median. A shared multiplicative
frequency transition cancels; a transition or outlier affecting only one
implementation remains a failure. Raw positive values, exact sample counts,
stored medians, rotation, equivalence, and all performance comparisons remain
fail-closed. Performance ratios continue to use the retained raw upper-median
durations; normalization is only the stability test for the four-item
microkernel.

### x86 vector-operation floor

Keep the 20% modeled profitability requirement. On x86-64, express the
additional amortization floor as four actual vector operations:

`minimum_groups = ceil(4 / UF)`

`minimum_trip >= minimum_groups * VF * UF`.

Consequently `VF4/UF4` admits a 16-element vector iteration rather than
requiring 64 elements. AArch64 retains its existing two-complete-group floor,
`minimum_trip >= 2 * VF * UF`. The proposer and independent checker must
recompute the same target-specific rule independently. Regression tests must
prove that x86 `VF4/UF4` admits the exact 16-element noalias loop, that a
runtime 16-element invocation enters SIMD, and that genuinely short exact
trips remain rejected.

## Acceptance and propagation

The repair must be implemented test-first. Before publishing a replacement
V0.12 candidate, local acceptance must include the focused sampler, checker,
vector discovery, independent transaction checker, native lowering, and
schema tests, followed by the repository's complete required local gates.
Generated x86 evidence must show SIMD for both domain fixtures; if any original
performance threshold still fails, that is a new blocker and must be diagnosed
rather than hidden by this protocol.

After V0.12 passes locally, V0.13 must import the exact repair and repin its
exact V0.12 replay identity. V0.14 must then import the repaired V0.13 state and
repin its exact V0.13 replay identity. Superseded remote runs may be cancelled
only after replacement commits are pushed. The monitor must be updated to the
new exact SHAs and run IDs.

No language or ABI rule changes. No timed work, batch size, twenty-row count,
seven-call count, upper-median statistic, 16-of-20 stability count, 75%..125%
band, 90%/95% SIMD threshold, 5% domain threshold, corpus, CPU policy, compiler
identity, target matrix, or required CI job may be reduced or skipped.
