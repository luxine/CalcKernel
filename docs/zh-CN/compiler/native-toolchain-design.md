# CalcKernel 0.10 原生工具链设计

[English](../../compiler/native-toolchain-design.md)

本文是 CalcKernel 0.10 原生工具链获准实施的设计。它面向未来：在实现、测试、
migration note 与 0.10 release contract 同时落地前，V0.9 reference/ABI 文档仍是
当前权威。

## 目标与已冻结决策

CalcKernel 0.10 增加自包含的优化型原生工具链，同时保留 C 与 WebAssembly source
emitter。每个支持平台的 release 都提供一个功能完整的 `ckc` executable，并静态
内嵌固定版本 LLVM、ORC 与 LLD。用户无需安装 Clang、LLVM、LLD、平台 linker 或
CK runtime。

首个版本在现有六个 release target 上支持宿主原生编译和执行：

- macOS AArch64 与 x86-64；
- Linux AArch64 与 x86-64；
- Windows AArch64 与 x86-64。

0.10 不支持跨 OS 或跨架构链接。`ckc run` 始终以当前 CPU 为目标；`ckc build`
以当前 OS/架构为目标，默认使用可移植 CPU baseline，显式 `--cpu native` 才绑定
构建机器的可用 CPU feature。

原生运行时不增加 heap、allocator、ownership system、string、命令行参数、标准
输入、module 或 sandbox。生成的 FFI library 仍是纯计算库，memory 仍由 caller
拥有。

## Compiler 架构

```text
CK source
  -> frontend 与 type checking
  -> MIR lowering 与 validation
  -> CK MIR O0-O3
  |  -> C emitter
  |  -> WAT/WASM emitter
  `  -> 结构化 LLVM module builder
       -> Native ABI export thunk
       -> LLVM verifier
       -> LLVM PassBuilder O0-O3
       |  -> ORC -> JITLink 或固定的 COFF/AArch64 RuntimeDyld layer
       `  -> TargetMachine -> object -> archive writer 或 LLD
```

Native backend 直接构造内存 LLVM module，不打印 LLVM text 后再解析。
`emit-llvm` 打印同一个 verified module，避免 textual IR 与 native code 因两套
lowering 分叉。

Rust 实现通过狭窄的内部 C-compatible boundary 使用 LLVM。Core IR、PassBuilder、
target 与 ORC 在覆盖充分时使用 LLVM C interface；只有 LLD/ORC 必需部分使用小型、
锁定版本的 C++ shim。C++ object 与 exception 不跨入 Rust。LLVM verification
failure 属于 internal compiler error，不提交任何 artifact。

0.10 line 固定 LLVM 22.1.8。Source tag 与 archive checksum 是仓库受控输入。
Patch-line 升级必须通过完整 native semantics、ABI、performance 与 release suite；
LLVM major 升级属于有版本的 compiler 变更，不是普通 dependency update。

Release build 只包含该平台需要的 host code generator、object format、ORC object
layer 与 LLD driver。ELF AArch64/x86-64、Mach-O AArch64/x86-64 与 COFF x86-64
使用 JITLink。LLVM 22.1.8 没有 COFF AArch64 JITLink backend，因此 Windows
AArch64 使用 ORC `RTDyldObjectLinkingLayer`，以及固定版本的
`RuntimeDyldCOFFAArch64` 与启用 reservation 的 `SectionMemoryManager`，保证 code、
stub 与 data 位于 AArch64 relocation range 内。这是 target-specific link-layer
compatibility choice，不是 interpreter 或 AOT fallback：两条路径都执行相同的 O3
TargetMachine object，并在 `main` 前完成 eager symbol resolution。

Release 不包含 Clang frontend 或无关 LLVM target。LLVM、LLD 与其非系统 C++ runtime
静态链接；会给目标机器增加 shared-library 需求的可选 LLVM dependency 必须关闭。
未来把 Windows AArch64 切换到 JITLink，必须作为 LLVM version 变更，并通过完整的
run、memory-protection、performance 与 release gate。

## Source 入口

`main` 是保留的程序入口名称，只接受：

