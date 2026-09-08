# Implementation blocker 58: honor the normative FileIdentity root order

## Evidence

V0.14 exact-SHA workflow-dispatch run `34225333284`, commit
`32fdbba0799d62afaf0ae240036ce2e11391c235`, completed both performance jobs
with failures while the workflow was still in progress.

The AArch64 job `102058308732` completed schema 8 and the full schema-9
collection, then the independent checker rejected
`cases.branch-layout.buildCommands.v013Pgo.command.inputs` because it was not
path-sorted. The retained list placed the evidence-root profile before the
repository-root CK source. A complete recursive audit of the report found the
same ordering in every mixed-root command input list, including oracle builds,
tune-use compile samples, validation builds, and archive packaging.

The x86-64 job `102058308829` independently passed schema 8, then correctly
rejected its AMD EPYC 7763 runner because it exposed AVX2 but no AVX-512 and
could not satisfy the frozen real `x86-64-v4` tier. The replacement exact-SHA run
must obtain a real v4 worker.

## Root cause and rejected shortcuts

Schema 9 defines FileIdentity root order as `repository` then `evidence`, followed
by UTF-8 path order. The report producer instead sorted the textual root values,
which places `evidence` before `repository`. Most earlier lists contained only one
root, so the defect first became observable in a mixed-root command after the
preceding independent-checker blocker was repaired.

Allowing either root order, skipping command validation, rewriting retained
evidence after collection, removing mixed-root identities, or weakening the x86
v4 requirement is rejected.

## Repair

A focused regression was added first and observed the producer return
`[evidence, repository]`. Producer and independent-checker sorting now share the
same explicit semantic key in their separate implementations: repository rank
zero, evidence rank one, then UTF-8 path bytes. All producer FileIdentity list
sites and all checker-side expected-list reconstruction sites use that key.

No language or public ABI, safety or strict-FP semantic, tuning decision, evidence
field, schema version, target eligibility, performance/stability/size threshold,
timed work, sample count, corpus, platform, or required job changes.

## Acceptance

The focused RED/GREEN regression, complete Python gate mutations, formatting,
lint, complete locked tests, release native build, ordinary/PGO/tune oracle
audits, and replacement exact-SHA ten-job workflow must pass. The replacement
x86 performance job must run on a real v4-capable worker; v3-only execution
remains a hard failure.
