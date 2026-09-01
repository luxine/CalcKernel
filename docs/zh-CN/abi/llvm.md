# CalcKernel 0.13 Native LLVM 与 C ABI

[English](../../abi/llvm.md)

CalcKernel 0.13 固定 LLVM 22.1.8。Verified KIR 经 checked C++ bridge 结构化 lowering，在
optimization 前后验证，由 host TargetMachine 生成 object bytes，并在进程内用 LLD 链接。
`emit-llvm` 输出该 verified module 供 inspection。

Native generation 为 host-only。显式 target 规范化后必须等于 host triple；cross-target 在
创建 artifact 前拒绝。Release baseline 为 LLVM `x86-64` 加 mandatory SSE2，以及 generic
ARMv8-A 加 ABI-mandated FP/Advanced SIMD。`--cpu native` 仅为 build opt-in，run 使用 host。

Release binary 内含所需 host code generator、LLD driver 与 ORC layer，运行时不依赖 LLVM、
LLD、Clang 或 non-system C++ runtime。`CKC_LLVM_PREFIX` 只用于从源码构建 compiler。
Bootstrap cache identity 除 pinned LLVM manifest 与 bootstrap recipe 外，还包含全部 native
runtime source、header、assembly 与 platform link input，cached prefix 因而不能保留过期
runtime object。

Windows 的 LLVM/LLD 与 bridge 使用 release-static MSVC CRT（`/MT`），Rust 在所有
build profile 使用 `+crt-static`。Bootstrap 设置
`CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`，并在构建前检查真实 C/C++ compile commands。
安装和 cache validation 都用 pinned `llvm-readobj` 检查真实 COFF archive directives，
拒绝 dynamic、debug 或混合 CRT。Windows manifest 记录此 CRT identity，并包含 COFF
driver 的 LibDriver、WindowsManifest 与 DTLTO 依赖。校验脚本属于 cache key；仅声明
`static_only = true` 不能证明内容使用静态 CRT。

Native 只接受 verified KIR artifact。Pre-LLVM fact audit 检查每个 attribute/metadata
candidate 的 origin、dominance、contract-instance scope、alias completeness、alignment、
range、effect 与 proof dependency；失败发生在 bridge invocation 前。合法 fact 才可映射到
LLVM `noalias`、`readonly`/`writeonly`、alignment、range、alias-scope、loop/vectorization
information，bridge 不会自行加强 fact。

Module 的规范化 `KirTargetProfile` 在优化前按精确 host target/CPU policy 查询 LLVM
22.1.8。它闭合固定 operation universe、vector lane legality、alignment 与 CK 独立 cost
checker 使用的 integer structural cost。Rust/C++ boundary 会重新验证其 digest，digest 进入
object/cache identity。Target、feature、query 或 digest 不匹配时在 LLVM IR 构造前终止。

CK integer 映射为同宽 LLVM integer，`f64` 为 `double`，bool 为 `i1`，pointer 为 opaque
`ptr`，struct 保持 field 顺序，void return 为 LLVM `void`。Natural void function 使用
`define void`，targetless call 使用 `call void`，完成时使用 `ret void`。Stored `slice<T>` 为
`{ ptr, i32 }`。`--overflow`/`--bounds` checked mode 使用显式 control flow 与 status code，
不使用 trap。这些是
compiler internal form，不是 public library ABI；0.9 的独立 textual LLVM export-shape promise
已退出。

KIR v3 fixed vector 结构化 lowering 为等宽 LLVM vector。只有 KIR 独立 checker 已闭合 lane
mapping、operation equivalence、fallback identity、target legality 与 cost/budget proof 后，才
输出 vector load/store、strict arithmetic、cast、compare/select 及 modular integer add/multiply
reduction。LLVM optimization 可以继续改进合法 module，但不是 CK safety 或 alias claim 的来源。

0.13 的 public Native C ABI 保持 version 1，Runtime ABI 保持 version 2。Private LLVM
bridge ABI 4 替代 0.12 bridge ABI 3；native cache/codegen identity 使用 KIR v3、
`CKCOBJ03` key schema 4 与 manifest schema 4。这会使 0.12 及更早 private object 失效，但不改变
foreign-call signature。

## Profile 与 multiversion object

Profile-generation module 链接 compiler-private schema-1 collector；只有 library topology
暴露生成的 full-identity flush control。最终 profile-use module 不包含 counter、writer、
profile path 或 generation runtime。Profile annotation 由 CK 独立 optimizer 消费，不能变成
LLVM safety metadata 或 proof。

