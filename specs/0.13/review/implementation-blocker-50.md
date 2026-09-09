# Implementation blocker 50: bound full-width integer scalar tails

Exact V0.13 `4add225778b867e33227236d178de33139a97d36` run `34305409171`
failed schema 8 on Xeon 8573C at memory-bound combined/faster =
26,613 / 24,885 = 1.0694394213. Its ordinary and PGO artifacts are byte-identical,
but their medians differ by about 8%, so that worker also shows sampling drift.
The full failed cohort is retained, not replaced by a selected rerun.

V0.14 `d964d7ddbfd234cf87bf462489214695187611e7` run `34307207415` independently
replayed the same V0.13 SHA on Xeon 6973P-C. Its stable memory-bound samples
failed dispatch/ordinary = 20,954 / 20,337 = 1.0303387914. Selected-direct was
20,963 ns, locating the regression in the selected body rather than the warmed
dispatch thunk. Both jobs produced the same exact CK artifact hashes.

## Root cause and minimal repair

The newly authorized 16-lane integer vector width combined with LLVM's default
four-way interleave. Retained AVX-512 objects process 64 elements per vector-loop
iteration and run a purely scalar remainder, leaving up to 63 scalar elements.
Complete 16-lane vectors can therefore fall into the scalar path. The ordinary
KIR-vectorized body processes 16 elements per chunk instead.

The existing restricted v4 wrapping-integer map rule now requests interleave 1
as well as width 16. No input length, fixture name, profile outcome, CPU model,
or sample statistic selects this rule. Existing safety exclusions and LLVM
legality are preserved; baseline, v3, strict-f64, checked, atomic/volatile,
call-bearing, already-vectorized and already-scheduled loops are unchanged.
The multiversion codegen identity adds a separate tail-schedule revision.

## Verification boundary

A source-contract regression failed before the repair and passed afterward.
The native exact-tier regression now covers both one-input map and two-input zip
and requires the v4 trip-count round-down to 16, while retaining baseline/v3
width assertions and object emission. Native x86 execution remains required
on the remote matrix because the local pinned prefix only includes AArch64.

An independent LLVM 22.1.8 x86 IR diagnostic reproduced the 64-element trip
round-down. Changing only interleave metadata to 1 changed that round-down and
vector induction step to 16. This proves the scheduling mechanism, not runtime
performance acceptance. All failure logs and complete artifacts are retained.
No threshold, timed work, sample count, corpus, matrix or required job changed.
