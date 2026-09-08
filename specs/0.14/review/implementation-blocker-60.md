# Implementation blocker 60: suppress the unused Unix-only fetch-add definition

## Evidence

Superseded exact-SHA V0.14 run `34252240969`, commit
`c38ecce50a1f8e2dadef5fdf29453c4ecac5e31d`, completed five required jobs with
the same bootstrap failure before the cancellation request was processed:

- AArch64 performance job `102149886806`;
- native integration job `102149886863`;
- Linux ARM64 native job `102149886902`;
- Darwin ARM64 native job `102149887060`;
- Linux x64 native job `102149887345`.

Every complete job log reports `-Werror=unused-function` for
`ck_profile_atomic_u32_fetch_add_relaxed`; the AArch64 Linux branch reports line
219 and the generic C11 branch reports line 356. Each job stopped during the
pinned prefix bootstrap, so no performance, fact-audit, or capability artifact
was produced.

## Root cause and rejected shortcuts

The uniform atomic abstraction added a 32-bit relaxed fetch-add definition to
all platform branches. Only Windows ARM64 uses that operation for its run-id
serial; collector code uses the other 32-bit operations. On Unix the unused
function was emitted as a non-inline internal definition, so the frozen
`-Wall -Wextra -Werror` runtime recipe rejected it.

Removing `-Werror`, weakening warnings, deleting any required job, skipping
profile runtime construction, or ignoring the artifact failures is rejected.
The upload failures are consequences of bootstrap stopping before their input
paths existed, not separate roots.

## Repair

A branch-specific contract was added first and observed failing. Only the
AArch64 Linux and generic C11 definitions of the Windows-only 32-bit relaxed
fetch-add wrapper are changed from `static` to `static inline`. Their bodies,
memory ordering, assembly, signatures, and Windows definitions are unchanged.
The profile-runtime provenance digest is updated to bind the repaired header.

This changes no language or public ABI, profile format, tuning decision,
schema 8 or 9 rule, optimizer policy, target eligibility,
performance/stability/size threshold, timed work, sample count, corpus,
platform, or required job.

## Acceptance

The focused RED/GREEN contract and a direct Darwin C11 compilation with the
frozen freestanding warning flags pass locally. Formatting, lint, complete
locked tests, provenance checks, and a replacement exact-SHA V0.14 ten-job
workflow must pass. Linux ARM64 and Linux x64 remain authoritative for their
respective header branches; no platform is skipped.
