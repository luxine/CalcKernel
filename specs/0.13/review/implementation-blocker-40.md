# Implementation blocker 40: vector oracle channels must share one data workspace

## Evidence

V0.14 exact-SHA workflow-dispatch run `34169415571`, commit
`5dae5b35664fb36d9e7435fc46f336895627b1e7`, rebuilt exact V0.13
`aa757ca0f78664cbfa4f824d655d820c87368dd3` and failed x86-64 performance job
`101886905457`. The checked `strict_f64` candidate measured `3,601,329 ns` against the faster
Rust SIMD oracle's `3,164,344 ns`, or about `87.87%`, below the unchanged 90% individual
throughput floor. All twenty retained rows were stable and used the complete
`20,000,000`-operation batch, three warmup rows, seven interleaved raw calls per channel, and
unchanged corpus.

This was not a generated-code regression. The preceding exact artifact from run
`34165564989` measured `3,194,955 ns` versus `3,191,369 ns`, while the candidate shared object
was byte-identical in both runs with SHA-256
`20825bfb2d2215d770da040e3d2e048e3fa577ffbc6ef6199d1aa6c20851cd22`. The Rust oracle was
also byte-identical with SHA-256
`f47cf120eb6d96da2dbba8742565a29de8d1320616da7a0a5f4cf3262f4a1965`. Disassembly of the
candidate `kernel` was identical, and both reports used the same AMD EPYC 7763 CPU model,
LLVM/Clang 22.1.8, Rust 1.90.0, and schema-7 recipe identity.

## Root cause and rejected shortcuts

The three candidate/C/Rust `KernelRunner` instances each allocated independent input and output
vectors. The rotating call order balanced temporal position but still compared code using three
different allocation placements. Pointer alignment, physical-page placement, and cache-set
mapping could therefore create a persistent per-channel difference within one process; the
existing stability check correctly found each contaminated channel stable but could not identify
that cross-channel confounder. The byte-identical pass/fail artifacts expose this harness defect.

Rerunning until a favorable placement appears, widening the 90% floor, reducing timed work or
sample rows, dropping the case, or changing CK strict-float semantics are rejected.

## Repair

The harness now keeps all three dynamic libraries alive but extracts their typed entry points and
executes every channel against one shared `KernelWorkspace`. Equivalence, warmup, and retained
measurements therefore see the same input/output addresses. The existing rotating order still
balances cache warmth and temporal position, and the complete seven-call upper-median row,
twenty retained rows, batch work, correctness digest, and fail-fast behavior remain unchanged.

The oracle sampling contract advances to
`interleaved-upper-median-three-channel-v3`; the manifest SHA-256 advances to
`e4e8e4e70893a81cb96f8d7e0e5dbc1e5f971236ee88b3d0b2e2c55fdda854b3`. Harness, checker,
tests, benchmark schema, and bilingual performance guidance bind the same identity.

No language or public ABI rule, strict-FP behavior, target ISA, optimization policy,
performance/stability/artifact-size threshold, timed work, repetition count, sample count,
corpus, platform, or required job changes.

## Acceptance

A focused regression must first fail while `measure_case` invokes separate per-channel runner
buffers, then pass only when every entry uses one shared workspace. A second RED/GREEN contract
binds protocol v3 across harness, manifest, schema, and bilingual guidance. Formatting, lint,
Rust/Python performance tests, Native compilation, and relevant local gates are rerun. New exact
V0.13 and V0.14 ten-job workflows remain authoritative for the stable x86/AArch64 measurements.
