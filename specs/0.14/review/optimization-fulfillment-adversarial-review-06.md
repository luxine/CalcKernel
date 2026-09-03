# CK 0.14 Optimization-Fulfilment Adversarial Review 06

Review target: commit `1f27df4b7992f1209f6762aeb11632509d888ae0`

Reviewer: fresh read-only gpt-5.6-sol, xhigh reasoning

Verdict: PASS

Blockers: 0

## Review loop and rediagnosis

Six independent review rounds were completed. Every blocking report was
rediagnosed against the repository before revision; no performance, correctness,
ABI, evidence, or CI threshold was weakened.

1. The original fulfilment design lacked a closed independent performance report
   and allowed the older implementation plan to bypass the new gate. The design
   gained `predicated-update-performance-1.md`, and the old plan was suspended.
2. The new report incorrectly encoded a 40-character Git SHA as a SHA-256 digest,
   compared executable bytes with an argv string, and could not identify the PGO
   shard directory. The order seed now uses `Text`; executable identity uses
   no-follow path resolution; directory snapshots bind the sole shard consumed by
   merge.
3. The report incorrectly required ordinary replay to create tuning-publication
   locks and lacked cold-cache evidence. Locks now cover only `pgoTuned`; four
   command-bound cache-scratch records retain create-new empty pre-state and live
   post-state evidence.
4. Cache evidence confused Linux `XDG_CACHE_HOME` with CK's mapped namespace. The
   environment base is now `cache/<command>` and the recorded namespace is
   `cache/<command>/ckc`.
5. A compound selected plan could attribute another choice's speedup to an
   unreachable predicated update. The gate now accepts exactly one PlanChoice,
   exactly one target SiteAlternative, `minimum <= 128`, true fixed-input guards,
   and at least one executed vector chunk in every split.
6. The final fresh review found no remaining blocker.

## Confirmed closures

- Strict-f64 compare/select/unmasked-store semantics, same-place dominating load,
  Memory SSA, alias/dependence, checked first-failure, fallback, and independent
  vector checking form a closed legality contract.
- Existing Loop SIMD payload and Decision Schema 1 carry the required VF, UF,
  threshold, plan, unit, variant, site, and post-state identities; no `CKTUNE01`,
  language, Native ABI, or Runtime ABI change is required.
- PGO-only and tuned channels bind the same compiler, source, immutable profile,
  target, CPU, modes, and LLVM pipeline. Their only policy difference is the sole
  attested choice.
- Frozen source/generator/result digests, exact build graph, sampling order,
  stability rules, 2% validation ceiling, 5% release gain, evidence inventory,
  publication locks, and cache snapshots are independently checkable.
- Windows atomics, Unix profile publication, host artifact naming, LLVM void calls,
  six-host native coverage, and two-host performance coverage remain release
  blockers without changing the ten-job topology.
- The previous implementation control and final acceptance remained suspended
  during review and could not authorize implementation.

The reviewer verified the exact target commit and a clean worktree and made no
edits.
