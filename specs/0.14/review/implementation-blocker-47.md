# Implementation blocker 47: inherit the V0.13 checked constant-map closure

## Evidence

Exact-SHA V0.13 workflow-dispatch run `34155662442`, commit
`6fd8234859dfe667419b7be9e601ad79426fd2dd`, exposed two actionable defects:

- Linux and Darwin ARM64 Native jobs `101846838287` and `101846838311` failed
  `compact_multiversion_helper_policy_should_survive_o3_before_symbol_stripping`.
- x86-64 performance job `101846838388` measured checked `specialized_length` at
  `4,860,459 ns` versus the faster `4,052,972 ns` Rust SIMD oracle, below the
  unchanged 90% throughput floor.

Exact-SHA V0.14 run `34155664658`, commit
`1c588a0b62ac981983703b7f7d7029f50d23cba8`, reproduced the Native failure in
Linux and Darwin ARM64 jobs `101847099013` and `101847098917`. The complete
diagnostic artifacts showed an ordinary Rust assertion failure rather than a bridge crash.

## Diagnosis and inherited repair

The detailed implementation diagnosis is recorded in
`specs/0.13/review/implementation-blocker-38.md`. The regression invoked ordinary KIR O3,
which correctly uses the larger ordinary inline budget and therefore destroyed the large
helper before the multiversion retention policy could be tested. Independently, blocker 37's
streaming-map repair attached `llvm.loop.unroll.disable` to every checked scalar memory map,
including an internal map whose bound is constant at every direct call.

V0.14 inherits exact V0.13 commit
`e869763366283e46cd76ffbf3bb85c6c3959c25c`:

- the Native regression enters through the production multiversion KIR pipeline and proves the
  compact helper is inlined while the large helper remains before LLVM lowering;
- checked constant-call maps use the existing bounded two-way schedule;
- unknown-length checked streaming maps continue to disable harmful unrolling;
- ordinary and multiversion object-cache identities advance to
  `x86-checked-memory-map-schedule-v2`.

The V0.13 replay manifest pins the exact repaired commit and compiler identity; its SHA-256 is
`4e9b37ae4687fa5f11c3da029e57fd3e1e6bd9512a2b66bd8599de9fd2337c3d`.

No language or public ABI rule, overflow/bounds behavior, strict-FP semantic, target ISA,
inline/growth budget, performance/stability/artifact-size threshold, timed work, sample count,
corpus, platform, schema-9 required tier, or required job changes.

## Acceptance

Focused structural and Native regressions must pass in V0.14, followed by the complete local
no-native/all-feature gates available on the host. A replacement exact-SHA V0.14 workflow must
rebuild the repaired V0.13 revision and independently pass all ten unchanged jobs. V0.13 keeps
its own independent exact-SHA acceptance; V0.14 cannot sign for it.
