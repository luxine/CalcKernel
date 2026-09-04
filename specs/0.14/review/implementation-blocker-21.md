# Implementation blocker 21: inherit the final schema-7 settling margin

Date: 2026-09-04

## Finding

Exact V0.12 run `33823603857` failed AArch64 performance job
`100871814907` because the pinned Rust unchecked `slp_quad` stream retained five
alternate-band samples after a fixed 32-batch ramp. The unchanged stability
rule correctly rejected 15 stable samples out of 20. V0.14 inherits schema 7
and exact V0.13 replay, so both identities had to advance together.

## Resolution

V0.14 inherits the fixed 64-batch, once-per-retained-sample ramp from V0.12 and
repins its accepted V0.13 revision and independently built replay to
`2baa45a49c687692dc3cba05a627742cbfdcbe69`. V0.14 keeps its stronger direct
Darwin `fgetattrlist` runtime implementation. Active specifications, manifests,
scripts, and contract tests bind the new identities.

No timed work, sample count, seven-call upper-median statistic, stability or
performance threshold, corpus, target policy, or required platform/job matrix
changed.
