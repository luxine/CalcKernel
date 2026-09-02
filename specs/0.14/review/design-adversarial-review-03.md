# CK 0.14 Design Adversarial Review 03

Review target: commit f98af8dd45352e6ea369abbd5bbda5be19445f90

Reviewer: fresh read-only gpt-5.6-sol, ultra reasoning

Verdict: BLOCKED

Blockers: 5

## Blocking findings

### B1. Multi-choice plan ranking is undefined

Plans may contain 64 choices, while predictions exist only per unit variant. No
aggregation or recomputation rule defines plan-level dynamic/static/KIR rank keys,
or the singular class/canonical-order key for a multi-choice plan.

### B2. Candidate wire-state matrix is incomplete

Outcomes include compiled-unmeasured, size-rejected, search-nonwinner, timeout,
validation rejection, and selected, but field/stream requirements are specified
only for timeout. The overview incorrectly makes every other missing stream invalid,
and there is no unambiguous terminal outcome for a qualifying second-ranked
validation entrant.

### B3. Different output-set locks can overlap physical outputs

The set id includes the configurable decision path. Two commands with the same
primary/sidecars and different tune-out paths therefore take different locks while
mutating the same artifact files.

### B4. Cold determinism incorrectly requires raw decision-byte equality

The canonical decision digest covers calibration and nanosecond samples. Independent
cold runs cannot be required to have equal whole-file digests. A separate
measurement-independent choice identity is required.

### B5. Schema-9 auxiliary hashes remain undefined

Recipe digest has no domain/framing, environmentDigest has no canonical material,
outputSetDigest is ambiguous between journal path identity and replay content, and
toolchain retained-file hashes do not identify their files.

## Confirmed closure

Runner/manifest snapshot authority, cooperative containment, calibration/deadline,
session ordering, smoke timeout, Windows environment count, manifest input order,
intra-set journal format/barriers/recovery, output sidecars, and English/Chinese
parity are closed. The publication issue is only cross-set overlap.

The reviewer confirmed clean HEAD f98af8dd45352e6ea369abbd5bbda5be19445f90
and made no edits.
