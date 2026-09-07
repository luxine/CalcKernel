# Implementation blocker 45: exact V0.13 replay exceeds the compile gate

## Evidence

Exact-SHA workflow-dispatch run `34123758500`, V0.14 commit
`02dcd8a3b1afceac01c5f104d33b0eaaa034d632`, failed AArch64 performance job
`101747821716` while preparing exact V0.13 replay. The complete failed-job log
and every artifact available from the run were downloaded before diagnosis.
The historical schema-8 report measured the multiversion/ordinary
source-to-object geometric ratio at `2.5245647`, above the unchanged `2.5`
limit. Branch-layout, call-constant-length, and memory-bound individually
measured about `2.7588x`, `2.7505x`, and `2.5742x`; artifact-size gates remained
within their existing limits.

## Diagnosis and rejected shortcuts

V0.13 retained both required target-neutral companion members after blocker 35,
but compared normalized KIR bodies by serializing complete modules and rebuilt
the same independently checked bundle in both the CLI and native emitter. The
small stable aggregate excess is therefore actionable compile work, not a
reason to retry or relax the threshold.

The same review found that coverage-first retained-set storage order was copied
directly into the runtime dispatch order. First-compatible selection could then
choose v3/SVE on a host that also supports the faster retained v4/SVE2 member.
Lowering the compile gate, removing a required member, reducing timed work or
samples, weakening the checker, or using V0.14 as a substitute for independent
V0.13 acceptance are rejected.

## Repair and propagation

The V0.13 repair at `7b883bf36a2edfb6720caa69aa7f10c94ebb9e43` was first
proved by RED/GREEN structural and dispatch-order regressions, then integrated
into V0.14. Normalized target-neutral bodies use complete structural KIR
equality without serialization. The CLI retains one opaque authority borrowing
the exact checked request and proposal through native emission; the raw public
emitter still independently checks arbitrary bundles and fails closed.

Retention stays coverage-first under the unchanged growth budget. Runtime
dispatch independently sorts retained members by predicted dynamic cost,
compatibility breadth, KIR size, tier identity, and root identity before
baseline. Cache identity includes `performance-first-dispatch-ranking-v1`.

The V0.14 replay manifest now pins exact V0.13
`7b883bf36a2edfb6720caa69aa7f10c94ebb9e43`; its SHA-256 is
`9131f9e83e96a81abbae7200a3afef054822442eb037b2238d1cadec7aec7ead`.
This propagation changes no language or public ABI rule, safety or strict-FP
semantics, target ISA, growth or profitability rule, performance or stability
threshold, timed work, sample count, corpus, platform, or required job matrix.

## Verification

V0.14 reruns the focused replay/schema contract and multiversion regressions,
the complete local no-native Rust and Python checker suites, release smoke, and
the replacement exact-SHA ten-job workflow. Native and performance acceptance
remains authoritative only for the exact pinned runner/toolchain result.
