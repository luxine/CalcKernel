# Implementation blocker 46: inherit the V0.13 remote portability closure

## Evidence

Exact-SHA workflow-dispatch run `34133617471`, V0.14 commit
`2be3d84cfa546d438d72900885c3fc161b547be0`, reproduced the actionable V0.13
failures while independently rebuilding commit
`7b883bf36a2edfb6720caa69aa7f10c94ebb9e43`:

- native integration job `101779914294` failed the same all-feature Clippy
  unused binding;
- AArch64 performance job `101779914299` correctly rejected the pinned V0.13
  replay's aggregate artifact-size failure;
- Windows ARM64 job `101779914398` reproduced both the stripped-PE helper-test
  false negative and unresolved `_Interlocked*` profile-runtime references.

The x86-64 performance job `101779914305` separately reached schema 9 after the
cumulative gates and rejected its allocated AMD EPYC 7763 host because it
provides x86-64-v3 but not the normative x86-64-v4 AVX-512 feature set. This is
a truthful required-capability failure: it does not authorize changing schema
9, accepting v3 evidence, skipping the job, or emulating timed v4 execution.

## Diagnosis and inherited repair

The implementation causes and rejected shortcuts are recorded in
`specs/0.13/review/implementation-blocker-37.md`. V0.14 inherits exact V0.13
commit `6fd8234859dfe667419b7be9e601ad79426fd2dd`:

- checked x86 streaming maps disable the harmful LLVM unroll while other
  checked scalar loops keep their bounded schedule;
- ordinary and multiversion Native caches bind the new object-affecting policy;
- Unix private runtime objects omit non-executable compiler-ident sections;
- helper retention is asserted in optimized IR before platform link stripping,
  while stripped Windows products are checked through their public export table;
- Windows ARM64 profile atomics use the already embedded kernel32 import closure,
  retaining MSVC intrinsic expansion on x64;
- the stale checked-emitter request binding is removed.

V0.14 preserves its schema-5 cache, tune runtime names, hashed provenance, and
all offline-autotuning-specific contracts while applying the same behavior. The
V0.13 replay manifest now pins the exact repaired commit and compiler identity;
its SHA-256 is
`138f4fe15331698f8b14c1b5fba56057d4935dfbbe1948fd5f22f935ee932932`.

No language or public ABI rule, checked-overflow or bounds behavior, strict-FP
semantic, target ISA, performance/stability/artifact-size threshold, timed work,
sample count, corpus, platform, schema-9 required tier, or required job changes.

## Acceptance

Focused structural tests and the complete local no-native suite are rerun after
the mechanical propagation. A replacement exact-SHA V0.14 workflow must rebuild
the repaired V0.13 revision, verify Native Windows and LLVM behavior, pass all
unchanged cumulative/performance gates, and obtain a real x86-64-v4 stable worker
for schema 9. A v3 worker remains a hard failure rather than an accepted result.
