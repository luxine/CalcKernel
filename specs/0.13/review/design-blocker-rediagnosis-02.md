# CK 0.13 Design Blocker Rediagnosis 02

## Decision

Both review 02 blockers reproduce and are accepted.

- Current `run_build` selects one Native executable consumer or one shared
  Native library consumer for dynamic/static/object output. Exact physical kind
  in profile identity contradicted this topology and made promised object use
  unreachable.
- Current object emission runs more than a non-copying layout tail. LLVM's
  standard machine block placement can consume profile information and perform
  tail duplication, so the first revision did not enforce O2's frozen
  permission boundary.

## Closed contract

### R1: profile topology is semantic; artifact kind is packaging

`CkProfileIdentity` now stores `native-executable` or `native-library` topology,
not dynamic/static/object kind. Compatible dynamic/static/object library
profiles interoperate when every canonical KIR/site/target/safety field matches.
Physical kind remains an exact final artifact/cache field. Native `emit-kir`
validates its selected consumer topology.

### R2: O2 has no LLVM profile metadata

O2 profile analysis remains a sidecar while ordinary IR and all structural
machine passes run profile-blind. A verified snapshot is taken only after those
passes. The CK-owned late-layout pass may permute existing blocks, functions,
and sections and perform only required terminator/fallthrough repair. It cannot
copy/delete bodies or change non-terminator instructions. Only target branch
relaxation, fixups, padding, and emission follow. A separate verifier checks the
closed pre/post delta. O3 retains LLVM metadata and broader transformations.

## Additional closure

The revised design also freezes runtime directory file-identity revalidation,
concurrent idempotent library flush, a static-library shutdown boundary, and
checker-proven histogram cost-difference bounds. These refinements do not lower
any correctness, performance, size, or CI gate.

## Gate for review 03

Planning remains forbidden until a new ultra adversarial reviewer verifies both
closures against current source and reports no blocking contradiction.

