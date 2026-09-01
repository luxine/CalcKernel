# CK 0.13 Design Adversarial Review 03

## Scope and method

This read-only ultra review examined commit `65f2b0fe25c1` against both design
mirrors and the repository's current CLI consumer mapping, KIR inspection,
LLVM 22.1.8 target pipeline, artifact/cache identities, ABI schemas, and CI
topology. It retested every prior blocker and specifically investigated whether
the O2 late-machine boundary is implementable across x86-64/AArch64 and all six
Native host jobs.

## Verdict

`PASS`. This is design-contract approval; implementation and acceptance remain
pending.

## Closure evidence

- `CkProfileIdentity` now follows the executable/library semantic consumer
  split already present in `run_build`; physical output kind remains exact in
  artifact/cache state. Dynamic/static/object library profiles therefore have a
  reachable and non-aliasing compatibility path, and `emit-kir` can validate by
  its selected consumer.
- O2 exposes no profile metadata to LLVM. LLVM 22.1.8 permits the bridge to
  construct the target pipeline through public `TargetPassConfig` boundaries,
  finish target-specific ordinary machine passes, snapshot, run the CK-owned
  late-layout verifier, repeat only required branch relaxation, and emit. No
  common target pass name is required.
- A late layout that would require CFI/unwind/LOH/security or other repair
  outside the closed verifier delta can reject conservatively and keep ordinary
  order; that is not an implementation blocker.
- Runtime directory identity, component-wise no-follow, concurrent sticky flush,
  static shutdown, raw-shard-only merge, saturation fallback, signed histogram
  cost bounds, multiversion object rejection, cache identity, and ABI/schema
  advances form closed contracts.
- English canonical and Chinese mirror contain the same material decisions, and
  the required CI topology remains six Native hosts plus two performance workers
  under the ten-job acceptance set.

## Important implementation constraints

- Keep a closed post-layout repair allowlist per target. If target state needs
  another repair, reject the layout rather than silently widening O2.
- Interpret `full-profile-identity-hex` as the complete lowercase SHA-256 digest
  of canonical identity bytes, not textual serialization.

## Final gate

No design blocker remains. Planning may begin, but no implementation claim is
implied by this review.
