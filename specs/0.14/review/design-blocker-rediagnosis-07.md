# CK 0.14 Design Blocker Rediagnosis 07

Review source: `design-adversarial-review-07.md`

Verdict: all three findings are confirmed blockers.

## R1. Runner path base

Confirmed. Relative `runner.path` will resolve from the canonical manifest parent,
matching configuration-file locality. Absolute paths remain accepted because the
runner is explicitly user-authorized and already excluded from canonical identity;
both forms use no-follow component resolution and immutable snapshotting.

## R2. Expansion ordinal

Confirmed. Ordinals will be zero-based and contiguous. The counter value is copied
to the attempt before increment, and the stored list must be exactly
`0..expansion_count-1`.

## R3. Search-result closure

Confirmed. A source-aware checker will replay the complete candidate space,
expansion algorithm, ranks, and diversity truncation from Frontier and Contract.
Trials will equal the derived compile-selection set. Actual retained/rebuilt
artifact identities will determine size rejection; the remaining complete set will
determine finalists, validation entrants, outcomes, and exact stream requirements.
A budget that prevents completing this closed set produces no decision.

## Acceptance rule

The revision must close all three equalities without weakening the existing beam,
compile, finalist, timeout, size, validation, or performance rules. A new ultra Sol
review with zero blockers remains mandatory.

## Applied revision and self-audit

- Relative runner paths now resolve only from the canonical manifest parent;
  absolute paths are explicitly accepted, and both forms use no-follow component
  walking plus a final file handle before snapshotting.
- The search pseudocode assigns the current counter before increment. Expansion
  records are exactly zero-based, contiguous, complete nested-loop attempts through
  unit exhaustion or the preset cap, with every disposition and metric rederived.
- The checker reconstructs the final beam and compile selection, requires the
  plan-digest-sorted trial list to be exactly that set, independently rebuilds all
  trials, derives the 110% size predicate, applies the exact postcompile diversity
  rank, and constrains every nonfinalist/finalist outcome and stream. Partial work
  caused by the wall budget cannot be serialized.

Result: PASS for this repair round. English, Chinese, and Decision Schema 1 agree;
no beam width, candidate limit, size ratio, measurement protocol, validation rule,
or release threshold changed. A fresh independent review is still required.
