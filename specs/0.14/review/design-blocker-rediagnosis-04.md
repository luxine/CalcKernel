# CK 0.14 Design Blocker Rediagnosis 04

Review source: `design-adversarial-review-04.md`

Verdict: all nine findings are confirmed blockers. None requires weakening product,
safety, determinism, or performance gates; each closes an implementation or
independent-verification ambiguity.

## R1. Manifest encoding

Confirmed. Manifest argv and logical input paths will be required to already be
NFC, NUL-free, and individually bounded by decision `Text`; the exact accepted
UTF-8 bytes are executed/staged. Shared tagged `ManifestInputMaterial` and
`ManifestCaseMaterial` records will freeze id, role, digest, and size ordering.

## R2. Timeout streams

Confirmed. A timed-out candidate will retain the canonically sorted set—not a
prefix—of exactly those measured streams completed before the timeout in the
recomputable rotated invocation schedule. The timed-out partial stream remains
absent and the timeout coordinate proves the boundary.

## R3. Validation selection

Confirmed. Let each round first filter qualifying plans and then rank them. Empty
qualifier set in either round selects `validation-threshold`; otherwise equal
round winners select `tuned`, and different winners select
`validation-disagreement`. Candidate terminal outcomes will follow the selected
reason without overlap.

## R4. Canonical leaf identities

Confirmed. The revision will add retained class-specific alternative payloads;
stable root anchors; canonical KIR state material for site, unit, and whole-plan
states; a primary-artifact digest; and typed compile/measurement cache key and entry
materials. Cross-record equalities will make all stored digests recomputable.

## R5. Inspection schema

Confirmed. A shared normative inspection attachment will define exact JSON bytes
and a line-oriented text grammar over the complete parsed decision tree. The
fixture will be an instance of that schema rather than its definition.

## R6. Crash recovery

Confirmed. Lock initialization and journal writes will use private unique write
files and atomic no-replace exposure of complete metadata. The journal will carry a
durable forward/rollback direction; a closed active/update/write-state table and
direction-specific idempotent recovery will cover every reachable crash state.

## R7. Filesystem lookup equivalence

Confirmed. Tune output leaf names will use a closed ASCII-safe grammar. Destination
identity will bind the opened parent directory's stable filesystem identity and the
leaf key under queried case sensitivity; unsupported/unknown semantics fail closed.
Existing aliases and all destination comparisons use this same key.

## R8. Validation performance evidence

Confirmed. Schema 9 will retain a seven-case external validation split with tuned,
v0.13 ordinary, and v0.13 PGO raw channels. The independent checker will derive the
faster v0.13 comparator and enforce the unchanged 102/100 ceiling.

## R9. Evidence foreign keys

Confirmed. Every file identity will have an explicit repository/evidence root.
Main, validation, and domain timing cases will retain closed build commands, bind
their channel artifacts to those command outputs, and bind tuned decision/output
identities to the seven canonical tuning artifact records.

## R10. Optimizer pipeline seam

Confirmed from the review's repository-feasibility appendix. The revision will
freeze the precise pre-tune snapshot, the seven class application phases, unit and
alternative order, mandatory v0.13 bridge passes, layout-metadata boundary, and
empty-plan behavior. This prevents two implementations from replaying the same
plan at different O3 points.

## R11. Row permutation identity

Confirmed. The revision will define `permutationKey` with the same typed,
domain-separated material used by the scheduler and distinguish its stored
per-case key from the recomputed case-list rotation.

## R12. Journal role and orphan closure

Confirmed from the publication appendix. Journals will accept only the three valid
decision/sidecar/primary layouts. Full 64-hex set and destination ids will appear
in reserved names so a pre-Prepared orphan cannot be confused with a digest-prefix
collision; unavailable atomic filesystem primitives will fail before staging.

## R13. Raw release samples and correctness

Confirmed. Schema 9 will retain all seven calls for every external measured row,
derive the stored minimum, and retain per-channel correctness digests. The
independent checker will derive internal and differential results and rerun the
recipe-pinned adversarial/oracle audit instead of trusting summary booleans.

## R14. Cross-section performance provenance

Confirmed. The first cold determinism run will be the authoritative retained
decision/output set. Timing, artifact-size, resource, and warm-reuse evidence will
reference it, and environment entries will retain both the exact executed value
and the file identities that give path-valued variables their semantic meaning.

## Acceptance rule

Revision 04 must implement all fourteen corrections in the shared attachments and both
language designs, be committed cleanly, and undergo a fresh ultra-reasoning Sol
review. Planning remains prohibited until a review returns PASS with zero blockers.

## Applied revision

All fourteen confirmed corrections are applied across the English and Chinese
designs plus Decision Schema 1, Inspection Schema 1, Publication Journal Schema 1,
and Performance Report Schema 9. The shared attachments are authoritative where a
translated overview is intentionally shorter. The revision preserves every safety,
determinism, and performance threshold and adds no discretionary fallback. A fresh
review must still return PASS before planning may begin.
