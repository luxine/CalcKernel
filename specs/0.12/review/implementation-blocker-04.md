# CK 0.12 Implementation Blocker 04: Cross-Host Lifetime and Performance Semantics

## Decision

The failures in exact-SHA run `33581641456` are accepted implementation and benchmark-oracle
defects. They do not justify lowering the frozen CK 0.12 performance thresholds, deleting a
kernel, reducing samples, changing the upper-median statistic, or weakening any required job.

## Rediagnosis

- Windows x64 and ARM64 both completed vector differential execution, then failed to delete the
  temporary directory because the two `DynamicLibrary` owners were still live. Explicitly
  dropping both handles before cleanup fixes the ownership error without changing generated code.
- Checked vector code laid out the common continuation after the cold status-return block without
  branch-frequency metadata. AArch64 consequently branched on every successful checked operation.
  The checked failure successor is now marked cold with branch weights `1:2000`, while the success
  block remains the lexical fall-through. This changes layout guidance only; the condition, status,
  and first-error semantics are unchanged.
- The x86 Rust `integer_cast` oracle used `_mm_cvtepi32_pd`, which interprets source lanes as signed
  `i32`. CK's source and C oracle accept the complete `u32` domain. The old timed corpus happened to
  contain only small values, hiding the semantic mismatch and giving Rust an unexpressible range
  precondition. The Rust oracle now uses an exact full-domain `u32`-to-`f64` conversion, and the
  differential audit includes both sides of the sign bit and `u32::MAX`.
- The fixed-vector KIR modular reduction horizontally folded every four x86 elements. Pinned
  candidate, replay, and Rust disassembly showed that the profitable x86 form instead carries
  vector accumulators and folds once at loop exit. x86 modular reductions therefore remain scalar
  KIR with a stable, audited Native-loop-vectorizer fallback. AArch64 retains the accepted explicit
  KIR reduction. The 20% proposer threshold and the runtime 90%/95% gates are unchanged.
- On the AArch64 worker, every `slp_quad` implementation—CK, C, and Rust—alternated between nearly
  exact 1x and 2x timings. Because the bimodality was shared by all three independently built
  artifacts, it is a worker frequency/scheduling effect rather than candidate-only instability.
  Each channel now runs the same unmeasured batch immediately before each timed sample of this
  four-element microkernel. Timed calls, rows, rotation, batch identity, upper median, and thresholds
  remain frozen.

## TDD closure

Each defect received a focused regression before its implementation change:

1. a structural ownership contract requires both dynamic libraries to be dropped before Windows
   cleanup;
2. Native LLVM IR requires the checked success block before the failure block and cold-failure
   branch weights;
3. the oracle contract forbids signed x86 conversion and requires the full-domain conversion
   constants, while the dynamic audit supplies high-bit inputs;
4. a synthetic x86 target-profile test requires the stable reduction fallback, and cross-target
   Native tests keep AArch64 explicit-vector expectations separate;
5. the benchmark contract requires equal short-kernel conditioning and pins it in the manifest.

## Acceptance boundary

- CK language semantics, public ABI 1, Runtime ABI 2, KIR schemas, safety modes, first-error rules,
  and strict floating-point behavior do not change.
- Vector and domain-fact thresholds remain per-kernel 90%, architecture geometric mean 95%, and
  domain-fact advantage 5%.
- The timed input corpus, warm-up rows, timed rows, seven timed calls, channel rotation, upper median,
  scalar replay identities, six-host matrix, and ten required jobs remain unchanged.
- The short-kernel conditioning batch is unmeasured, identical for CK/C/Rust, source-pinned, and
  explicit in the oracle manifest.
- Local tests cannot sign the two remote performance jobs. A new exact-SHA run must pass all ten
  jobs before CK 0.12 can be merged or called complete.

## Self-review

The fixes restore resource ownership, semantic parity, target-specific profitable lowering, and
stable cross-channel measurement without relaxing a gate. Every exception is narrow, named, and
machine-auditable. No old run may be reused for the new candidate SHA.

Verdict: the blocker is closed in design; final acceptance remains pending a new exact-SHA ten-job
workflow.
