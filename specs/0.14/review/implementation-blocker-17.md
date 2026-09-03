# Implementation blocker 17: cumulative same-core and self-contained evidence repair

Date: 2026-09-04

## Finding

Exact V0.13 CI run `33795954634` invalidated the previous cumulative repair.
AArch64 job `100784364185` still observed the shared approximately 4.42 ms / 8.84 ms
bands in the short `slp_quad` streams after four conditioning batches because
the Linux thread could migrate between vCPUs. x86-64 job `100784363740` also
proved that schema 8 retained `results-schema7.json` without the relative
`measurement-*` evidence directory named by that JSON.

V0.14 shares the schema-7 harness, executes fresh schema-8 compatibility, and
replays exact V0.13. Carrying either defect or retaining the superseded V0.13
identity would make schema-9 cumulative acceptance incomplete.

## Resolution

V0.14 inherits the repaired V0.12 Linux same-core scope and the V0.13
self-contained schema-8 collector with their regression tests. Every active
V0.12 replay owner pins `11ca3dbb1220710f184e3c32c873b267d24a22cb`;
every active V0.13 replay owner pins
`1a7f89b841ce3033063ef4a8ac458aa695c8a8c0`; both manifest digests were
recomputed from those exact identities.

The Linux harness chooses one CPU from the inherited allowed mask before any
runner conditioning and restores the mask after the entire three-channel case.
The schema-8 collector validates the closed relative schema-7 directory name,
rejects a redirected root, and retains the complete directory beside
`results-schema7.json`.

No timed work, conditioning count, sample count, statistic, threshold, corpus,
oracle, tuning/search policy, language/ABI rule, or platform/job matrix changed.
V0.14 requires a fresh exact-SHA cumulative CI run.
