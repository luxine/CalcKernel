# CalcKernel 0.10 MIR

[English](../../reference/mir.md)

本文档定义 `ckc emit-mir` 输出的 deterministic textual MIR。MIR 是 C、WebAssembly 与
Native LLVM lowering 共用的唯一 typed three-address control-flow representation。

`MirModule` 按顺序持有 struct 与 function。Function 记录 name、export flag、entry role、
parameter、return type、local、block 与 runtime effect reachability。每个 block 有且仅有一个
return、jump 或 branch terminator。

`MirType` 覆盖全部 CK value type，包括 `MirType::Slice` 与 return-only `MirType::Void`。
Instruction 包括 constant、move、unary/binary/compare、conversion、load/store、call、
runtime print call、`MakeSlice`、`SliceIndex` 与 `Subslice`。Void call 为 `target: None`，
void return 为 `value: None`；不存在 synthetic void value/local。`break`/`continue` 分别变成
到最内层 loop exit/condition 的 `MirTerminator::Jump`。Slice operand 与 endpoint 按源码顺序
各 lowering 一次。

七个 Native print builtin 成为显式 runtime effect。Optimization 只能删除不可达 effect，
必须在 call、loop 与 inline 后保持所有可达 print 的次数与源码顺序。Backend root validator
拒绝所选 artifact 无法承载的 effect。

Overflow/bounds selection 作为 backend context 贯穿 lowering，不改变 source typing。Checked
Native 与 C 实现相同的 first-error order；MIR optimization 不得改变可能的 checked failure 或
runtime effect。

Validator 拒绝 unresolved type、void value、非法 slice shape、type-inconsistent operation、
malformed block/terminator。打印顺序由源码决定，不含 path、time、address 或 hash-map order。
O0 输出 lowering 后 MIR，O1–O3 使用 deterministic pipeline。`0.10.x` 内 textual format 兼容。