```ck
fn main() -> void
fn main() -> i32
```

它不得带参数，也不得声明为 `export`。`ckc run` 与
`build --kind executable` 要求一个合法 `main`。Void main 的进程状态为 0；
i32 结果交给平台 exit facility。可移植程序使用 0–239；超出平台可观察 exit
范围的值遵循平台 exit 语义。

Library/object build 不导出 `main`；它保持 internal，并在不可达时删除。C 与
WebAssembly 可以把合法 `main` lower 为普通 internal function，但不创建 native
process entry。

## CLI contract

统一后的 native build surface：

```text
ckc run <file.ck> [-O0|-O1|-O2|-O3]
    [--overflow unchecked|checked]
    [--bounds unchecked|checked]
    [--no-cache]

ckc build <file.ck> --kind executable|dynamic|static|object --out <path>
    [-O0|-O1|-O2|-O3]
    [--overflow unchecked|checked]
    [--bounds unchecked|checked]
    [--cpu baseline|native]
```

省略 `--kind` 继续表示 `dynamic`，保留 V0.9 command default。`build-llvm` 在
0.10 中作为 dynamic/object native build 的 deprecated alias，向 stderr 写一条
migration warning；它不再是独立 backend。

`run` 与 `build` 默认 O3。一个优化选择同时控制 CK MIR pipeline 与 LLVM default
pipeline。`check` 与 `emit-*` 保持 O0 默认值。O3 保持 CK strict floating-point
语义，绝不隐含 LLVM fast-math flag；fast floating-point mode 不属于本设计。

`run` 始终使用检测到的 host CPU 与 feature。`build` 默认使用对应 release target
记录的 baseline；`--cpu native` opt in host feature。Native `build` 与
`emit-llvm` 都在创建 artifact 前拒绝非 host target triple，使 printed module、
target DataLayout 与 C ABI thunk 始终处于同一个 verified host-native contract。

所有 x86-64 release 的 CPU baseline 为 LLVM `x86-64` 加 architecture mandatory
SSE2；所有 AArch64 release 为 generic ARMv8-A 加 ABI mandatory FP/Advanced SIMD。
Baseline artifact 不得仅因 compiler 运行在更新 host 就获得新的 optional ISA feature。

`emit-c` 继续生成 C/header，但不调用 compiler。产品 CLI 不包含外部 Clang
discovery、subprocess 或自动 fallback。开发者安装的 Clang 只能用于仓库 oracle 与
benchmark test。

## Build artifact

`object` 在 ELF/Mach-O host 写 `.o`，Windows 写 `.obj`；`static` 写 `.a` 或
`.lib`；`dynamic` 写 `.so`、`.dylib` 或 `.dll`，Windows dynamic build 还写
import `.lib`；`executable` 写平台原生 executable。

Object、static 与 dynamic build 同时生成 sibling C ABI header；executable 不生成
header。Windows dynamic header 为 consumer 标记 `dllimport`；object/static header
定义不带 DLL storage class 的 `CK_API`。所有输出完成 staging 与 validation 后才开始
替换 destination。每个 destination 作为单独文件原子替换，因此绝不暴露部分写入的
object、library、executable、header 或 import library。Commit 前失败保持所有已有
destination 不变。Multi-file output 在 commit-time failure 时从同 filesystem backup
rollback，并报告受影响 path；普通 filesystem 不提供可移植的多文件 transaction，
因此进程或 OS 非正常失败可能留下完整的新旧文件并存。Release packaging 与其他要求
crash consistency 的 consumer 必须构建到新目录，并仅在 `ckc` 成功后发布该目录。

TargetMachine 直接生成 object byte。Archive writer 不通过平台 `ar` command 即可
封装 native object。LLD 通过内部 FFI boundary 以 in-process library 方式调用。
它的 trusted input 仅限 verified compiler-produced object，以及 compiler-owned
entry/runtime object、helper object、export list 与最小 platform import definition。
0.10 不接受用户提供的任意 object、library、linker script 或 flag。