`--cpu multiversion` 将 verified baseline module、从 same KIR pre-state 生成的零个或多个独立
verified target variant，以及 compiler-private dispatch runtime 作为不同 named-object member
lowering。每个 object 在 assembly 前都通过 verifier 与 feature audit。baseline-safe detector
只识别闭合的 x86-64 v3/v4 和 Linux AArch64 SVE/SVE2 tier；状态不完整时 fail closed，并以
acquire-release atomic 发布一次 process-local selection。Public Native C ABI thunk 的 name、
address、signature、checked-status behavior 与 visibility 保持；baseline、variant、detector、
runtime symbol 都隐藏。

named-object bundle 可链接为 executable、dynamic library 或 static archive。multiversion object
output 会拒绝，因为 0.13 不定义 partial-link bundle contract；baseline/native single-version
object 继续支持。`CKCOBJ03` 只有在 ordered member name/role、target set、profile、dispatch
runtime、physical artifact kind、每个 object digest、key schema 4 与 manifest schema 4 全部
匹配时才接受 cache hit。最终 artifact 延续 self-contained system-runtime policy，不新增
CK/LLVM/compiler shared dependency。

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
Dynamic library 只 export 请求的 CK symbol 与 required platform metadata。ELF linked product
中的 non-allocating producer provenance 不是依赖：产物保留精确的
`Linker: LLD 22.1.8` `.comment`，artifact audit 要求该 section 必须为 non-`ALLOC`，并独立
拒绝 loader-visible dependency、executable undefined symbol 与 unexpected export。Linux executable
runtime 使用 kernel boundary；Windows 使用 embedded stable process import 且 computation DLL
无 entry；Darwin 使用 embedded minimal system stub 与 LLD ad-hoc signing。Darwin AOT 与
ORC 的 object 统一使用 PIC 与显式 Small code model。Internal call 不得在 executable
`__text` 中产生 absolute pointer fixup，dyld 不得需要写入代码页。`LC_MAIN` 指向 compiler
生成的 C-ABI entry `_main`，dyld 以普通函数方式调用，并将返回值作为 process exit status。
Runtime failure 经 embedded platform exit helper 终止。

`ckc run` 通过 ORC 执行相同 optimized object semantics。ELF/Mach-O AArch64/x86-64 与 COFF
x86-64 使用 JITLink；COFF AArch64 因 LLVM 22.1.8 尚无对应 JITLink backend，使用固定
RuntimeDyld compatibility path。两者都 eager resolve symbol，并在调用 `main` 前完成 RW-to-RX。
COFF AArch64 compatibility layer 保留 CK 的 audited section memory manager，同时恢复
LLVM 22.1.8 LLJIT 标准 COFF responsibility contract：将 RuntimeDyld object flags 与
materialization responsibility 对齐，并自动认领 weak/COMDAT 等额外 object symbols。
该行为仅限既有 compatibility path，不开放 process-symbol search，也不把 RuntimeDyld
扩展为 CK 的通用后端。
COFF x86-64 JITLink 继续禁用任意 process-symbol lookup。五个 embedded CK runtime object
仅在 JIT execution 中与一个独立散列、纯数据的 `__ImageBase` anchor 组合；anchor 与固定
object set 位于同一个 512 MiB JIT reservation，使 MSVC `.pdata` 的 image-relative
relocation 保持可表示。该 support object 只属于 `run` 内部，不会传给 LLD 的 object、
static、dynamic 或 executable artifact，也不增加公开 CK symbol 或运行依赖。CK program
object 若定义 PE/COFF 保留名 `__ImageBase`，会在执行前被拒绝。
Darwin ORC 按 runtime capability 在两条互斥 W^X 机制中选择：支持 per-thread JIT write
protection 时使用 `MAP_JIT`，在线程级 writable/non-executable 与 readable/executable 间
切换；能力不可用（包括 Darwin x86-64 和受限 virtual host）时，先预留普通 RW/NX pages，
再逐 segment 以页保护 finalization 为 RX 或 R/NX。后者不是 RWX fallback。Internal audit
拒绝混合 capability tuple，并为两条路径验证 relocation、最终 code/data permission 与
instruction-cache finalization。
0.13 不提供 public embeddable ORC API；`emit-llvm` 也不承诺 stable external LLVM ABI。
