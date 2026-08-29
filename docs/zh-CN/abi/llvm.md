# CalcKernel 0.11 Native LLVM 与 C ABI

[English](../../abi/llvm.md)

CalcKernel 0.11 固定 LLVM 22.1.8。Verified KIR 经 checked C++ bridge 结构化 lowering，在
optimization 前后验证，由 host TargetMachine 生成 object bytes，并在进程内用 LLD 链接。
`emit-llvm` 输出该 verified module 供 inspection。

Native generation 为 host-only。显式 target 规范化后必须等于 host triple；cross-target 在
创建 artifact 前拒绝。Release baseline 为 LLVM `x86-64` 加 mandatory SSE2，以及 generic
ARMv8-A 加 ABI-mandated FP/Advanced SIMD。`--cpu native` 仅为 build opt-in，run 使用 host。

Release binary 内含所需 host code generator、LLD driver 与 ORC layer，运行时不依赖 LLVM、
LLD、Clang 或 non-system C++ runtime。`CKC_LLVM_PREFIX` 只用于从源码构建 compiler。

Native 只接受 verified KIR artifact。Pre-LLVM fact audit 检查每个 attribute/metadata
candidate 的 origin、dominance、contract-instance scope、alias completeness、alignment、
range、effect 与 proof dependency；失败发生在 bridge invocation 前。合法 fact 才可映射到
LLVM `noalias`、`readonly`/`writeonly`、alignment、range、alias-scope、loop/vectorization
information，bridge 不会自行加强 fact。

CK integer 映射为同宽 LLVM integer，`f64` 为 `double`，bool 为 `i1`，pointer 为 opaque
`ptr`，struct 保持 field 顺序，void return 为 LLVM `void`。Natural void function 使用
`define void`，targetless call 使用 `call void`，完成时使用 `ret void`。Stored `slice<T>` 为
`{ ptr, i32 }`。`--overflow`/`--bounds` checked mode 使用显式 control flow 与 status code，
不使用 trap。这些是
compiler internal form，不是 public library ABI；0.9 的独立 textual LLVM export-shape promise
已退出。

0.11 的 public Native C ABI 保持 version 1；private LLVM bridge ABI 与 contract-aware
runtime ABI 为 version 2，native cache/codegen identity 使用 KIR v1。这会使 incompatible
0.10 object 失效，但不改变 foreign-call signature。

Native object/static/dynamic 通过 generated header 暴露唯一 Native C ABI。每个 public source
function 由 export thunk 包装 internal natural function；thunk 实现 target ABI classification、
bool normalization、slice flattening、checked return/status 与 symbol visibility。三个 library
artifact kind 采用同一契约。

Public mapping 为 fixed-width C integer、strict `double`、target C `_Bool`、保持 declaration
order 与 target padding/alignment 的 C struct、flattened `(T* data, uint32_t len)` slice
parameter、direct unchecked return/C `void`，或 module-wide checked status/result out-pointer。
Source symbol name/default visibility 保持；Windows dynamic export 使用 generated DLL decoration。

Compiler 显式拥有 SysV AMD64、Darwin x86-64、Linux/Darwin AAPCS64、Windows x64 与
Windows ARM64 classifier。它决定 register class、indirect/by-value aggregate、small aggregate
return、extension attribute、alignment 与 hidden result。Pinned Clang fixture 只作 development
oracle；generated header 是 consumer authority。

Native user artifact 不依赖 CK、LLVM、ORC、LLD、Clang、libc formatting 或 external compiler
runtime。Object/static archive 自然需要 consumer link step，但链接后不增加 CK runtime。
Dynamic library 只 export 请求的 CK symbol 与 required platform metadata。Linux executable
runtime 使用 kernel boundary；Windows 使用 embedded stable process import 且 computation DLL
无 entry；Darwin 使用 embedded minimal system stub 与 LLD ad-hoc signing。

`ckc run` 通过 ORC 执行相同 optimized object semantics。ELF/Mach-O AArch64/x86-64 与 COFF
x86-64 使用 JITLink；COFF AArch64 因 LLVM 22.1.8 尚无对应 JITLink backend，使用固定
RuntimeDyld compatibility path。两者都 eager resolve symbol，并在调用 `main` 前完成 RW-to-RX。
0.11 不提供 public embeddable ORC API；`emit-llvm` 也不承诺 stable external LLVM ABI。
