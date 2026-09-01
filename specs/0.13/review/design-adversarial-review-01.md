# CK 0.13 Design Adversarial Review 01

## Scope and method

This review examined the canonical and Chinese CK 0.13 PGO/multiversion
specifications against the current 0.12 CLI, Native artifact, LLVM bridge,
cache, ABI, optimizer, and CI implementation. The review looked for
implementability or correctness blockers, not optional polish.

Reviewed design commit: `391c63f`.

## Verdict

`BLOCKED` with three blocking findings.

## Blocking findings

### B1: recursive final-profile merge cannot detect overlapping runs

The design accepted both raw shards and final profiles as merge inputs, required
duplicate runs to be rejected, and deliberately removed member run identities
from a final profile. For example, profiles made from `A+B` and `B+C` have
different final digests but overlap on run `B`; the proposed format cannot
detect that overlap. This changes weights and confidence while appearing valid.

Minimum closure: schema 1 must either retain canonical member provenance or
restrict merge to one raw-shard-to-final layer.

### B2: separate variant modules conflict with the single-object CLI contract

The design required dispatcher, baseline, and enhanced variants to remain in
separate LLVM modules, while the existing `--kind object` contract emits one
`.o`/`.obj`. Current `run_build`, `NativeObject`, and artifact paths have no
multi-object bundle or portable partial-link contract. The missing decision
changes output shape, downstream linking, cache manifests, symbol visibility,
and size accounting.

Minimum closure: reject `multiversion + object`, define a bundle, or specify and
test a six-platform relocatable assembly operation.

### B3: O2 profile metadata visibility does not enforce its no-copy boundary

The current LLVM bridge runs one complete default PassBuilder pipeline. If LLVM
frequency metadata is present before default O2, profile-guided inlining,
vectorization, loop, or CFG transforms may consume it. Merely saying "after the
O2 inlining frontier" does not exclude later copying passes. If metadata is
attached afterward, the design must define the surviving mapping and allowed
tail.

Minimum closure: make the default O2 IR pipeline profile-blind, validate a
post-O2 mapping, then expose profile metadata only to a closed non-copying
layout/codegen tail; alternatively broaden O2's promised permissions.

## Important non-blocking findings

- Profile-weighted dynamic cost arithmetic needed exact integer normalization,
  guard/fallback accounting, saturation behavior, and checker recomputation.
- Library-unload shard failure had no reliable host-visible completion status.
  This is especially poor as a cross-platform lifecycle contract even though it
  does not by itself alter CK program semantics.

## Confirmed closed areas

The source-level contract, no-argument training entry, profile identity and site
topology, safety/profitability separation, target feature tables and fail-closed
detection, dispatcher ABI, cache identity, final-runtime independence, and
performance/CI gates formed an implementable closed loop apart from the
findings above.

