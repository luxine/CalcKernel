# CalcKernel 0.10 Performance Guide

[简体中文](../zh-CN/guides/performance.md)

The Native runtime contract compares optimized CK with the same source emitted
as C and compiled by the pinned Clang 22.1.8 oracle. Both use O3, strict
floating-point behavior, the same target/CPU policy, checked mode, warm-up, and
measurement harness.

For the geometric mean of accepted kernels, Native throughput must be at least
95% of the C oracle. No individual kernel may regress by more than 10%. Checked
and unchecked suites are reported and gated separately on controlled x86-64 and
AArch64 workers, for baseline and native CPU policies. The gate compares runtime
only; compilation latency, cold/warm run, memory, and artifact size are reported
separately.

Run the contract harness with:

```sh
cargo bench --bench ckc_perf --features native-toolchain
```

The general summaries are `build/perf/latest.summary.json` and
`build/perf/latest.summary.md`; the strict Native comparison is
`target/ckc-perf/results.json`.

Cases live in `benches/cases/native-cases.tsv`, sources under
`benches/fixtures`, and the machine-readable contract in
`benches/summary-schema.md`. The CI harness uses repeated samples and medians,
batches kernels to suppress process/FFI noise, records compiler/OS/architecture
identity, and rejects mismatched semantics before timing.

Performance never permits changed diagnostics, evaluation order, integer or
strict floating semantics, checked first-error order, runtime print order, MIR,
or ABI. A benchmark or threshold change requires review as a contract change.
