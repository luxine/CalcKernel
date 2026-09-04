# Implementation blocker 15: V0.13 must inherit the final V0.12 settling margin

Date: 2026-09-04

## Finding

Exact V0.12 run `33823603857` showed that the corrected once-per-retained-sample
32-batch ramp still left five alternate-band samples in the pinned Rust
`slp_quad` stream on AArch64. The unchanged stability rule correctly failed at
15 of 20 samples. V0.13 replayed that exact V0.12 harness, so leaving its base
pin unchanged would preserve a known failing prerequisite.

## Resolution

V0.13 inherits the fixed 64-batch, once-per-retained-sample ramp and repins its
independently built exact V0.12 replay to
`a49fa419669c400447dc13bcfa41ea464b3b040d`. All replay manifests, hashes,
scripts, tests, and active acceptance documents bind that same identity.

No timed work, sample count, seven-call upper-median statistic, stability or
performance threshold, corpus, target policy, or required platform/job matrix
changed.
