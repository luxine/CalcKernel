# Implementation blocker 57: normalize inspected output byte counts

## Evidence

V0.14 exact-SHA workflow-dispatch run `34217917663`, commit
`fc1fcd9333248246d84b9fe40139bbb9d9d7057f`, completed both performance jobs
with failures while the workflow was still in progress.

The AArch64 job `102034356856` completed the full schema-9 collection and wrote
all seven decisions, artifacts, samples, receipts, and retained replay evidence.
The independent checker rejected `branch-layout` with `schema-9 tuning decision
branch-layout disagrees with decoded decision`. Field-by-field comparison against
the retained 47,102-byte decision proved that all digests, reason, certificate,
logical names, roles, and hashes matched. Only `outputRecords[].bytes` differed:
the collector normalized inspected textual `u64` values such as `"1504"` to JSON
integers, while the checker left the same decoded values as strings.

The x86-64 job `102034357269` independently passed schema 8, then correctly
rejected its AMD EPYC 7763 runner because it exposed AVX2 but no AVX-512 and
therefore could not satisfy the frozen real `x86-64-v4` tier. That environmental
failure is not repaired by weakening capability requirements; the replacement
exact-SHA run must obtain a real v4 worker.

## Root cause and rejected shortcuts

The schema-9 decision inspector intentionally renders typed `u64` nodes as text
in its JSON tree. The collector converted those nodes to integers before writing
the performance report, but the separately implemented checker omitted the same
normalization before comparing the report with the retained decision. This was a
checker type-normalization defect, not corrupted evidence or a tuning decision
mismatch.

Accepting numeric strings in the report, skipping retained-decision equality,
removing output sizes, retrying only the AArch artifact, or relaxing the x86 v4
requirement is rejected.

## Repair

A focused regression test was added first and observed failing with `str` instead
of `int`. The checker now parses the inspected output-byte node and passes it
through the existing schema-9 `u64` range checker before equality. The retained
AArch64 decisions then pass independent decoding without changing any evidence.

No language or public ABI, safety or strict-FP semantic, tuning decision, evidence
field, schema version, target eligibility, performance/stability/size threshold,
timed work, sample count, corpus, platform, or required job changes.

## Acceptance

The focused RED/GREEN regression, complete Python gate mutations, formatting,
lint, complete locked tests, release native build, ordinary/PGO/tune oracle audits,
and replacement exact-SHA ten-job workflow must pass. The replacement x86
performance job must run on a real v4-capable worker; v3-only execution remains a
hard failure.
