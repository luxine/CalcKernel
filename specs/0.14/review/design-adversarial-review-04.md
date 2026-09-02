# CK 0.14 Design Adversarial Review 04

Review target: commit ce6bcd94b28d12dd7c8f222963374ab378101550

Reviewer: fresh read-only gpt-5.6-sol, ultra reasoning

Verdict: BLOCKED

Blockers: 9

## Blocking findings

### B1. Manifest-to-decision encoding is inconsistent

The manifest admits argv under an aggregate 64 KiB bound without requiring the
per-value NFC and 4,096-byte `Text` constraints, so a valid invocation can be
unencodable or execute bytes different from its identity. The main design's
manifest digest prose also disagrees with the decision attachment on input field
order and omission of case ids.

Minimum correction: align manifest validation and executed argv with `Text`, and
freeze identical typed input and case material records in both authorities.

### B2. Rotated timeout cannot always satisfy the terminal schema

The invocation schedule rotates cases by row, while the timed-out outcome requires
a canonical prefix of stored streams. A timeout can complete a later canonical
stream while leaving an earlier one incomplete, making either storage choice
invalid.

Minimum correction: define the stored set as every stream completed before timeout
in recomputable deterministic invocation order, or retain partial rows.

### B3. Validation outcomes overlap

Different round winners selects `validation-disagreement`, while no plan qualifying
in both selects `validation-threshold`. Evidence where only A qualifies in round 1
and only B in round 2 satisfies both, and the selected reason affects choice
identity.

Minimum correction: freeze a disjoint exhaustive decision table and corresponding
candidate outcomes in both language designs.

### B4. Decision identities lack canonical leaf material

Alternative choice payloads, root/state digests, and compile/measurement cache key
and entry digests lack bounded typed material and domains. An independent checker
cannot reproduce them.

Minimum correction: freeze material/domains for every alternative class, root and
state, and cache key/entry, including cross-record equalities.

### B5. Inspection schema 1 is unspecified

Stable text and JSON inspection are public behavior, but exact JSON keys/types and
text grammar do not exist; naming a future fixture cannot define them.

Minimum correction: add the closed JSON schema and stable text grammar, and bind
the fixture to that contract.

### B6. Publication has unrecoverable reachable crash states

Journal update and lock files expose their final names before complete writes, a
sole valid journal update is undefined, rollback direction is not durable, and a
crash during rollback from a late phase can be misclassified as roll-forward.

Minimum correction: atomically expose complete metadata, define safe persistent
lock initialization, durably record recovery direction, and provide an exhaustive
recovery table for active/update/interrupted-write/interrupted-rollback states.

### B7. Destination locks ignore filesystem lookup equivalence

Exact Unix path bytes are not sufficient on case- or normalization-insensitive
Darwin filesystems. Byte-distinct absent leaf paths can address one entry but take
different locks.

Minimum correction: use parent-filesystem lookup equivalence for aliases, ordering,
and lock ids, or fail closed when it cannot be established.

### B8. Schema 9 cannot evaluate the validation-vs-v0.13 gate

The policy applies the 102/100 ceiling to validation and release-held-out inputs,
but schema 9 retains six-channel external samples only for release-held-out.
Internal validation compares against the v0.14 tuning baseline, not v0.13 ordinary
and PGO.

Minimum correction: retain closed raw validation-channel evidence against both
v0.13 comparators and have the checker derive the faster comparator and gate.

### B9. Schema 9 measured artifacts are not referentially closed

Main cases do not bind the timed tuned artifact to tuning artifacts/decisions,
main-channel build provenance is absent, and `FileIdentity.path` has ambiguous
repository/evidence roots.

Minimum correction: add explicit foreign-key equalities and build commands for all
timed channels, and give every file identity an unambiguous root.

## Confirmed nonblockers

- English and Chinese have no separate material divergence because shared
  attachments are normative.
- Existing optimizer transactions and platform output-set code provide feasible
  integration seams.
- Missing final v0.13 acceptance artifacts are already a provisional release gate.
- Clarifying an old zero-byte destination journal rule is useful but not blocking.

The reviewer confirmed clean commit ce6bcd94b28d12dd7c8f222963374ab378101550
and made no edits.

## Supplemental findings from the review's scoped audits

The primary verdict reported nine consolidated blockers. Its two read-only scoped
audits also exposed five narrower issues that remain genuine and are included in
the same rediagnosis rather than deferred:

- the pre-tune O3 snapshot and class application order were not frozen;
- `permutationKey` lacked exact canonical material;
- the journal admitted invalid role layouts and abbreviated pre-Prepared orphan
  namespaces;
- release evidence retained row minima but not all seven underlying calls or
  per-channel correctness observations;
- timing, artifact-size, resource, and determinism identities did not share one
  authoritative first-cold provenance graph.

These findings do not change the BLOCKED verdict or weaken any requested gate.
