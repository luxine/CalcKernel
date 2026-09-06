# Implementation blocker 24: lower-tier coverage and AArch64 dynamic-library detection

Date: 2026-09-06

## Finding

V0.14 exact replay run `34038295553` rebuilt V0.13 commit
`4a04fb34eb0f1358d0f8fa308f95d031954e72b0` and failed both schema-8
performance jobs while their sample streams remained stable:

- x86-64 `compute-bound` measured ordinary 70,752 ns, combined 70,773 ns,
  selected-direct 71,144 ns, Clang PGO 42,930 ns, and Rust PGO 42,800 ns;
- AArch64 dispatch geometric-mean improvement was about 1.003 rather than the
  unchanged required 1.08.

No evidence supports changing a performance/stability threshold, workload,
sample count, statistic, target platform, or required CI job.

## Rediagnosis

The x86 shared object contained only an `x86-64-v4` enhanced member even though
the stable worker exposed x86-64-v3. Disassembly showed that member's main loop
was materially identical to the Clang PGO AVX2 loop, while the resolver correctly
selected the SSE2 baseline because AVX-512 was unavailable. The prior
coverage-first ordering operated only after the isolated per-tier profitability
filter; it could not retain v3 when v4 passed the fixed floor and v3 was predicted
non-regressing but below that floor.

The AArch64 shared objects contained valid SVE members, but both dispatch and the
exact selected-direct channel executed baseline implementations. Linux
executables capture HWCAP/HWCAP2 from their CK startup stack. A dynamic library
loaded by the Python performance harness has no CK entry point, so the private
runtime never received that snapshot and deliberately reported baseline. The
collector's `/proc/cpuinfo` capability manifest therefore described the host but
not the implementation actually selected by the artifact.

## Resolution

Multiversion root eligibility still requires a legal tier to pass the unchanged
10-percent and two-unit profitability floor. When that profitable tier has a
strict required-feature subset whose predicted cost is no worse than baseline,
the subset may enter the bounded retained set as a compatibility companion. It
is independently verified and feature-audited and does not establish safety or
profitability. Compatibility breadth remains the first retained-set key, so a
one-full-root budget can materialize the v3/SVE member usable by the required
worker. A RED/GREEN unit regression rejects a regressing, equal-feature, or
non-subset companion. The cache identity adds
`coverage-companion-profitability-v1`.

The freestanding AArch64 Linux dispatch runtime now retains startup-stack auxv
as the executable fast path and adds a dynamic-library fallback that reads
binary `/proc/self/auxv` records through direct `openat`, `read`, and `close`
system calls. Missing, partial, or malformed data still selects baseline. The
fallback imports no libc, loader, allocator, environment, network, or LLVM
runtime symbol. A source-contract RED/GREEN test and an AArch64 cross syntax
check cover the new path; the required real AArch64 performance job remains the
selection and throughput authority.

No CK language, public ABI, strict floating rule, safety rule, target ISA,
performance/stability gate, workload, timed region, sample count, corpus,
platform, or required job changed.

## Verdict

Accepted implementation blocker. Focused local regressions and complete local
gates must pass before an exact-SHA rerun. Both exact x86-64 and AArch64
performance jobs remain required for final acceptance.
