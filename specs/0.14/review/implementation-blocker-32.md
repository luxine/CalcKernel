# Implementation blocker 32: inherit V0.13 shared-link size closure

Date: 2026-09-07

## Finding

Exact V0.13 run `34049750799`, AArch64 performance job `101531090175`,
passed cumulative schema 7 and every schema-8 runtime gate but failed the
unchanged artifact-size gate. Its branch-layout ordinary and multiversion
dynamic libraries were 1,808 and 4,576 bytes, respectively, yielding
`2.53097 > 2.5`.

V0.14 run `34050348155` pinned that exact V0.13 revision. It was cancelled once
the source failure proved the accepted base invalid, rather than spending more
hosted-runner time preparing a replay that could not pass. No V0.14 tuning gate
was waived and no threshold, workload, sample count, statistic, platform, or
required job may change.

## Rediagnosis

The inherited dispatch runtime already emitted separate function/data sections,
but the shared-library LLD entry point did not request dead-section elimination.
Thus dynamic multiversion artifacts retained compiler-private startup capture
and generic selection routines that their generated resolver never referenced.
This is an inherited final-link defect, not a replay-adapter or Auto-Tuning
defect.

## Resolution

V0.14 imports the exact V0.13 repair at
`2ba127c18a6c5f831dc55814c37da2dd1ecedea6`. Its shared-library linker uses
`-dead_strip`, `/opt:ref`, or `--gc-sections` for Mach-O, COFF, or ELF while
retaining user exports and all referenced helpers, dispatch thunks, detectors,
and target members. Contract and real Native link regressions are inherited
unchanged.

Relinking the failed job's retained AArch64 objects with the corrected ELF
option changes the exact 4,576-byte artifact to 4,048 bytes, or `2.23894` times
the ordinary control. The V0.13 replay manifest and compiler identity advance
to the repaired commit; its new SHA-256 is
`e4c8fceab818681350f52715ee334ccfe7325a99167261350c0e2269843e7873`.
No replay adapter is introduced.

No CK language or public ABI, strict floating or safety rule, target ISA,
tuning frontier, profitability or growth budget, performance or stability
threshold, timed work, sample count, corpus, platform, or required CI topology
changed.

## Verdict

Accepted inherited implementation blocker. Exact detached V0.13 replay and
complete V0.14 local gates must pass before publishing a new candidate. Remote
x86-64 and AArch64 performance jobs remain authoritative for both schema 8 and
schema 9.
