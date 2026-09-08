# Implementation blocker 56: inherit full-width V0.13 v4 compute lowering

## Evidence

V0.14 exact-SHA workflow-dispatch run `34212249513`, commit
`b829284fdcfd635db86401a2bc908de069e9f027`, independently rebuilt accepted
V0.13 commit `6258089cf44ebc317247e3fc38e2c765e424132e` on an AMD EPYC 9V74 worker with
the complete `x86-64-v4` capability set. The unchanged schema-8 compute-bound
gate measured combined CK at 34,832 ns and the Rust PGO oracle at 30,025 ns,
about 86.2% oracle throughput against the unchanged 90% minimum. Twenty samples
were complete and stable. The retained V0.13 v4 object selected the correct v4
member but used four 256-bit YMM chains, while the Rust oracle used four 512-bit
ZMM chains.

The same accepted V0.13 SHA passed its independent x86 performance job on a
v3-only worker, where the v4-only lowering defect could not execute. This makes
the V0.14 replay failure actionable dependency evidence rather than a substitute
for independent V0.13 acceptance.

## Repair

V0.14 exactly inherits V0.13 blocker 45. Only an explicit `x86-64-v4` target
with `+avx512f` may receive vector width eight, and only after an independent
analysis clone identifies a scalar memory map with non-local load and store, at
least eight strict scalar `f64` operations, no fast-math operation, and no
existing schedule. All other loops keep LLVM's normal target cost decision.
The multiversion object-cache codegen identity advances with the repair.

The accepted V0.13 base and independent replay are repinned to exact commit
`e4f3fc6388a3ebd15cb1beb4ecd4dd9ca55b0dbe`. The updated replay manifest SHA-256
is `1de279a1972fcad7a9447d8e7281dd86aa9faf49fd514c181ffa82936294fa20`.

No language or public ABI, strict-FP or safety semantic, target eligibility,
profile/tuning/report schema, performance/stability/size threshold, timed work,
sample count, corpus, platform, or required job changes.

## Acceptance

The focused contract, formatting, lint, complete locked tests, release native
build, ordinary/PGO/tune oracle audits, and replacement exact-SHA ten-job V0.14
workflow must pass. V0.13 retains its own independent replacement ten-job
workflow and may not be accepted through V0.14 replay alone.
