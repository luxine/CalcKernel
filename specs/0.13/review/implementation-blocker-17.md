# Implementation blocker 17: V0.13 must inherit the repaired V0.12 x86 schedule

Date: 2026-09-05

## Finding

Exact V0.12 run `33966418774` failed the unchanged x86 domain-fact gate after
selecting `VF4/UF2` for the unchecked noalias fixture. Artifact inspection
showed per-UF `load/compute/store` serialization, leaving the two-suite
faster-oracle/CK geometric mean at approximately 1.0057 instead of the required
1.05. V0.13 replays and cumulatively gates exact V0.12, so retaining the prior
pin would retain a known failing prerequisite.

## Resolution

V0.13 imports the exact V0.12 x86 `UF > 1` dependency-ready list scheduler and
its structural regression test. It repins the independently built V0.12 replay
to `e1bcea461492a5a2619cdb960ea00dd668847f0a` and binds the resulting manifest
digest in scripts and tests.

No V0.13 PGO or multiversion behavior changes. No language/ABI rule,
performance threshold, timed work, sample count, statistic, corpus, target
policy, or required platform/job matrix changes.
