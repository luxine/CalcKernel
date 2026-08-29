# CalcKernel 0.11 Performance Guide

[简体中文](../zh-CN/guides/performance.md)

The Native runtime contract compares optimized CK with the same source emitted
as C and compiled by the pinned Clang 22.1.8 oracle, and with the exact 0.10
compiler at commit `df816502876fba41676f9ebc190e4fadd18cd5a5`. Runs use O3,
strict floating-point behavior, identical target/CPU policy and modes, fixed
inputs, warm-up, batching, and measurement statistics.

For the geometric mean of accepted kernels, Native throughput must be at least
95% of the C oracle. No individual kernel may regress by more than 10%. Checked
and unchecked suites are reported and gated separately on controlled x86-64 and
AArch64 workers under the portable baseline CPU policy. Native-CPU measurements
remain available for investigation but are not compared with the frozen baseline.

Relative to the recorded 0.10 compiler, 0.11 runtime may regress at most 3%
geometrically and 8% for an individual case. A canonical proof-loop checked
suite must deliver at least 97% of unchecked throughput for the same workload.
KIR optimizer latency is gated against the 0.10 MIR optimizer: the suite median
ratio is at most 2x and every individual ratio at most 3x. Runtime throughput,
optimization latency, cold/warm run, memory, and artifact size are reported as
separate quantities.

Run the contract harness with:

```sh
cargo bench --bench ckc_perf
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
python3 scripts/check-native-performance.py target/ckc-perf/results.json
```

The strict machine-readable result is `target/ckc-perf/results.json`. Cases
live in `benches/cases/native-cases.tsv`, sources under `benches/fixtures`, and
the report contract in `benches/summary-schema.md`. The harness rejects semantic
mismatches before timing and records compiler, LLVM, OS, architecture, target,
CPU policy, mode, warm-up, sample, batching, and statistic identity.
The normative checker rejects any non-pinned identity or investigative CPU
policy and recomputes each reported upper median from its sample array.
The general compiler-stage summaries remain `build/perf/latest.summary.json`
and `build/perf/latest.summary.md`.

`benches/baselines/v0_10_compiler.toml` pins the 0.10 commit, compiler identity,
LLVM version, target/CPU/mode, harness/statistics identity, and SHA-256 of every
measured source. A digest or identity mismatch is a hard failure, not permission
to silently refresh a baseline.

The frozen 0.10 benchmark did not know the later proof-loop slice ABI. Baseline
capture therefore applies the checksum-pinned
`benches/baselines/v0_10_proof_loop_harness.patch` to the measurement harness
only; the compiler remains the exact pinned 0.10 commit. The patch supplies the
same deterministic input and call ABI used by the 0.11 harness.

A second checksum-pinned adapter,
`benches/baselines/v0_10_mir_optimizer_harness.patch`, measures only the frozen
0.10 MIR pass pipeline: parsing and MIR construction happen outside the timed
region. This matches the 0.11 KIR-only timing boundary and prevents frontend
`check` time from being mislabeled as the optimizer baseline.

A third checksum-pinned adapter,
`benches/baselines/v0_10_linux_cpp_runtime_harness.patch`, asks the selected C++
compiler for the absolute static `libstdc++.a` directory before Cargo links the
unchanged 0.10 compiler. This closes a hosted Ubuntu AArch64 search-path gap; it
does not alter CK source, IR, code generation, benchmark inputs, or timing.

Performance never permits changed diagnostics, evaluation order, modular
integer or strict floating semantics, checked first-error order, runtime print
order, semantic MIR, ABI, or contract domain. Generated contract cases contain
only inputs satisfying the declared domain. A benchmark, baseline, or threshold
change requires review as a contract change.