这些 compiler-owned input 在不查找 SDK/toolchain 的情况下闭合 platform-linker
boundary。Linux executable 使用 embedded syscall runtime，不使用 system import
library。Windows executable 使用内嵌的稳定 process API import definition，纯计算 DLL
使用 `/noentry`。Darwin executable 使用内嵌的最小 `libSystem` text stub、显式
platform version 与 LLD ad-hoc signing；生成的 Mach-O executable/dynamic library
无需调用 `codesign` 即可运行/加载，distributor 之后可以用自己的 identity 替换 ad-hoc
signature。每个 embedded linker input 的 source、license provenance、symbol 与 hash
都由仓库控制，并纳入 release test。

## Native FFI ABI

已记录的 C ABI 成为唯一公共 Native ABI。LLVM IR type 是内部表示，不得泄漏进
exported signature。每个 `export fn` 都有 external C ABI thunk；thunk 后方的
optimized CK implementation 可以使用不同 internal signature。

Thunk 保持全部已有承诺：

- 固定宽度 integer 与 strict `double` mapping；
- target C `_Bool` parameter/result/stored-field 表示；
- declaration-order C struct layout 与 target padding/alignment；
- flatten slice parameter `(T* data, uint32_t len)`；
- unchecked direct return 与 C `void`；
- module-wide checked status return 与 result out-pointer；
- source symbol name、default visibility 与 Windows DLL export。

Source aggregate ABI lowering 是 frontend 职责，LLVM 不会从 named IR struct 自动
推导。因此 Native backend 为下列 target family 显式实现 C ABI classifier：

- Linux x86-64 的 SysV AMD64；
- Darwin x86-64；
- Linux/Darwin AArch64 的 AAPCS64 variant；
- Windows x64 与 Windows ARM64。

Classifier 决定 register class、indirect/by-value parameter、small aggregate
return、alignment、extension attribute 与 hidden result pointer。开发期使用同一
固定 Clang major 对 fixture 做差分；这只是 compiler-development oracle，不是
release 或用户依赖。

Export thunk 在 LLVM O3 前加入。LLVM 可以把 CK implementation inline 进 thunk，
因此 ABI boundary 不必增加内部 call。FFI caller 仍只承担一次普通 native C call，
与可比 C library 相同。Generated header 继续是 consumer 权威。

## Checked mode

Native backend 支持 overflow 与 slice-bounds 的四种组合，默认都仍为 unchecked。
任一 checked mode 启用已有 module-wide `CK_Status` ABI，并保持 error order。

LLVM lowering 使用 overflow intrinsic、显式 division guard、slice guard 与 status
propagation，不用 trap 实现 checked 行为。Unchecked lowering 保持当前语义，不增加
guard。

Checked program entry 的 generated wrapper 在需要时提供有效 result pointer。
`CK_OK` 使用 source main result；传播的 checked failure 忽略未写入结果，向 stderr
写一行固定英文 runtime diagnostic，并使用以下保留状态退出：

| Runtime ID | 条件 | 进程状态 |
| --- | --- | ---: |
| `CKR0001` | integer overflow | 240 |
| `CKR0002` | integer division/modulo by zero | 241 |
| `CKR0003` | null checked result pointer | 242 |
| `CKR0004` | slice index/sub-slice out of bounds | 243 |
| `CKR0005` | standard-output write failure | 244 |
| `CKR0006` | abnormal native child termination | 245 |

ID、英文消息与进程状态是稳定 0.10 runtime contract，与 source diagnostic 分离。
可移植 application 不应把 240–245 用作自身 process-level signal。

精确 diagnostic byte string 为 UTF-8/ASCII，并以一个 LF byte 结尾：

```text
CKR0001: integer overflow
CKR0002: integer division or modulo by zero
CKR0003: null checked result pointer
CKR0004: slice index or sub-slice out of bounds
CKR0005: standard output write failed
CKR0006: native child terminated abnormally
```

stdout 失败后尝试向 stderr 写 `CKR0005`；该 diagnostic write 再失败也不改变 status
244。`CKR0006` 只由 `ckc run` parent 输出，绝不覆盖更具体的正常 child status。

