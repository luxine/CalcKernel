# Implementation blocker 35: a v4 host cannot retain its AVX-512 member

## Evidence

Exact-SHA V0.13 workflow-dispatch run `34117792378`, commit
`dd6239e677720845dee874ac3095710941b58d5b`, failed x86-64 performance job
`101728726666` in the cumulative schema 7/schema 8 gate. All job logs and the
complete available artifact set were downloaded before diagnosis. Schema 8
reported:

```text
native performance gate failed: compute-bound CK throughput is below 90% of its PGO oracle
```

The stable compute-bound samples measured combined CK at 24,126 ns against the
17,797 ns Clang and 17,836 ns Rust PGO oracles. Disassembly showed that both
oracles use four AVX-512 ZMM chains, while the selected CK member uses four AVX2
YMM chains. The worker capability manifest proves x86-64-v4, including AVX-512F,
BW, DQ, and VL, is available.

## Root cause and rejected shortcuts

The resolver is correct: it selects the first compatible compiler-ranked
member. Coverage-first ordering intentionally retains x86-64-v3 before v4 so a
v3 host does not fall back to SSE2. However, the shared growth ledger charged a
complete full-root KIR clone to every target profile. A one-root module therefore
exhausted its unchanged baseline-sized additional KIR budget after v3 and could
never retain the profitable v4 member, even though the two logical KIR bodies
differ only in target profile and tier-derived hidden names. Choosing v4 first
would regress required v3 runners; increasing the `2x` logical growth ceiling,
lowering the 90% performance gate, reducing work, or changing the platform
matrix are rejected.

## Repair

A regression test first required the fixture to retain both ranked x86-64-v3
and x86-64-v4 members while charging their exact normalized KIR body once; it
failed with one retained member before the implementation changed.

The planner now normalizes only the target profile and the verified tier-derived
hidden-name mapping, then compares the complete printed KIR bytes. Byte-identical
normalized bodies for the same root share one logical body charge. Any
instruction, CFG, ABI, call mapping, or other structural difference still pays
its full KIR units. The independent checker reconstructs the same decision from
immutable inputs. Every target keeps its separate profile/proof/feature/codegen
digest, LLVM module, object, audit, cache identity, and physical artifact-size
gate; cross-variant LTO remains forbidden. The object-affecting cache contract is
advanced with `shared-target-neutral-variant-budget-v1`.

This repair changes no language or public ABI rule, safety or strict-FP
semantics, target ISA, candidate frontier, profitability floor, performance or
stability threshold, timed work, sample count, corpus, platform, or required job
matrix.

## Verification

The focused optimizer regression is green. The complete no-native Rust suite,
all 21 independent Python performance-checker tests, formatting, and
warning-denying all-target Clippy are green locally. The replacement exact-SHA
workflow-dispatch run provides the authoritative native and performance
verification on the pinned runner/toolchain matrix.
