# Implementation blocker 22: inherit bounded schema-7 band calibration

Date: 2026-09-04

## Finding

Exact V0.12 run `33825887411` proved that the fixed 64-batch ramp still left
all three unchecked `slp_quad` channels split almost evenly between hosted
AArch64 execution bands. V0.14 inherits schema 7 and exact V0.13 replay, so both
accepted identities had to advance together.

## Resolution

V0.14 inherits the fail-closed `bounded-upper-band-v1` calibration from V0.12
and repins its accepted V0.13 revision and independent replay to
`ee8dc5f25e3df085b359608c57a0fba0f3490213`. V0.14 keeps its stronger direct
Darwin `fgetattrlist` runtime implementation. Active specifications, manifests,
scripts, and contract tests bind the new identities.

No timed work, sample count, seven-call upper-median statistic, stability or
performance threshold, corpus, target policy, or required platform/job matrix
changed.
