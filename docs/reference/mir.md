# CalcKernel V0.9 MIR

[简体中文](../zh-CN/reference/mir.md)

This document is normative for the textual MIR emitted by `ckc emit-mir` in the
`0.9.x` line. MIR is a typed, three-address, basic-block representation shared
by the C, WASM, and LLVM backends.

## Model

`MirModule` owns ordered structs and functions. A `MirFunction` owns its name,
export flag, typed parameters, return type, locals, and ordered blocks. Every
`MirBlock` has ordered instructions and exactly one terminator.

`MirType` covers integers, `f64`, `bool`, pointers, structs, `MirType::Slice`,
and `MirType::Void`. Void is valid only as a function return. Slices carry an
element type and lower as a data/length pair in backend-specific form.

Instructions include constants, move, unary/binary/compare, cast, load/store,
call, `MakeSlice`, `SliceIndex`, and `Subslice`. Places cover local, parameter,
field, and index storage. Terminators are return, jump, and branch.

A void call has `target: None`; a void return has `value: None`. Natural void
fallthrough becomes a valueless return. No synthetic void local or value exists.
`break` and `continue` lower to `MirTerminator::Jump` targeting the innermost
loop exit and condition respectively.

Slice construction, index, and range operands are lowered once in source order.
`MakeSlice` retains pointer and `u32` length; `SliceIndex` retains the slice and
index; `Subslice` retains slice, start, and end. Checked bounds remain explicit
backend context and are never erased or reordered by optimization.

## Validation and printing

The validator rejects unresolved or invalid types, void values, direct nested
slices, exported slice returns, type-inconsistent calls/returns/places, malformed
slice instructions, missing blocks, and malformed terminators. All blocks are
terminated before a backend receives MIR.

Printing is deterministic: declaration, local, block, instruction, operand, and
terminator order derives solely from the source and selected optimization
pipeline. It contains no filesystem path, timestamp, address, or hash-map order.

`-O0` prints lowered MIR without optimization. `-O1`–`-O3` print the result of
the documented pipeline in [Optimizer](../compiler/optimizer.md). Backend modes
do not change source checking or the MIR emitted by `emit-mir`.

## Compatibility

Within `0.9.x`, accepted MIR syntax, instruction/terminator meaning, deterministic
printing, and `emit-mir` behavior are backward compatible. Additive explanatory
comments are not emitted into the stable text. A breaking MIR change requires a
minor-version boundary and migration guidance under the project
[compatibility policy](../project/compatibility.md).
