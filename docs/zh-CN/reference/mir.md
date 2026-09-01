# CalcKernel 0.12 MIR 与 KIR 边界

[English](../../reference/mir.md)

本文档定义 `ckc emit-mir` 输出的 deterministic textual semantic MIR，以及它与 internal KIR
的边界。MIR 负责 source evaluation order、checked first-error order、runtime print order 与
backend-independent meaning；所有 C、WebAssembly、Native LLVM artifact 都从 verified KIR
lowering，不存在 optimized-MIR product path。

`MirModule` 顺序持有 struct/function；function 记录 export、entry、parameter、return、local、
block 与 runtime effect reachability。每个 block 有一个 return/jump/branch terminator。
`MirType::Slice`、`MirType::Void`、`MakeSlice`、`SliceIndex`、`Subslice`、使用
`target: None` 的 void call、使用 `value: None` 的 void return，以及 `break`/`continue` 的
`MirTerminator::Jump` 都保留 semantic form；不存在 synthetic void value/local。
Operand 与 range endpoint 只按 source order lowering 一次。

七个 Native print builtin 是 explicit runtime effect。MIR 保持所有可达 print 与 possible
failure 的次数和顺序；consumer root validator 在 KIR 构造前拒绝 artifact 无法承载的 effect。
Overflow/bounds 不改变 source typing 或 MIR。Library root 是 export，executable root 是
`main`，`emit-kir` inspection root 是两者并集；mode-specific KIR 显式 materialize 所需 guard。

Textual MIR 不含 path、time、address 或 hash-map order，并在 0.12 line 内保持 semantic/byte
compatibility；`-O` 不再创建另一份 MIR。KIR 包含 scalar SSA、block parameter、region Memory
SSA、fact、effect summary 与 Proof certificate，在每个 pass 前后验证，是全部 backend 唯一的
target-neutral optimized input。`emit-kir` 是 deterministic inspection，但 KIR text 不承诺跨
版本兼容。
