# CalcKernel 0.12 MIR and KIR Boundary

[简体中文](../zh-CN/reference/mir.md)

This document defines deterministic textual semantic MIR emitted by
`ckc emit-mir` and its boundary with internal KIR. MIR owns source evaluation
order, checked first-error order, runtime-print order, and backend-independent
meaning. All C, WebAssembly, and Native LLVM artifacts are lowered from verified
KIR, not from an optimized-MIR product path.

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

Overflow and bounds selections do not alter source typing or semantic MIR.
After consumer reachability and capability checks, KIR construction materializes
exactly the selected guards and ordered effects. Library consumers root exports,
executables root `main`, and `emit-kir` inspection roots their union.

## Validation, KIR, and printing

Validation rejects unresolved types, void values, invalid slice shapes,
type-inconsistent calls/returns/places, malformed instructions, missing blocks,
and malformed terminators. All blocks are terminated before backend lowering.

Textual MIR follows source-derived declaration, local, block, instruction,
operand, and terminator order. It contains no path, time, address, or hash-map
order. `emit-mir` remains semantic and byte-stable for the 0.12 line regardless
of `-O`; optimization no longer creates a second MIR product path.

KIR uses scalar SSA, explicit block parameters, region Memory SSA, facts,
interprocedural effect summaries, and Proof certificates. It is built for the
selected consumer and modes, verified before optimization and after every pass,
and is the sole target-neutral optimized input to all backends. `emit-kir`
prints deterministic inspection text and may also print fact/effect/proof
evidence. KIR is a private compiler format with no cross-version text guarantee.

The semantic MIR textual format is compatible within `0.12.x`; a breaking grammar or meaning
change requires a later minor release and migration note under the project
[compatibility policy](../project/compatibility.md).