## 最小 Native Runtime

Runtime 是无 heap 的小型 host-specific object，以 bytes 内嵌进对应 `ckc` release。
`run` JIT-link 它；executable build 通过 LLD 链接同一个 object。它提供 entry/exit
glue、runtime diagnostic，以及这些保留、仅 LLVM native 可用的 predeclared
function：

```ck
print_i32(value: i32) -> void
print_i64(value: i64) -> void
print_u32(value: u32) -> void
print_u64(value: u64) -> void
print_f64(value: f64) -> void
print_bool(value: bool) -> void
print_newline() -> void
```

Value function 不追加 newline；`print_newline` 在所有平台输出一个 LF byte。
Integer 输出十进制，不使用 locale、分组、前导零或正号。Bool 输出 `true`/`false`。
Finite f64 在 round-to-nearest ties-to-even 下使用无 allocation 的 shortest-round-trip
decimal algorithm；negative zero 输出 `-0.0`，特殊值为 `nan`、`inf`、`-inf`，不表示
NaN payload/sign。

每次调用在 bounded stack buffer 中格式化，完整输出或以 `CKR0005` 退出。Linux 使用
支持的 kernel write/exit boundary；Darwin/Windows 通过最小 embedded import
metadata 使用稳定 OS process API。生成 executable 不链接 libc formatting、locale、
heap、Rust runtime 或 CK dynamic runtime。

Print call 是 observable side effect。MIR/LLVM optimization 不得相对 source
evaluation order 删除、复制、合并、hoist、sink 或 reorder。即使 checked mode 启用
module-wide status ABI，它们仍是 void runtime intrinsic；输出失败终止进程，不返回
`CK_Status`。

Print function 可以出现在 `emit-mir`/`emit-llvm` inspection output 中，但只有 `run`
与 executable build 能链接。非 executable native build 遇到 artifact root 可达的
print call 时拒绝；C 与 WebAssembly emission 在写 artifact 前拒绝任何 print
builtin。这样 FFI library 不会因输出失败终止 host，并保持纯计算。

## 零依赖 library 保证

Native object、static 与 dynamic output 不依赖 CK、LLVM、ORC、LLD、Clang、libc
formatting 或 external compiler runtime。LLVM 只是 compiler implementation detail，
不进入用户 artifact。

如果 target lowering 引入 compiler helper，则把对应 permissive-license 实现静态
链接进 artifact。Dynamic library 可以依赖 platform loader，但没有 CK/LLVM runtime
import。Release suite 检查 ELF `DT_NEEDED`、Mach-O load command 与 PE import，拒绝
意外依赖。Windows 纯计算 DLL 不使用 runtime entry point。

Library 只暴露请求的 CK export 与必要 ABI metadata。Pointer/slice memory 仍由
caller allocation、alignment 与 ownership。Static archive/object 自然需要消费语言
执行 link step，但完成链接后不增加 runtime library 需求。

## `ckc run` process 与 cache model

公共 `ckc run` process 以 private child mode 启动同一个 executable。Child 完成
compile、cache lookup、JIT link 与 main execution；parent 转发 stdout/stderr，返回
child status，转发用户 interrupt，并把可识别 signal/Windows exception 转成
`CKR0006`。这是 process isolation，不是 security sandbox。

Child 使用 eager ORC O3 compilation。开始执行后不存在 interpreter loop，也没有
hot CK function 的 lazy-compile stub；steady state 是普通 native machine code。

Persistent object cache 默认启用。Cache entry name 是下列 key 的 canonical、
versioned serialization 所得 SHA-256 digest 的小写十六进制；key 覆盖：

- 精确 source bytes 与 compiler version；
- runtime 与 Native ABI revision；
- LLVM version 与 target triple；
- MIR/LLVM optimization level；
- overflow/bounds mode；
- host CPU name 与完整 feature set；
- 所有影响 object byte 的 codegen option。

