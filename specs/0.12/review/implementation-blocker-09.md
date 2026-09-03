# Implementation blocker 09: x86 ELF Large code-model regression

Date: 2026-09-04

## Finding

Exact candidate CI run `33781955921`, job `100738211965`, passed the
compile-time stability repair but failed the unchanged schema-7 domain-fact
gate. Its retained x86-64 artifact shows that `contract_noalias` was emitted
with the ORC builder's Large code model: the hot 16-element kernel contains
additional `movabs` address materialization and Large-model constant access,
while both generic oracle artifacts use the ordinary Small model.

This is an implementation/contract mismatch, not a benchmark exception.
The durable LLVM and release contracts already require PIC with an explicit
Small code model for both AOT and ORC, but the host target constructor applied
Small only to Mach-O. ELF therefore lost the no-alias kernel's intended
advantage to fixed per-call address construction.

## Resolution

The host target constructor now selects PIC plus Small unconditionally for the
closed native host matrix. CK's ORC runtime dependencies already reside in the
same object graph, so this retains the required reachability while removing
Large-model overhead from ELF and COFF products. A structural regression
requires the setting to precede target-machine construction and rejects a
Mach-O-only guard.

No language, ABI, vector policy, performance threshold, sample count, corpus,
or platform gate changes. The exact schema-7 gate must be rerun on the repaired
x86-64 artifact; the failed run cannot sign acceptance.
