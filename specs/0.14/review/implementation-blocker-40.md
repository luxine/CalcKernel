# Implementation blocker 40: sub-resolution timing changed cold plan identity

Date: 2026-09-07

## Finding

Exact V0.14 run `34106159689`, x86-64 performance job `101691955900`,
completed its schema-7 and schema-8 gates but failed schema-9 collection for
`contract-fixed-length`:

    schema-9 collection failed: contract-fixed-length independent cold tuning is not deterministic

The complete job log and `performance-x86-64` artifact `10013831124` preserve
both cold decisions, raw measurement streams, event logs, artifacts, cache
snapshots, and supervisor receipts. `cold-one` selected plan
`493f1128445db8f84296fc3470d7bec9fae3c6e611e3cd092a5163f595e84230`;
`cold-two` selected the baseline with `validation-disagreement`. The warm run
exactly reused `cold-one`, so cache publication and reuse were not the failure.

All validation candidates passed the unchanged stability and profitability
requirements. Their aggregate Q32 scores were tightly clustered: cold-one round
one ranged from `3830979833` through `3831247471`, while cold-two round two ranged
from `3830161453` through `3830300640`. Nevertheless, the implementation ranked
every nanosecond-derived Q32 delta before deterministic artifact/choice/digest
keys. Small host noise therefore changed both the three-member search entrant set
and the leading validation plan even though the contract simultaneously required
independent cold runs to publish the same plan and output content.

## Rediagnosis

The schema correctly retains exact raw timings and exact Q32 ratios for audit and
threshold enforcement, but it lacked a declared measurement resolution for
ranking. Treating an arbitrarily small timing delta as reproducible evidence was
the contradiction. Removing the cold determinism requirement, repeating samples
after observing the result, or relaxing performance thresholds would only hide
the defect and is rejected.

## Contract repair

Search and validation now derive, with checked integer arithmetic:

    score_percent_ceiling = ceil(score_q32 * 100 / 2^32)

Ranking uses the lower one-percentage-point ceiling bucket, then smaller actual
primary-artifact bytes, fewer non-baseline choices, and lower plan digest. Exact
Q32 values remain recorded and continue to decide the unchanged 0.97 aggregate,
1.02 per-case, stability, and 16-of-20 paired-win requirements. The repair adds no
rerun, sample clipping, floating-point comparison, or discretionary selection.

The formula is a checked derivation of existing Decision Schema 1 fields, so the
wire shape and persisted evidence remain unchanged. Source-aware acceptance binds
the corrected compiler implementation and rederives both search entrants and each
validation rank from the retained raw streams.

Two regression tests first failed under the exact-Q32 ordering: independent
search sessions with swapped sub-bucket timings produced different entrant order,
and two validation rounds produced different winners. They now require identical
deterministic ordering while retaining the original exact threshold tests.

## Contract preservation

Language and public ABI, Runtime ABI 2, Native ABI 1, CKTUNE01 fields, candidate
frontier, target ISA, correctness semantics, cache safety, workload, timed work,
warmups, measured rows, calls per evaluation, corpus, platform matrix, required
jobs, performance thresholds, and stability thresholds are unchanged. V0.14 still
requires its independent exact-SHA schema-9 acceptance.

## Verdict

Accepted implementation blocker. Relevant local gates and a replacement exact-SHA
V0.14 CI run are mandatory before final acceptance.
