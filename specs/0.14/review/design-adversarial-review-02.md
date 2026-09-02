# CK 0.14 Design Adversarial Review 02

Review target: commit ee7e7a0875419deeabe29d51cecad8a11d4bd498

Reviewer: fresh read-only gpt-5.6-sol, ultra reasoning

Verdict: BLOCKED

Blockers: 6

## Blocking findings

### B1. Deterministic ordering is not closed

`session_digest` drives every measurement rotation but is never defined, and equal
Q32 search scores have no tie-break at the validation-entrant cutoff. The later
validation tie-break cannot repair a different entrant set.

### B2. Smoke timeout cannot be serialized

Every post-calibration candidate timeout is a normal rejection, including the
required pre-search smoke invocation, but `TimeoutRecord.phase` only admits the six
warmup/measured `OrderingPhase` values and has no smoke phase.

### B3. Windows effective environment can exceed its wire bound

The manifest accepts 16 inherited names, while `SystemRoot` and `WINDIR` can be
additional identity entries. The decision wire allows only 16 total entries and
defines neither a total-limit nor reserved-slot rule.

### B4. Canonical logical records still have undefined digests and ordering

The manifest hash lacks its domain and primitive framing; `siteAlternatives` lacks
canonical order; singular candidate/certificate correctness digests lack an
aggregation rule over multiple cases; and round, certificate, replay-result, plan,
and empty-plan digest derivations are absent.

### B5. Durable publication is not one crash-consistency protocol

The journal remains a prose field list without exact framing, bounds, integrity and
atomic-update rules, or a complete filename scheme. Publication also omits explicit
parent-directory durability barriers between backup/output renames and their
journal phases.

### B6. Schema 9 acceptance remains partially discretionary

Mandatory rotations have no domain/serialization formula; the five tune manifests
are named but their logical records are not frozen; enhanced CPU tiers remain
workflow-declared; and domain tuned channels lack retained decision/build
provenance.

## Important non-blocking finding

The English and Chinese overviews say inputs are sorted by logical mount name while
the wire authority says manifest order. The attachment wins, but both overviews
should be corrected.

## Review summary

First-review B2 is closed. The runner-kind/snapshot part of B1 is closed. The
findings above leave deterministic/wire details of B1 and B3 through B7 partially
open. English and Chinese have no material policy divergence and share the same
defects through the normative attachments.

The reviewer confirmed HEAD ee7e7a0875419deeabe29d51cecad8a11d4bd498,
clean status, and no reviewer edits.
