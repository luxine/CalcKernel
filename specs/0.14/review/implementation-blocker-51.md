# Implementation blocker 51: inherit the repaired V0.13 PGO oracle audit

## Evidence

Exact-SHA V0.13 workflow-dispatch run `34178811720`, commit
`21448738b90ccfd1ea9ab79e9355450ef325769c`, failed both required performance
jobs. Linux x86-64 job `101913645601` and Linux AArch64 job `101913645752`
completed the ordinary oracle audit, then failed the PGO oracle audit because
it passed a raw record dictionary to the migrated `Kernel` constructor. The
failure preceded performance evidence creation, so artifact upload also failed.

V0.14 exact run `34178814878` still pinned that rejected V0.13 revision and
carried the same stale audit call. Even without a V0.14 job failure yet, its
accepted replay dependency and own inherited oracle audit were therefore
superseded by the independently diagnosed V0.13 failure.

## Root cause and rejected shortcuts

The schema-8 shared-address repair introduced `KernelWorkspace`, but the PGO
oracle audit was missed during the interface migration. V0.14 inherited that
call site and could neither validate its own performance job nor reconstruct an
acceptable exact V0.13 replay.

Allowing V0.14 to retain the rejected replay SHA, accepting an old V0.13 run,
skipping the PGO audit, or reducing any oracle/performance work is rejected.
V0.13 and V0.14 remain independently accountable to their exact-SHA workflows.

## Repair

V0.14 imports the complete V0.13 repair at
`77e5e0a95b83d0faa8f63ddc8f2451a9b1322a40`, including its RED/GREEN structural
regression, shared-workspace PGO oracle call, root-cause record, and master
control update. For every record, C, UBSan, and Rust oracle kernels now receive
one shared `KernelWorkspace`.

The accepted V0.13 base, replay manifest, preparer, structural contract, and
current V0.14 task/acceptance/design documents are repinned to the same exact
V0.13 commit. The resulting `benches/baselines/v0_13_replay.toml` SHA-256 is
`9cb05a28b504e504ccd6dca50617241e11a29a8e8d9034b35f4d2b72d181dfe8`.
Historical blocker records retain the superseded identities as evidence.

This repair changes no language or public ABI rule, safety or strict-FP
semantics, oracle source, target ISA, schema 8 or 9 threshold, timed work,
sample count, corpus, platform, or required job matrix.

## Verification

The inherited constructor regression is observed RED before import and GREEN
afterward. The ordinary and PGO oracle audits, V0.14 replay structural tests,
formatting, lint, release, repository, and exact-SHA ten-job workflow gates are
rerun. The replacement V0.13 and V0.14 workflows remain separate authorities.
