# Implementation blocker 49: historical vector replay must remove allocation-placement bias

## Evidence

V0.14 exact-SHA workflow-dispatch run `34169415571`, commit
`5dae5b35664fb36d9e7435fc46f336895627b1e7`, failed x86-64 performance job
`101886905457` while preparing the exact V0.13 historical replay. The checked
`strict_f64` candidate measured `3,601,329 ns` against the faster Rust SIMD oracle's
`3,164,344 ns`, or about `87.87%`, below the unchanged 90% individual throughput
floor. The job retained all twenty stable sample rows with the full 20,000,000-operation
batch, three warmup rows, seven interleaved raw calls per channel, and unchanged corpus.

The preceding exact artifact measured `3,194,955 / 3,191,369 ns`. Candidate and Rust
oracle shared objects were byte-identical between the passing and failing artifacts,
with SHA-256 values `20825bfb2d2215d770da040e3d2e048e3fa577ffbc6ef6199d1aa6c20851cd22`
and `f47cf120eb6d96da2dbba8742565a29de8d1320616da7a0a5f4cf3262f4a1965`;
candidate disassembly was also identical. Both jobs used the same AMD EPYC 7763 CPU,
LLVM/Clang 22.1.8, Rust 1.90.0, and schema-7 recipe identity.

## Root cause

The inherited schema-7 harness created a separate `KernelRunner` data allocation for
candidate, C, and Rust. Rotating call order balanced temporal position, but pointer
alignment, physical-page placement, and cache-set mapping remained permanently tied to
each channel. A contaminated channel could therefore remain internally stable while
cross-channel comparison measured allocation placement as if it were generated-code
performance.

Rerunning until allocation placement is favorable, relaxing the throughput floor,
reducing timed work or samples, dropping `strict_f64`, or changing strict-FP semantics
are rejected.

## Repair and replay identity

V0.13 commit `1aad5bdd964f3afa4b367434c1c3810fb63f8e8f` keeps each dynamic library alive,
extracts its typed entry, and runs candidate/C/Rust equivalence, warmup, and timed calls
through one shared `KernelWorkspace`. The protocol advances to
`interleaved-upper-median-three-channel-v3`, with oracle manifest SHA-256
`e4e8e4e70893a81cb96f8d7e0e5dbc1e5f971236ee88b3d0b2e2c55fdda854b3`.
V0.14 inherits that exact harness closure in commit
`f416f9e9cb276c9a21e65bf6c14664480d2bbae2` and pins its historical replay to the
new V0.13 commit. The resulting `benches/baselines/v0_13_replay.toml` SHA-256 is
`578868dabbba1a10267c1500269fe75b1e953a3ef913ba71612257c196478fdb`.

No language or public ABI rule, strict-FP behavior, target ISA, optimization policy,
performance/stability/artifact-size threshold, timed work, repetition count, sample
count, corpus, platform, or required job changes.

## Acceptance

The V0.14 shared-workspace regression was observed RED against the old harness and GREEN
after inheritance. Formatting, lint, Rust/Python performance tests, Native compilation,
and the complete local test suite are rerun. A new exact-SHA ten-job V0.14 workflow must
rebuild and validate historical V0.13 commit
`1aad5bdd964f3afa4b367434c1c3810fb63f8e8f`; no old run or moving branch may substitute.
