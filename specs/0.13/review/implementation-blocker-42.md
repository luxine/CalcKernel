# Implementation blocker 42: stale PGO oracle workspace protocol

## Evidence

Exact-SHA V0.13 workflow-dispatch run `34178811720`, commit
`21448738b90ccfd1ea9ab79e9355450ef325769c`, failed both required performance
jobs before measurement. Linux x86-64 job `101913645601` and Linux AArch64 job
`101913645752` completed the unprofiled oracle audit, then failed the PGO oracle
audit with the same traceback:

```text
measurement.Kernel(library, case, record).result_digest()
AttributeError: 'dict' object has no attribute 'arguments'
```

No performance artifact existed because the failure preceded creation of
`target/ckc-perf`; the upload step consequently reported that no files were
found. All available run artifacts were capability or fact-audit evidence and
were unrelated to this failure.

## Root cause and rejected shortcuts

The preceding schema-8 sampling repair separated mutable workload storage into
`KernelWorkspace` and changed `Kernel` to require that workspace. The collector
and timed channels migrated to the new constructor protocol, but
`audit_pgo()` retained the old raw-record argument. The audit therefore failed
before exercising C, UBSan, and Rust differential results on every platform.

Skipping the PGO audit, tolerating an absent performance artifact, weakening
the oracle matrix, or rerunning unchanged code are rejected. None would repair
the incompatible call boundary.

## Repair

A structural regression was added first and observed failing. It requires the
PGO oracle audit to construct one `KernelWorkspace(case, record)` and pass that
same workspace to each C, UBSan, and Rust `Kernel` for the record. The audit now
uses that protocol, matching the measured-channel allocation contract and
preserving exact differential comparison on shared input/output addresses.

This repair changes no language or public ABI rule, safety or strict-FP
semantics, oracle source, target ISA, optimization policy, performance or
stability threshold, timed work, sample count, corpus, platform, or required
job matrix.

## Verification

The focused regression is rerun RED/GREEN, followed by the complete performance
contract tests, formatting, lint, release, repository, and exact-SHA remote
workflow gates. The replacement two-platform PGO oracle audit remains the
authoritative pinned-toolchain integration proof.
