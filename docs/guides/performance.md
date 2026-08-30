# CalcKernel 0.11 Performance Guide

[简体中文](../zh-CN/guides/performance.md)

The Native runtime contract compares optimized CK with the same source emitted
as C and compiled by the pinned Clang 22.1.8 oracle, and with the exact 0.10
compiler at commit `df816502876fba41676f9ebc190e4fadd18cd5a5`. Runs use O3,
strict floating-point behavior, identical target/CPU policy and modes, fixed
inputs, warm-up, batching, and measurement statistics.

For the geometric mean of accepted kernels, Native throughput must be at least
95% of the C oracle. No individual kernel may regress by more than 10%. Checked
and unchecked suites are reported and gated separately on x86-64 and AArch64
workers under the portable baseline CPU policy. Each run compiles the
digest-pinned C oracle emitted by the exact 0.10 compiler with the same Clang
22.1.8 process used to calibrate the candidate. Native-CPU policy is not part of
this baseline replay protocol and cannot satisfy the release comparison.

Relative to the exact pinned 0.10 compiler, the gate compares
`(T0.11-Native / Tcurrent-Clang) / (T0.10-Native / T0.10-Clang)` and permits at
most 3% geometric and 8% individual regression. All four terms are sampled in
the same process on the same worker, with independently built pinned 0.10 Native
libraries and the exact frozen 0.10 C oracle. Clang normalization alone does not
remove arbitrary CPU-generation differences. Historical medians remain unchanged
provenance; actual same-process replay samples supply the comparison denominator.
Both safety modes and compiler versions use a deterministic eight-channel rotating
schedule on identical inputs. A canonical proof-loop checked suite must deliver
at least 97% of unchecked throughput; this raw candidate ratio is not normalized.
KIR optimizer latency is gated against the 0.10 MIR optimizer: the suite median
ratio is at most 2x and every individual ratio at most 3x. Runtime throughput,
optimization latency, cold/warm run, memory, and artifact size are reported as
separate quantities.

Run the contract harness with:

```sh
cargo bench --bench ckc_perf
# Set CKC_LLVM_PREFIX and CKC_CLANG_ORACLE to the pinned LLVM/Clang installation.
python3 scripts/prepare-performance-replay.py --out target/ckc-perf/v010-replay
export CKC_V010_RUNTIME_BUNDLE="$PWD/target/ckc-perf/v010-replay"
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
python3 scripts/check-native-performance.py target/ckc-perf/results.json
```

Preparation requires a new output directory and builds the pinned compiler in an
owned local clone, without changing main or an existing baseline worktree. It
applies only the four frozen adapters below, validates source/toolchain identity,
and hashes the actual compiler and eight libraries. Reuse an intact bundle only
with the same preparation/replay recipe; choose a new output directory when that
recipe changes. Missing or modified replay evidence is a hard error.
The development workflow requires Python 3.11+, Git history containing the fixed
commit, Rust 1.90.0, and the pinned LLVM/Clang 22.1.8 installation. CI fetches full
history before preparing the local clone. The checker also requires
`CKC_LLVM_PREFIX` to verify the installed component-manifest digest.

Retain the report-relative `target/ckc-perf/measurement-<pid>-<timestamp>` directory
alongside the report, plus the selected replay bundle's `ckc-v010`, eight libraries,
`replay.tsv` and `preparation.log`. Both candidate modes and the two Clang copies
are hashed before/after measurement. The exact schedule and all four sample arrays
per mode are recorded; moving a report alone loses required evidence.

The strict schema-6 result is `target/ckc-perf/results.json`. Cases
live in `benches/cases/native-cases.tsv`, sources under `benches/fixtures`, and
the report contract in `benches/summary-schema.md`. The harness rejects semantic
mismatches before timing and records compiler, LLVM, OS, architecture, target,
CPU policy, mode, warm-up, sample, batching, and statistic identity.
The normative checker rejects any non-pinned identity or investigative CPU
policy, verifies historical paired V0.10 medians against the schema-2 baseline
manifest, validates the replay bundle and measured artifact hashes, and recomputes
all candidate/replay upper medians from their stable sample arrays. It requires the
exact interleaving schedule, three warm-up rounds, twenty samples, seven calls per
sample and twenty-million-input batches. Quick measurements cannot pass this gate.
The general compiler-stage summaries remain `build/perf/latest.summary.json`
and `build/perf/latest.summary.md`.

`benches/baselines/v0_10_compiler.toml` pins the 0.10 commit, compiler identity,
LLVM version, target/CPU/mode, paired Native/Clang medians, harness/statistics
identity, and SHA-256 of every measured CK and frozen C-oracle source. A digest
or identity mismatch is a hard failure, not permission to silently refresh a
baseline.

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

The checksum-pinned `benches/baselines/v0_10_clang_cpu_harness.patch` makes the
portable Clang reference architecture-specific: x86-64 uses
`-march=x86-64 -mtune=generic`, matching CK's `x86-64` baseline, while AArch64
keeps `-mcpu=generic`, matching CK's generic ARMv8-A baseline. Native-CPU flags
remain investigative and are never accepted by the release gate.

Performance never permits changed diagnostics, evaluation order, modular
integer or strict floating semantics, checked first-error order, runtime print
order, semantic MIR, ABI, or contract domain. Generated contract cases contain
only inputs satisfying the declared domain. A benchmark, baseline, or threshold
change requires review as a contract change.

CI prepares the replay bundle before either architecture's full performance gate
and retains the first report, build provenance and actual nonempty measured library
files with their hashes. Failure-only diagnostics retain CPU identity and inspect
those same artifacts; an empty extracted section is never machine-code evidence.
The same diagnostics can be requested explicitly with the workflow-dispatch
`performance_diagnostics` input to investigate without waiting for another failure.
These artifacts do not replace the original gate or authorize refreshing the
frozen baseline; an original required-job failure remains failed.
