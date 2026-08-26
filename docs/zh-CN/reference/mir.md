# CalcKernel V0.9 MIR

[English](../../reference/mir.md)

本文档规范 `0.9.x` 中 `ckc emit-mir` 的 textual MIR。MIR 是 C、WASM、LLVM
共享的 typed three-address basic-block representation。

`MirModule` 按顺序包含 struct 与 function；`MirFunction` 包含 name、export flag、
typed parameter、return type、local 与 block；每个 `MirBlock` 按顺序包含
instruction 且只有一个 terminator。

`MirType` 包括 integer、`f64`、`bool`、pointer、struct、`MirType::Slice` 与
`MirType::Void`。Void 只可作为 function return；slice 在 backend 中变成明确的
data/length pair。

Instruction 包括 constant、move、unary/binary/compare、cast、load/store、call、
`MakeSlice`、`SliceIndex` 与 `Subslice`。Place 包括 local、parameter、field 与
index。Terminator 包括 return、jump 与 branch。

Void call 使用 `target: None`，void return 使用 `value: None`；自然 fallthrough
也生成 valueless return，不制造 synthetic void value。`break` 与 `continue`
分别成为指向最内层 loop exit/condition 的 `MirTerminator::Jump`。

Slice 构造、index 与 range operand 按源码顺序各 lower 一次。Checked bounds 是
明确 backend context，optimizer 不可删除或重排可观察 guard。

Validator 拒绝 unresolved/invalid type、void value、direct nested slice、exported
slice return、不一致 call/return/place、malformed slice instruction 与未终止 block。
Printer 的 declaration/local/block/instruction/operand/terminator 顺序确定，不包含
path、timestamp、address 或 hash-map 顺序。

`-O0` 输出未优化 lowered MIR；`-O1`–`-O3` 使用[Optimizer](../compiler/optimizer.md)
pipeline。Backend mode 不改变 `emit-mir`。`0.9.x` 保持 MIR syntax、instruction
meaning 与 deterministic printing 向后兼容；破坏性变化必须遵守[兼容性策略](../project/compatibility.md)。
