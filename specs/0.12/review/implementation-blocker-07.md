# Implementation blocker 07: x86 reduction discovery mutated unrelated O3 loops

## Failure

The exact x86-64 remote performance gate for candidate `02d7a5e7a516415892e947048c533ff25875dd54`
failed the unchanged V0.11 replay limit for `scalar unchecked/integer_accumulate`:

- candidate Native upper median: `26,427,902 ns`;
- exact V0.11 replay Native upper median: `23,009,295 ns`;
- regression: approximately `14.86%`, above the frozen `8%` limit.

The twenty samples in each channel were tightly clustered. Candidate and replay Clang copies were
also effectively identical, so the failure was neither worker noise nor calibration drift.

## Root cause

The x86 integer-memory-reduction handoff introduced after the first cross-host acceptance run
called `PromoteMemToReg` directly on every production O3 function before deciding whether the
function contained the target reduction. `integer_accumulate` is not an integer memory reduction,
but this unconditional early canonicalization changed the LLVM pipeline's recurrence shape on
x86-64. The retained disassembly showed an extra per-iteration `or`/increment recurrence in the
candidate, while the exact V0.11 replay retained two independent add recurrences.

On AArch64 the handoff returns before the early promotion, and the candidate and V0.11 optimized
LLVM IR are byte-identical for the same fixture. This isolates the regression to the x86-only
pre-pass mutation rather than CK semantics, the KIR optimizer, or the pinned LLVM version.

## Correction

Reduction discovery now:

1. skips functions without a possible non-local load;
2. clones each remaining function;
3. runs the temporary `mem2reg` and loop classification only on that detached clone; and
4. maps a proven clone loop back to the corresponding production loop and attaches only the
   required `llvm.loop.interleave.count = 8` metadata.

The production function is otherwise untouched before LLVM's standard O3 pipeline. A structural
regression requires clone-based discovery and forbids the prior production-module promotion.

## Acceptance boundary

The x86 modular integer memory-reduction handoff and its fixed interleave width remain in force.
No language, ABI, safety, corpus, sampling, or performance threshold changed. Remote x86-64
performance CI remains the authoritative dynamic confirmation of the correction.