Cache entry 包含 manifest、object byte 及覆盖两者的 SHA-256 integrity digest。写入
使用仅 owner 可访问的同 filesystem temporary file 与 atomic rename。Cache root 与
entry 必须由当前 OS identity 所有，且不可由其他 identity 写入。Ownership、permission、
manifest、digest 或 object parse 无效都视为 miss，且不得使合法 source build 失败。
Digest 只检测 corruption，不认证使用相同 OS credential 的恶意修改：cache content 与
source/compiler configuration 一样，位于用户信任边界内；该边界不可信时必须使用
`--no-cache`。默认 soft limit 为 1 GiB，best-effort LRU eviction。`--no-cache` 绕过
读写，`ckc cache clean` 只删除解析后的 CK cache directory。

Resolved directory 在 Linux 为 `$XDG_CACHE_HOME/ckc` 或 `$HOME/.cache/ckc`，macOS
为 `$HOME/Library/Caches/ckc`，Windows 为
`%LOCALAPPDATA%\CalcKernel\cache`。缺失必需 base directory 时，本次 run 禁用 cache，
不会自行选择 process-wide writable location。

Cache 内容与 eviction order 不是 compatibility contract。Cache hit 必须与 clean
compile 产生相同输出和 runtime 行为。

## 性能 contract

Reference comparison 是同一 CK source 通过 C backend 生成后，由固定 Clang 以
strict `-O3` 编译，并使用相同 CPU baseline/native feature 与 checked mode。
Fast-math 或语义更弱的 C reference 无效。

指定 core runtime suite 必须满足：

- Native LLVM throughput 几何平均至少达到 C/Clang O3 的 95%；
- 任一 kernel 慢超过 10% 都阻断 release，除非记录经审查、可复现的 target limitation；
- scalar 与简单 loop 应产生同等级 instruction quality；
- unchecked/checked suite 分别报告和门禁；
- FFI benchmark 必须 batch work，不能把 host-language call overhead 标成 generated
  code performance。

Compilation latency、cold run、warm cache-hit run、peak memory、artifact size 与
steady-state runtime 分开测量。O3 可以编译较慢，但仍是 run/build default。受控
x86-64/AArch64 benchmark host 执行性能门禁；六个 release target 都执行 functional
与 ABI suite。

## Diagnostic、failure 与安全边界

Source error 仍使用稳定 `CKxxxx` diagnostic，并在 native output 前产生。
Unsupported target、CPU、artifact kind 或 backend/runtime 组合属于 CLI error。
LLVM、ORC、ABI classifier、embedded runtime 与 LLD failure 保留 stage，不伪装成
source diagnostic。

JIT child 隔离执行 CK raw pointer/unchecked operation 导致的进程失败，但不会让代码
memory-safe。Persistent compiler process 不加载用户 machine code。LLD 只接收上述
compiler-owned trusted input。平台提供相应能力时，temporary output/cache path 拒绝
不安全 ownership 或 symlink replacement。

执行 JIT code 的 thread 不得写入该 code。Linux/Windows 的 ORC memory manager 在
relocation 阶段分配 writable/non-executable page，完成后把 code 转为 read/execute、
data 保持 non-executable，并 flush instruction cache 后再转移控制。Windows AArch64
`SectionMemoryManager` 必须执行相同的 RW-to-RX finalization。Darwin 使用 `MAP_JIT`
与 Apple per-thread JIT write protection：mapping 可以声明 writable/executable maximum
permission，但执行 thread 绝不能同时启用 write 与 execute access。Signed macOS
release binary 仅在 hardened runtime 要求时携带最小范围的 `allow-jit` entitlement；
不得为运行 JIT 而关闭 library validation 或其他 code-signing protection。

Compilation、runtime、checked、output 或 abnormal child failure 都使 parent 非零
退出；child 成功退出前不打印 success line。成功 `run` 完全不打印 compiler status
text：stdout 归 CK program，compiler/runtime diagnostic 归 stderr。

`CKR0006` 是 `ckc run` parent diagnostic。Standalone executable 若被未处理 machine
fault 终止，则保留 host OS signal/exception 行为；最小 runtime 不安装可能干扰 raw
pointer 语义的 process-wide crash handler。

## 验证与 release 门禁

