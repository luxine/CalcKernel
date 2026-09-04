# Implementation blocker 16: V0.13 must inherit bounded SLP band calibration

Date: 2026-09-04

## Finding

Exact V0.12 run `33825887411` showed that a fixed 64-batch ramp still left all
three unchecked `slp_quad` channels split almost evenly across the hosted
AArch64 execution bands. V0.13 inherits schema 7 and replays exact V0.12, so
the prior base remained a known failing prerequisite.

## Resolution

V0.13 inherits `bounded-upper-band-v1`: each channel calibrates from 64
unmeasured probes and must re-enter its conservative upper-duration band within
256 unmeasured probes before every retained seven-call sample. V0.13 repins its
independently built exact V0.12 replay to
`af9aa37d262d9b447f407f07aa73e33ed63b4926` and binds the new manifest hash in
scripts and tests.

No timed work, sample count, seven-call upper-median statistic, stability or
performance threshold, corpus, target policy, or required platform/job matrix
changed.
