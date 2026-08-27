# CalcKernel 0.10 MIR

[简体中文](../zh-CN/reference/mir.md)

This document defines deterministic textual MIR emitted by `ckc emit-mir`.
MIR is the single typed, three-address control-flow representation consumed by
C, WebAssembly, and Native LLVM lowering.

## Model

`MirModule` owns ordered structs and functions. Each function records its name,
export flag, optional program-entry role, parameters, return type, locals,
blocks, and runtime effect reachability. Every block has ordered instructions
and exactly one return, jump, or branch terminator.

`MirType` covers all CK value types, including `MirType::Slice`, plus return-only
`MirType::Void`. Instructions include
constants, moves, unary/binary/compare, conversions, load/store, ordinary call,
runtime print call, `MakeSlice`, `SliceIndex`, and `Subslice`. Places cover
local, parameter, field, and indexed storage.

A void call has `target: None` and a void return has `value: None`. Natural void
fallthrough becomes a valueless return; no synthetic void value/local exists.
`break` and `continue` become `MirTerminator::Jump` to the innermost loop exit
and condition. Slice operands and endpoints are lowered once in source order.

The seven Native print builtins lower to explicit runtime effect instructions.
Optimization may remove only unreachable effects and must preserve the count
and source order of every reachable print through calls, loops, and inlining.
Backend root validation rejects effects that the selected artifact cannot host.

## Checked semantics

Overflow and bounds selections are backend context carried through lowering;
they do not alter source typing. Checked Native and C lowering implement the
same first-error ordering. MIR optimization cannot erase, duplicate, or reorder
an operation in a way that changes a possible checked failure or runtime effect.

## Validation, optimization, and printing

Validation rejects unresolved types, void values, invalid slice shapes,
type-inconsistent calls/returns/places, malformed instructions, missing blocks,
and malformed terminators. All blocks are terminated before backend lowering.

Text output follows source-derived declaration, local, block, instruction,
operand, and terminator order. It contains no path, time, address, or hash-map
order. O0 prints lowered MIR; O1–O3 apply the documented deterministic pipeline.
One optimization selection is used by MIR and Native LLVM when building.

The textual format is compatible within `0.10.x`; a breaking grammar or meaning
change requires a later minor release and migration note under the project
[compatibility policy](../project/compatibility.md).