以下门禁全部通过前，implementation 不算完成：

1. 每个代表性 language fixture 与 checked-mode combination 在 construction/O3 后
   都通过 Native IR verifier。
2. C-versus-Native differential test 覆盖 scalar、control flow、void、call、struct、
   pointer、slice、checked ordering 与 f64 edge。
3. 六个 release target 的 ABI classifier 都与固定 Clang fixture 比较；开发期 C
   harness 能编译 generated header。
4. Python `ctypes` 或等价 system FFI test 在没有 compiler 时加载 dynamic library，
   并执行所有 exported shape。
5. External-tool PATH 为空时仍生成 object/static/dynamic/executable；dependency
   inspection 证明零 runtime guarantee。
6. 六个平台上的 run 与 AOT executable 在输出、正常 exit status、checked
   diagnostic 与 print formatting 方面一致；另行证明 run parent 把 abnormal child
   termination 映射为 `CKR0006`。
7. 测试 cache miss/hit/bypass/corruption/permission/concurrent write/eviction/clean；
   损坏 cache 绝不改变 semantics。
8. 受控 x86-64/AArch64 worker 通过性能 contract。
9. Release archive 保持现有六个 target name 与 checksum sidecar；
   `ckc --version --verbose` 报告 compiler、LLVM、Native ABI、runtime ABI、target 与
   enabled CPU backend，以及 active ORC object layer。
10. Release binary 没有 dynamic LLVM/LLD/Clang 或非系统 C++ runtime dependency。
    LLVM required notice 内嵌并可通过 `ckc licenses` 查看，使功能性 distribution
    仍为单 executable。
11. Linux/Windows 的 JIT page-permission test 证明 RW-to-RX transition，包括 COFF
    AArch64 RuntimeDyld 路径；Darwin test 证明 thread-level JIT write protection，
    而不是拒绝 `MAP_JIT` mapping 的 maximum permission。Signed macOS
    AArch64/x86-64 archive 必须在其 release signing/hardened-runtime configuration
    下实际执行 `ckc run`。

Release CI 在受控、可缓存 stage 构建固定 LLVM source，并静态链接 target-specific
component。普通 compiler test 无需每次重建 LLVM；required native-toolchain job 负责
完整 integration matrix。从 source 构建 `ckc` 时文档说明匹配的 LLVM bootstrap；
release archive 用户不执行该步骤。

## 0.10 兼容边界

0.10 只通过显式 release documentation 改变实现与 command 行为：

- `build` 仍默认生成 dynamic C-ABI library，但内部改用 LLVM/LLD，不再需要 Clang；
- `build --kind` 增加 executable、static 与 object；
- `run`、`main` 与 LLVM-native numeric output 是新增能力；
- `main` 与七个 native print name 成为 reserved；冲突的 V0.9 user declaration 必须
  重命名；
- `build-llvm` 是 deprecated compatibility alias；
- Native checked mode 现在匹配已有 C status contract；
- 独立 LLVM exported-shape ABI 退役，统一为 Native C ABI；`emit-llvm` 仍是 inspection
  artifact；
- Native `emit-llvm --target` 限制为 normalized host triple，避免 DataLayout 与
  public thunk 声称不受支持的 cross-target ABI；
- Native build 不再留下 generated `.c`/`.ll` intermediate；需要这些 inspection
  artifact 时使用 `emit-c`/`emit-llvm`；
- `emit-c` 保留，但绝不编译或链接输出。

C/WASM contract 不会静默获得 Native runtime I/O。不依赖已记录 standalone LLVM
export shape 的 V0.9 program 保持 source semantics。Tag 前 compatibility fixture
与 release note 必须覆盖每项有意的 0.10 变更。

## 明确非目标

0.10 Native toolchain 不包括 cross compilation、fat multi-version library、程序
参数、string、stdin、general byte I/O、dynamic memory、allocator、ownership、
exception、thread、REPL、security sandbox、fast-math、C/WASM runtime print 或公共
embeddable JIT API。这些能力需要独立的 versioned design，不得仅为通过本计划而加入。
