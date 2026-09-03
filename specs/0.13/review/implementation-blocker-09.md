# Implementation blocker 09: repaired scheduler migration and cumulative evidence retention

Date: 2026-09-04

## Finding

Exact V0.13 CI run `33795954634` failed both performance workers for two
independent reasons:

- AArch64 job `100784364185` rejected
  `vectorSuites/unchecked/slp_quad rustSimdSamplesNs` as unstable. Its sample
  stream still alternated between approximately 4.42 ms and 8.84 ms after the
  fixed four-batch conditioning ramp. Candidate CK and C showed the same two
  bands. The earlier V0.12 pass contained the same residual bands below the
  rejection count, so conditioning alone had not closed the scheduler flake.
  The Linux thread could condition one vCPU and migrate to a differently
  conditioned vCPU before timing.
- x86-64 job `100784363740` produced a schema-8 report whose retained
  `results-schema7.json` referenced `measurement-27362-1788468209606443935`,
  but the collector copied only the JSON into the schema-8 evidence root. The
  referenced schema-7 evidence directory remained at the outer
  `target/ckc-perf` level, so the independent cumulative checker correctly
  failed closed on a missing measured directory.

## Rediagnosis

The AArch64 artifact from V0.13 and the passing V0.12 artifact both prove a
shared scheduler/execution-state effect across independent CK, C, and Rust
implementations; this is not an optimizer regression and cannot be repaired by
selective reruns. Linux `sched_getaffinity`/`sched_setaffinity` provides a
deterministic same-core boundary without changing measured work.

The x86-64 artifact contains the complete schema-7 directory next to the
schema-8 directory. The defect is therefore collector retention, not missing
measurement generation or a checker path error. A portable schema-8 artifact
must carry both the retained JSON and the exact relative directory that JSON
names.

## Resolution

V0.12 exact candidate `11ca3dbb1220710f184e3c32c873b267d24a22cb`
pins each Linux three-channel schema-7 case to one CPU already allowed by the
inherited affinity mask before runner creation, keeps the existing four
conditioning batches, and restores the prior mask when the case ends. V0.13
inherits that implementation and pins its replay to the repaired exact V0.12
candidate.

The schema-8 collector now validates the cumulative JSON's
`evidenceDirectory` against the closed `measurement-<pid>-<timestamp>` form,
rejects a redirected source root, copies that whole tree without following
symlinks, and then retains the JSON as `results-schema7.json`. Regression tests
cover a complete portable copy, traversal rejection, and symlink-root
rejection.

No timed work, conditioning count, sample count, statistic, threshold, corpus,
oracle, optimization policy, language/ABI rule, or platform gate was changed.
Both repaired candidates require fresh exact-SHA CI.
