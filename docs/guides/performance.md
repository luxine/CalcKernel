# CalcKernel Performance Guide

[简体中文](../zh-CN/guides/performance.md)

Use `cargo bench --bench ckc_perf` for the native benchmark harness. Cases are
declared in `bench/perf/cases/native-cases.tsv` until the repository-layout task
moves them to `benches/cases/native-cases.tsv`; fixtures and the JSON summary
schema live beside that manifest. (These paths are updated atomically with the
move.)

Benchmark lexer/parser/checker, MIR lowering/optimization, backend emission,
toolchain build time, artifact size, and runtime separately. Warm up external
tools, record compiler/OS/architecture/toolchain versions, use multiple samples,
and compare medians rather than a single run. Runtime kernels should batch work
to avoid measuring FFI call overhead as compiler performance.

Compare O0–O3 and all supported C mode combinations. Checked modes intentionally
add branches and status propagation; report them separately from unchecked
throughput. WASM/LLVM have no checked-mode counterpart.

Performance changes are accepted only with semantic tests and a reproducible
summary. A faster result never justifies altered diagnostics, evaluation order,
integer/floating semantics, checked error order, MIR, or ABI.
