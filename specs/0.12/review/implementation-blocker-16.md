# Implementation blocker 16: one 32-batch ramp left a residual AArch64 band

Date: 2026-09-04

## Finding

Exact V0.12 run `33823603857` failed AArch64 performance job
`100871814907`. For unchecked `slp_quad`, the corrected once-per-retained-sample
ramp reduced the approximately 4.42/8.84 ms split to one candidate outlier and
two C outliers, but pinned Rust still had five outliers. The unchanged
stability rule requires at least 16 of 20 samples within 75%--125% of the
median; Rust therefore correctly failed at 15 of 20.

## Rediagnosis

The placement fix from blocker 15 is effective: the seven timed calls no
longer multiply the ramp, and the retained samples are overwhelmingly in one
frequency band. The exact worker evidence nevertheless shows that 32
unmeasured batches do not provide enough fixed settling margin for every
channel on the hosted Neoverse-N2 CPU. This is a residual host-state problem,
not a CK code-generation regression and not grounds for weakening the
stability rule.

## Resolution

The identical fixed ramp is increased from 32 to 64 unmeasured batches and
still runs exactly once immediately before each retained seven-call sample.
The manifest and structural tests pin both the new count and its placement.

No timed work, twenty-sample count, seven-call count, rotation, upper-median
statistic, stability rule, performance threshold, corpus, target policy, or
required platform/job matrix changed.
