# Implementation blocker 54: inherit the Windows profile-root repair

## Evidence

Exact-SHA V0.13 workflow-dispatch run `34182330156`, commit
`77e5e0a95b83d0faa8f63ddc8f2451a9b1322a40`, failed the required Windows x64
CLI suite because all four real PGO training executables returned the fixed
directory status `43`. The repair and platform root cause are recorded in
`specs/0.13/review/implementation-blocker-43.md`.

The concurrent V0.14 run `34192455322`, commit
`c9950c756e12d8bf8dd578ccc051c9a1e7dc1975`, still pinned that rejected V0.13
revision. Its AArch64 performance job also rejected the retained schema-8
multiversion source-to-object geometric mean at `2.5002854475`, against the
unchanged `2.5` maximum. The independently executed V0.13 AArch64 job for the
same historical SHA measured `2.37818`; both reports contain all fifteen
samples for every channel and neither permits selective replay or threshold
adjustment. Once the V0.13 candidate failed its independent Windows gate, the
outer V0.14 run could no longer authorize that historical dependency in any
case.

## Repair

V0.14 imports the exact V0.13 Windows collector repair at
`528f0734a0c4525a2c84158c4d73067e468f292c`, including its RED/GREEN structural
contract, source provenance, root-cause record, and master-control evidence.
The accepted-base manifest, independent replay preparer, structural contract,
and current normative/task/acceptance documents are repinned to that same
commit. The resulting `benches/baselines/v0_13_replay.toml` SHA-256 is
`2b2d2e66333b3eed4b8bd260325f3440e0040e6cdf43c1f4d81546eb5756eba4`.

The failed schema-8 compile result is not discarded, selectively rerun, or
made passing by changing a limit. The replacement exact-SHA V0.14 workflow must
rebuild and measure the newly accepted V0.13 compiler as a complete replay on
the same assigned worker. A repeated compile-time failure remains actionable.

No language or public ABI rule, strict-FP or safety semantic, target ISA,
schema-8/schema-9 shape, performance/stability/artifact-size threshold, timed
work, sample count, corpus, platform, or required job changes.

## Acceptance

The Windows path regression, V0.13 replay-pin contract, complete local gates,
and exact-SHA ten-job workflow must pass. The required performance jobs must
execute the unmodified sample protocol and thresholds; V0.14 cannot substitute
its own cumulative result for the independently retained V0.13 schema-8 gate.
