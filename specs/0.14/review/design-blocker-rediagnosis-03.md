# CK 0.14 Design Blocker Rediagnosis 03

Review source: `design-adversarial-review-03.md`

Verdict: all five findings are confirmed blockers. They can be closed without
changing the product architecture or weakening any release threshold.

## R1. Plan-level ranking

Confirmed.

Resolution:

- after each legal extension, apply the complete plan to a fresh pre-tune snapshot;
- recompute whole-plan dynamic/static cost and canonical KIR byte length;
- rank multi-choice plans by those values, choice count, lexicographic class vector,
  lexicographic `(unit id, variant id)` vector, and plan digest;
- record those whole-plan metrics in every successful expansion.

## R2. Candidate state matrix

Confirmed.

Resolution:

- add a validation-nonwinner outcome;
- freeze required/forbidden object, correctness, timeout, and stream fields for
  every terminal outcome;
- define exactly which search/validation streams are complete for each state;
- make a missing required stream invalid without forbidding intentionally
  unmeasured states.

## R3. Overlapping output-set serialization

Confirmed.

Resolution:

- derive one persistent lock per canonical physical destination, not per set;
- acquire all destination locks in canonical path-byte order before inspecting a
  set journal;
- retain the set-id journal for transaction discovery, but serialize any two sets
  sharing even one decision/artifact/sidecar destination;
- release locks in reverse order and keep persistent lock files.

## R4. Determinism identity

Confirmed.

Resolution:

- add `choiceIdentityDigest`, derived from measurement-independent identity,
  frontier, selected plan/reason, object graph and link recipe;
- cold sessions must match choice identity, code identities, and role-tagged output
  content, not measurement-bearing raw decision bytes;
- warm completed-decision reuse must reproduce the original decision and output
  bytes exactly and compile/measure zero candidates.

## R5. Schema-9 auxiliary identities

Confirmed.

Resolution:

- define typed domain-separated recipe and environment digests;
- rename outputSetDigest to `outputContentDigest` and derive it from role/digest/
  size records, not journal paths;
- name the exact retained LLVM component manifest and Clang profile-runtime files;
- close command and environment inputs under the same file/hash rules.

## Acceptance rule

Revision 03 must be committed and undergo another fresh ultra-reasoning Sol review.
Planning remains prohibited until a review returns PASS with zero blockers.

## Applied revision

Revision 03 implements all five resolutions in the shared normative attachments
and both language designs:

- successful expansions now carry recomputed whole-plan rank metrics and complete
  deterministic tie-break vectors;
- the candidate terminal-state matrix includes validation nonwinners and freezes
  every optional/required field combination;
- publication uses persistent per-destination locks plus stable overlap-closure
  discovery and recovery;
- decision schema 1 carries a measurement-independent choice identity, while
  schema 9 distinguishes cold choice/code/content equality from exact warm byte
  reuse;
- schema 9 defines typed recipe, hardware, command-environment, and output-content
  digests and retains identified LLVM/Clang evidence files.
