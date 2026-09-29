# CalcKernel 0.15 MIR 与 KIR 边界

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

Textual MIR 不含 path、time、address 或 hash-map order，并在 0.15 line 内保持 semantic/byte
compatibility；`-O` 不再创建另一份 MIR。KIR 包含 scalar SSA、block parameter、region Memory
SSA、fact、effect summary 与 Proof certificate，在每个 pass 前后验证，是全部 backend 唯一的
target-neutral optimized input。`emit-kir` 是 deterministic inspection，但 KIR text 不承诺跨
版本兼容。

KIR 的纯 `VersionPredicate` instruction 返回 `bool`，最多含四个 conjunct：一个
`TripThreshold` 和三个非 trip predicate。仅用于 WebAssembly 的 `WasmSliceRange` conjunct 接收
`slice<T>`、`start: u32`、`count: u32` 和与元素宽度匹配的 4/8 字节值；`T` 仅限 `i32`、`u32`、
`i64`、`u64` 或 `f64`。它只适用于 Wasm32 `simd128` profile。`count` 为零时结果为 true；否则使用
扩宽后的无符号 64 位运算，检查 `start + count <= slice.len`，并检查 exclusive end byte address
`slice.data + (start + count) * element_bytes` 同时不超过 `2^32` 和 `memory.size * 65536`。
该谓词不访问或修改内存。独立 checker 仅在能够重新证明受支持 `f64x2` 仿射循环的准确标量访问
范围、源码 alias 证据及标量回退路径时接受它；其他候选保持标量。
