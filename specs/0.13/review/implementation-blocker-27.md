# Implementation blocker 27: shared-library dead-section elimination

Date: 2026-09-07

## Finding

Exact V0.13 run `34049750799`, AArch64 performance job `101531090175`,
passed the unchanged cumulative schema-7 gate and all schema-8 runtime gates,
but failed the schema-8 artifact-size gate. The branch-layout ordinary artifact
was 1,808 bytes and its multiversion artifact was 4,576 bytes, a ratio of
`2.53097` against the required maximum of `2.5`. The retained evidence showed
that runtime throughput and dispatch selection were otherwise valid.

No threshold, workload, sample count, statistic, platform, or required job may
change.

## Rediagnosis

The dispatch runtime was already compiled with function and data sections, but
the embedded shared-library LLD entry point did not request unreachable-section
elimination on ELF, Mach-O, or COFF. Consequently a dynamic multiversion library
retained compiler-private routines such as executable startup-stack capture and
the unused generic ranked selector even though its generated resolver called
only capability detection. The AArch64 artifact contained 164 bytes of
`__ck_dispatch_capture_initial_stack` and 116 bytes of
`__ck_dispatch_select_ranked`, in addition to their alignment and metadata.

This was a final-link closure defect, not a reason to weaken multiversioning or
the size gate.

## Resolution

The embedded shared-library linker now enables the native deterministic
dead-section option for every object format: `-dead_strip` for Mach-O,
`/opt:ref` for COFF, and `--gc-sections` for ELF. Only unreachable private
function/data sections are removed. User exports, referenced helpers, generated
dispatch thunks, capability detection, the selected target members, static
archives, and executable-link behavior are unchanged.

A source contract regression requires all three platform spellings in the
shared-link entry point. A real Native dynamic-library regression verifies that
the link retains the intended medium helper, removes the compact inlined helper,
and does not retain the two unreachable compiler-private runtime symbols.
Relinking the failed job's exact retained AArch64 ELF objects in manifest order
with the same LLD 22.1.8 changed the artifact from the reproduced 4,576 bytes
to 4,048 bytes, or `2.23894` times its 1,808-byte ordinary control. The local
AArch64 Darwin link also reduced its multiversion diagnostic artifact to only
64 bytes more than its ordinary counterpart. Authoritative Linux AArch64 size
and performance remain the CI gate.

No CK language or public ABI, strict floating or safety rule, target ISA,
candidate frontier, profitability floor, code-growth budget, performance or
stability threshold, timed work, sample count, corpus, platform, or required CI
job changed.

## Verdict

Accepted implementation blocker. Complete local verification must pass before
publishing a new exact candidate SHA. The Linux AArch64 performance job and all
other required jobs remain mandatory.
