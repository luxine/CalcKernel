# CalcKernel 0.10 原生工具链阶段任务

[English](../../../compiler/native-toolchain-implementation/stage-tasks.md)

> **给 agentic worker：**必须使用 `superpowers:executing-plans`，按顺序执行每个
> checkbox。本计划完全行内执行，不允许使用子代理。

**目标：**把获批的 0.10 原生工具链设计转换为结合仓库实际文件和命令的小粒度、
测试先行实施任务。

**架构：**当前 frontend 与 MIR 继续作为语义中心。唯一结构化 LLVM backend 通过
固定 bridge 提供 inspection IR、native object、library、executable 和 ORC 执行。
CLI 只协调 typed compiler service 和 transactional output，不执行外部工具链。

**技术栈：**Rust 2024、LLVM/ORC/LLD 22.1.8、C++20 bridge、CMake/Ninja、
GitHub Actions 及平台原生验证工具。

---

每项任务开始前必须运行指定红灯测试，确认失败原因是行为尚未实现而不是环境错误。
完成最小实现后运行聚焦 suite，并在绿灯下重构。每个阶段边界执行
[阶段验收](stage-acceptance.md)中的对应部分。

## 阶段 1 — 固定原生依赖与 ownership 边界

### 任务 1.1：声明构建 profile 与精确源码 manifest

**文件：**

- 修改：`Cargo.toml`、`Cargo.lock`、`.gitignore`
- 新建：`build.rs`、`native/llvm/manifest.toml`
- 新建：`scripts/bootstrap-llvm.sh`、`scripts/bootstrap-llvm.ps1`
- 新建：`tests/contracts/native_toolchain.rs`
- 修改：`tests/contracts.rs`

- [ ] 先添加 contract 红灯测试，要求 `native-toolchain` feature、精确 LLVM
  `22.1.8`、source tag 与 SHA-256、host-only target、静态链接 policy、bootstrap
  script 和被忽略的输出。运行
  `cargo test --locked --test contracts native_toolchain_manifest` 并确认缺失项失败。
- [ ] 添加 `cc` build dependency、`sha2` runtime dependency，并且只在标准库不能
  表达 ownership/process 操作时添加 target-scoped system API dependency。不得添加
  `llvm-sys`、Clang、动态加载器或通用 linker command dependency。
- [ ] 两个 bootstrap script 都必须使用显式输入/输出路径、解压前 checksum 校验、
  固定 CMake 配置、宿主 target 选择、release profile 的 Clang 排除断言，并安装供
  `build.rs` 消费的 component manifest。另建隔离的 `oracle` profile，从同一验证
  源码把 Clang 22 构建到不同 prefix，并为 CI 输出精确 executable 路径。
- [ ] feature 关闭时 `build.rs` no-op；开启时要求 `CKC_LLVM_PREFIX`，校验精确
  manifest/version、构建 bridge 并输出 target-specific static link directive。对
  缺失、版本不符或只有 shared library 的 prefix 给出可操作错误。
- [ ] contract 测试转绿，然后在不启用 feature 时运行 `cargo build --locked`，证明
  普通开发不会引入 LLVM。

### 任务 1.2：定义 exception-safe bridge contract

**文件：**`native/bridge/ckc_llvm.h`、`native/bridge/ckc_llvm.cpp`、
`src/backend/llvm/ffi.rs`、`src/backend/llvm/error.rs`、`tests/native.rs`、
`tests/native/bridge.rs`。

- [ ] 添加 C header compile-time assertion 与 Rust 集成红灯测试，覆盖 ABI version、
  LLVM version、target triple、success/error ownership 和配对 string/byte-buffer
  release。
- [ ] header 只暴露 C-compatible opaque handle、integer status、byte span 与 owned
  bridge error；每个导出 bridge 函数捕获所有 C++ exception。C++ object/container、
  exception 或 LLVM class 不得跨 header。
- [ ] 用 typed `NativeError` 包装 raw call，保留失败 stage，并精确一次转换和释放
  bridge-owned error。
- [ ] 在 native CI 的 ASan 下测试非法输入与注入 bridge failure；Rust 必须得到
  typed error，不能 abort 或 unwind。

### 任务 1.3：建立 LLVM/ORC lifetime owner

**文件：**`src/backend/llvm/context.rs`、`target.rs`、`jit.rs`、`mod.rs`，
`src/backend/mod.rs`、`src/lib.rs`、`tests/native/ownership.rs`。

- [ ] 添加反复 create/drop context、target、module、object、empty JIT 的红灯测试，
  包括中途注入错误。
- [ ] safe wrapper 不实现 `Clone`，constructor 校验 target 与 ownership 关系；opaque
  pointer 保持私有。只有 LLVM 明确允许时才实现 `Send`/`Sync`，否则默认均不实现。
- [ ] 按逆序 `Drop` 并显式消费 ORC error。Windows AArch64 选择启用 reserve 的
  RuntimeDyld layer，另五个平台选择 JITLink，通过 typed enum 报告。
- [ ] native CI 在 ASan/LSan 下运行 ownership 重复测试，本地运行聚焦 Rust suite。

### 任务 1.4：报告嵌入工具链与 notice

**文件：**`src/backend/llvm/notices.rs`、`src/cli/mod.rs`、`commands.rs`、
`src/bin/ckc.rs`、`tests/cli.rs`、`tests/cli/commands.rs`。

- [ ] 添加 `ckc --version`、`--version --verbose`、`ckc licenses` 以及 feature-disabled
  developer binary 唯一 native-unavailable error 的 CLI 红灯测试。
- [ ] 构建时嵌入 compiler、LLVM、Native ABI、runtime ABI、host target、enabled
  code generator 与 active ORC object layer；以 bytes 嵌入 LLVM/third-party notice，
  不依赖外部文件即可输出。
- [ ] `src/bin/ckc.rs` 只处理 process argument/exit plumbing，解析和消息留在
  `src/cli`。

## 阶段 2 — Entry、print builtin、effect 与 root

### 任务 2.1：保留并校验 `main`

**文件：**`src/frontend/typeck.rs`、`diagnostics.rs`、
`tests/frontend/checker.rs`、`surface.rs`、`tests/fixtures/native/entry/*.ck`。

- [x] 为每个拒绝形态添加独立红灯测试：有参数、`export`、非法结果、重复 entry，以及
  executable consumer 要求 entry；同时覆盖合法 void/i32 和无 main library。
  diagnostic 使用稳定 `CK` ID 与精确 span。
- [x] 在 checked program 数据中保存 entry classification，不允许 CLI 再按名字解析。
  合法 `main` 在 library/object root 中保持 internal。
- [x] 保留普通 function/export 的 V0.9 行为并运行完整 frontend suite。

### 任务 2.2：添加七个保留 native print symbol

**文件：**`src/frontend/typeck.rs`，必要时 `ast.rs`，
`tests/frontend/checker.rs`、`tests/fixtures/native/print/*.ck`。

- [x] 为精确签名、仅 void statement 使用、arity/type error、用户重定义及七个名字添加
  红灯测试。
- [x] compiler builtin metadata 增加 backend availability 和 observable effect
  identity；不能把 print 建模为 user declaration，也不能让其地址 escape。
- [x] `check` 与 MIR inspection 接受合法调用，source error 仍为普通诊断。

### 任务 2.3：让 entry 与 print effect 贯穿 MIR

**文件：**`src/ir/model.rs`、`lower.rs`、`validate.rs`、`print.rs`、
`tests/ir/mir.rs`。

- [x] 添加 entry metadata、typed print instruction、source-order operand、validator
  拒绝非法 builtin 签名及稳定 inspection text 的红灯测试。
- [x] 引入显式 effectful runtime-call MIR instruction 或同等 typed callee identity；
  不得按名称编码成可被删除的普通 call。
- [x] effect 之前先 lower argument，并保留源码求值顺序。module-wide checked mode 下
  print 仍为 void。

### 任务 2.4：让 optimization 与 artifact-root analysis 感知 effect

**文件：**`src/optimizer/analysis.rs`、`passes/dce.rs`、`inlining.rs`、`cse.rs`、
`loops.rs`、`pipeline.rs`、`tests/optimizer/passes.rs`、
`src/ir/reachability.rs`、`src/ir/mod.rs`。

- [x] 添加 O0-O3 红灯测试，证明 print 不被删除、复制、合并、hoist、sink 或重排，
  包括 inline function 和 loop 内调用。
- [x] 集中 effect classification，并让每个 transformation pass 查询它；增加 entry、
  requested export 与 non-executable print rejection 的 artifact-root reachability。
- [x] 添加 backend 红灯测试：C/WASM 与 native library 在写 artifact 前拒绝可达 print；
  library root 不可达的 print-only function 可以被消除。

## 阶段 3 — 结构化 LLVM lowering 与 verified native object

### 任务 3.1：用结构化 module builder 替换文本拼接

**文件：**替换 `src/backend/llvm/emit.rs`；新建 `module.rs`、`lower.rs`；修改
`layout.rs`、`names.rs`、`mod.rs`、`tests/backend/llvm.rs`；新建
`tests/native/llvm_ir.rs`。

- [x] 逐个 fixture 迁移成检查 LLVM 打印 verified module 的结构化红灯测试，覆盖
  constant、arithmetic、comparison、branch、loop、phi、call、void、struct、pointer、
  slice、index 与 sub-slice。
- [x] 通过 safe wrapper 构建 LLVM value/block，绝不插值生成 IR。布局前由宿主
  TargetMachine 生成 target triple 与 DataLayout。
- [x] `emit-llvm` 打印这个 module，并在打开输出目的地前拒绝 normalized non-host
  target。
- [x] 所有迁移后的 backend 测试通过后才删除旧文本 emission helper。

### 任务 3.2：通过 typed state 完成 verify、optimize 与 codegen

**文件：**新建 `src/backend/llvm/verify.rs`、`passes.rs`、`object.rs`；修改
`context.rs`；修改 `tests/native/llvm_ir.rs`；新建 `tests/native/object.rs`。

- [x] 添加 verifier rejection、O0-O3 pipeline、strict floating-point、baseline/native
  CPU attribute、object magic 与 host-target rejection 红灯测试。
- [x] construction 产生 unverified module；verification 产生 `VerifiedModule`；
  PassBuilder 消费它并在二次 verification 后产生 `OptimizedModule`；只有此状态可被
  TargetMachine 消费为 object bytes。
- [x] MIR 与 LLVM 使用同一 optimization 选择；O3 不能启用 fast-math 或收缩 strict
  operation。
- [x] 暴露 `NativeObject` 前通过 LLVM 解析 object bytes，并在所有错误中保留 stage。

### 任务 3.3：实现 unchecked 与 checked lowering

**文件：**新建 `src/backend/llvm/checked.rs`；修改 `lower.rs`、`module.rs` 与
`tests/native/llvm_ir.rs`。

- [x] 为 overflow/bounds 四组合添加红灯测试：overflow intrinsic、signed div/mod
  zero 与 minimum/-1、slice index、`start <= end <= len`、first-error ordering、void
  result 与 checked result pointer。
- [x] checked control flow lower 成显式 `CK_Status` propagation，不用 trap；unchecked
  code 按现有语义保持无 guard。
- [x] 将代表性 checked module 与固定结构 fixture 及无需执行的 Clang-derived operation
  语义比较。value/status 执行差分推迟到具备合法 in-process execution path 的阶段 4
  library harness 和阶段 5-6 entry harness。required native CI 外缺少 Clang 只能
  显式 skip oracle，绝不能成为产品 fallback。

### 任务 3.4：在产生输出前强制 backend availability

**文件：**`src/cli/args.rs`、`commands.rs`、`output.rs`、
`tests/cli/commands.rs`、`oracle_portability.rs`。

- [x] 添加 `run/build` 默认 O3、inspection 默认 O0、`-O0` 至 `-O3`、checked mode、
  CPU policy、host target normalization 与错误时无部分输出的红灯测试。
- [x] 解析 command-specific option，禁止 unknown/irrelevant flag 在命令间泄漏；
  artifact kind 与 CPU 使用 enum。
- [x] 删除所有产品 target probe 与 Clang invocation；Clang helper 只能留在
  `tests/support/oracle.rs`。

## 阶段 4 — Native C ABI 与 library artifact

### 任务 4.1：定义 target-family ABI classifier

**文件：**新建 `src/backend/native_abi/{mod,model,sysv_x64,darwin_x64,aapcs64,windows}.rs`；
修改 `src/backend/mod.rs`；新建 `tests/native/abi.rs` 与
`tests/fixtures/native/abi/*`。

- [x] 为每个 primitive、pointer、slice、struct size/alignment boundary、aggregate
  parameter/return、bool、checked result 和 target family 添加 table-driven 红灯测试。
- [x] 显式实现 register class、indirect/by-value、extension、alignment 和 hidden-result
  决策；不能查询 host C layout 来推断别的 family。
- [x] 每个 release host 将 LLVM attribute/calling sequence fixture 与固定 Clang 22
  development oracle 比较。

### 任务 4.2：生成并校验 Native C ABI export thunk

**文件：**新建 `src/backend/llvm/abi.rs`、`src/backend/header.rs`；修改
`llvm/module.rs`、`backend/c/emit.rs`、`layout.rs`、`backend/mod.rs`、
`tests/native/abi.rs`、`tests/backend/c.rs`；新建 `tests/native/differential.rs`。

- [x] 添加 native header 对现有 C commitment 的比较红灯测试，并断言 internal LLVM
  signature 不会导出。
- [x] 只把共享 header/layout concept 移入 `backend::header` 与 `native_abi`；C emitter
  特有文本仍在 `backend::c`。
- [x] 在 O3 前插入 external thunk，保留 source symbol 与 visibility；允许 LLVM inline
  internal implementation，但不能删除 public boundary。
- [x] 六个 target job 都用固定 C harness 编译每份生成 header。
- [x] 通过阶段 4 system-FFI loader，将所有 exported scalar、control-flow、void、call、
  struct、pointer、slice 与 checked-ordering fixture 和 `CKC_CLANG_ORACLE` 从 C emission
  产生的 library 做执行差分。

### 任务 4.3：进程内创建 object 与 static artifact

**文件：**新建 `src/backend/artifact/mod.rs`、`archive.rs`；修改 bridge header/cpp、
`src/backend/mod.rs`；新建 `tests/native/artifacts.rs`。

- [x] 添加平台 suffix、object/header pair、deterministic static archive、symbol index、
  拒绝任意输入 object 的红灯测试。
- [x] 通过 trusted bridge 暴露 LLVM archive writer；Rust API 只接受 `NativeObject`
  与 compiler-owned helper identity。
- [x] CLI 接收前校验每个 staged object/archive。

### 任务 4.4：链接 dynamic library 并提交多文件输出

**文件：**新建 `src/backend/artifact/lld.rs`、`platform.rs`；修改 bridge、
`src/cli/output.rs`、`commands.rs`；新建 `tests/native/libraries.rs`；修改 CLI 测试。

- [x] 添加 trusted LLD argument、平台动态库与 Windows import lib、header `CK_API`
  mode、pre-commit failure、commit rollback、symlink rejection 与 cleanup 红灯测试。
- [x] bridge `lld::lldMain` 并捕获 diagnostic，只用 allowlisted argument builder。
  user object/library/linker script/response file/raw flag 均不得进入。
- [x] 实现 same-filesystem staging、逐文件 atomic replace 以及 multi-output backup。
  commit 前失败保持全部 destination 不变；commit 中失败尝试 rollback 并报告。
- [x] 用空外部工具 PATH 经 system FFI 加载动态库，覆盖每个 export shape 与 checked
  组合。

### 任务 4.5：统一 `build` 并弃用 `build-llvm`

**文件：**`src/cli/args.rs`、`commands.rs`、`toolchain.rs`、
`tests/cli/commands.rs`、`oracle_readiness.rs`。

- [x] 添加四种 `--kind`、dynamic 默认、CPU mode、header rule、精确路径、空 PATH 和
  `build-llvm` dynamic/object 单次弃用 warning 的 CLI 红灯测试。
- [x] `build` 直接调用 native backend；executable 无 entry 要拒绝，library/object
  可达 print 在 staging 前拒绝。
- [x] 删除产品 Clang discovery、`.c`/`.ll` 中间件和 fallback message；`emit-c`
  保持只输出源码。

## 阶段 5 — 最小运行时与 standalone executable

### 任务 5.1：实现平台 write/exit 与稳定 runtime failure

**文件：**新建 `native/runtime/include/ckc_runtime.h`、
`common/runtime.c`、`linux/syscalls.S`、`darwin/process.c`、
`windows/process.c`、`provenance.toml` 与 `tests/native/runtime.rs`。

- [x] 添加 `CKR0001` 至 `CKR0006` byte-exact 文本、240-245 exit code、stdout failure
  fallback、无 heap import 与全平台 LF 红灯测试。
- [x] 只实现 bounded stack write 与平台 process API；禁止 allocation、locale、libc
  formatting、CK dynamic runtime 和 process crash handler。
- [x] bootstrap 时编译 runtime/entry object、记录 hash，并在 Cargo 构建时将宿主 bytes
  嵌入 `ckc`。

### 任务 5.2：实现无 allocation numeric formatting

**文件：**新建 `native/runtime/common/format_int.c`、`format_float.c`，在
`native/runtime/vendor/` 放置 vendored algorithm 与 license；修改 provenance 与
runtime tests。

- [x] 测试 integer extrema、bool、无 newline value function、`print_newline`、finite
  f64 shortest-round-trip、halfway、subnormal、infinity、NaN 与保留 `-0.0`。
- [x] vendor permissive licensed 的 bounded-buffer shortest-round-trip algorithm，
  保留原 notice，并改造成不使用 heap、locale、static mutable state 或 libc formatting。
- [x] 每个 finite spelling parse-back 后必须得到相同 f64 bits，只有文档规定的 NaN
  payload/sign 丢失例外。

### 任务 5.3：构建 entry wrapper 与 standalone executable

**文件：**新建 `src/backend/llvm/entry.rs`；修改 `module.rs`、artifact platform/lld；
在 `native/runtime/platform/` 添加 link input；修改 artifact tests 并新建
`tests/native/executable.rs`。

- [x] 添加 void/i32 main、checked result pointer、propagated runtime diagnostic、
  application exit、无 main、print reachability 与无 header 红灯测试。
- [x] 生成 compiler-owned process entry wrapper，只链接 verified program object、
  embedded runtime/entry/helper、allowlisted export 与 embedded platform import metadata。
- [x] Linux 使用 syscall boundary；Windows 使用 stable import definition，且仅 DLL
  用 `/noentry`；Darwin 使用固定最小 libSystem text stub、显式 platform version 与
  LLD ad-hoc signing。
- [x] 在空外部工具 PATH 下执行 artifact，对比 runtime contract 的 stdout、stderr
  与 exit status。

### 任务 5.4：证明零运行时依赖

**文件：**新建 `scripts/audit-native-artifact.sh`、`.ps1`；修改 artifact tests 与
runtime provenance。

- [x] 审计 ELF `DT_NEEDED`、Mach-O load command、PE import、exported symbol、禁止的
  LLVM/LLD/Clang/CK 名称、compiler helper 与 runtime hash drift，先观察红灯。
- [x] 只允许设计列出的 platform loader/API dependency；所需 permissive compiler
  helper 静态链接并加入 notice。
- [x] 分别审计 object、static、dynamic 与 executable。

## 阶段 6 — ORC 执行、parent/child 隔离与 cache

### 任务 6.1：用 ORC 链接并执行同一 native object

**文件：**修改 `src/backend/llvm/jit.rs` 与 bridge；新建 `tests/native/jit.rs`。

- [ ] 添加 eager resolution、entry lookup、embedded runtime symbol、checked 四组合、
  无 lazy hot-function stub 与 object-layer 选择红灯测试。
- [ ] ORC 消费与 AOT 完全相同的 O3 `NativeObject`；五平台用 JITLink，Windows
  AArch64 用 reserve-enabled RuntimeDyld/SectionMemoryManager。
- [ ] 调用 entry 前解析所有 symbol，并在执行用户代码前返回 typed compile/link/lookup
  error。

### 任务 6.2：实现 private child 与 public run parent

**文件：**新建 `src/cli/run.rs`；修改 CLI mod/commands/args、thin binary；新建
`tests/native/run.rs` 并修改 CLI tests。

- [ ] CLI 红灯测试证明 parent 以不可伪造/private child mode self-spawn 同一 executable，
  继承 program stdio、不输出成功文本、返回 normal status、转发 interrupt，并把可识别
  signal/exception 映射成精确 `CKR0006`。
- [ ] compilation、cache、ORC 与 user machine code 全在 child；parent 校验私有协议，
  绝不加载生成代码。
- [ ] 区分 compiler failure、normal checked failure、program exit、output failure 与
  abnormal termination，不覆盖更具体 status。

### 任务 6.3：定义 canonical cache key 与 validated entry

**文件：**新建 `src/cli/cache/mod.rs`、`key.rs`、`entry.rs`；修改 CLI mod；新建
`tests/native/cache.rs`。

- [ ] 为 versioned canonical serialization 与 lowercase SHA-256 name 添加固定向量红灯
  测试；逐个改变所有语义输入，证明 key 改变。
- [ ] 用架构无关格式编码 length/integer；不得 hash debug output、unordered map 顺序、
  path、timestamp 或 host-native integer bytes。
- [ ] 保存 bounded manifest、object bytes 与二者 digest；hit 前校验 size、version、
  key、digest 与 LLVM object parsing。

### 任务 6.4：安全 cache storage、eviction 与 clean

**文件：**新建 `src/cli/cache/path.rs`、`store.rs`、`evict.rs`；修改 CLI；扩展 cache
和 commands tests。

- [ ] 添加三 OS path、缺失 base dir、owner-only creation、不安全 owner/permission、
  symlink、corruption、并发 writer、atomic rename、1-GiB soft limit、deterministic
  best-effort LRU、`--no-cache` 与 `ckc cache clean` scope 红灯测试。
- [ ] 非法 entry 或 unsafe root 一律视为 miss；required base 无法解析时关闭 cache，
  不创建全局可写路径，cache maintenance 失败也不能让合法 source run 失败。
- [ ] 使用 owner-checked same-filesystem temp，以及宿主可用时的 no-follow/open-new。
  clean 只能删除解析出的 CK cache root。

### 任务 6.5：证明 JIT memory-protection 行为

**文件：**扩展 bridge；修改 `tests/native/jit.rs`；新建
`scripts/audit-jit-memory.sh`、`.ps1`。

- [ ] Linux/Windows 宿主测试观察 relocation 期间 writable/non-executable、final
  read/execute code 与 non-executable data；Windows AArch64 包括 instruction-cache
  finalization。
- [ ] Darwin 测试 `MAP_JIT` 与 per-thread write-protection transition，不能仅因 mapping
  最大 permission 同时含 write/execute 就拒绝。
- [ ] signed/hardened macOS release candidate 只能使用狭义所需 JIT entitlement。

## 阶段 7 — 性能、CI、发布与法律闭环

### 任务 7.1：构建严格差分性能 harness

**文件：**修改 benchmark/performance tests；增加 native performance fixtures；新建
`scripts/check-native-performance.py`。

- [ ] 添加 reference equivalence、warm-up、sample stability、geometric mean、单项
  regression threshold、checked/unchecked 分离、CPU policy 与拒绝 fast-math reference
  的 harness 红灯测试。
- [ ] C reference 用固定 Clang strict `-O3`；CK 用相同 baseline/native 的 native
  TargetMachine O3。批量 FFI call，并分别报告 compilation、cold/warm run、memory、
  artifact size 与 throughput。
- [ ] geometric mean 门槛 95%，每 kernel 不得慢超过 10%；例外只能作为经审查、可复现
  的 target limitation 加入 normative release evidence，不能藏在 harness。

### 任务 7.2：添加固定 native integration CI

**文件：**修改 `.github/workflows/ci.yml`；新建
`.github/actions/bootstrap-ckc-llvm/action.yml`；修改 CI/native contract tests。

- [ ] workflow contract 红灯测试要求精确 manifest/checksum、cached host bootstrap、
  fast non-native quality job、native all-feature lint/test、六宿主功能矩阵及受控
  x86-64/AArch64 性能 worker。
- [ ] fast job 不依赖 LLVM 并移除错误的 `--all-features`；required native job 运行
  fmt、all-feature clippy、全部测试、支持平台的 bridge sanitizer、artifact/dependency
  audit、JIT permission 与 cache/process suite。
- [ ] 按发布政策固定 action 与外部工具版本。CI 可获取 checksum-verified LLVM source，
  但不能接受 runner 任意 system LLVM。

### 任务 7.3：生成完整六宿主 release archive

**文件：**修改 native release workflow、release contract tests、中英 release policy
与 checklist。

- [ ] release contract 红灯测试要求 native feature build、六个精确 archive 名称、
  checksum sidecar、verbose version evidence、notice、zero-dependency audit、run/build
  smoke、macOS signing/JIT 与 immutable GitHub release。
- [ ] 每个 host bootstrap target-minimal static LLVM/ORC/LLD，构建唯一 self-contained
  `ckc`；包含 dynamic LLVM/LLD/Clang 或 non-system C++ runtime dependency 的 archive
  一律拒绝。
- [ ] 保持现有 archive 名称，只在六份 artifact 与 checksum 都验证后 publish。

### 任务 7.4：闭合源码与 license provenance

**文件：**新建 `THIRD_PARTY_NOTICES.md`；修改 notices、LLVM manifest、runtime
provenance 与 native contract tests。

- [ ] 测试枚举每个 embedded/statically linked third-party component，对比 source hash、
  license、notice 与 `ckc licenses` 输出，先观察红灯。
- [ ] 缺失、过期或无引用 provenance 必须同时让 source build 与 release CI 失败。

## 阶段 8 — 0.10 contract 与仓库冻结

### 任务 8.1：更新规范性双语 contract

**文件：**成对修改 `docs/reference/`、`docs/abi/`、`docs/compiler/`、`docs/guides/`、
`docs/project/`，中英 index、README、CHANGELOG 与 docs contract tests。

- [ ] 先让 contract tests 要求当前 0.10 language、CLI、MIR、Native C ABI、runtime、
  compatibility、security、build、performance 与 release wording，以及递归双语镜像和
  有效链接。
- [ ] 替换已失效 V0.9-only promise，不保留设计历史叙事。明确 C/WASM 行为，不能暗示
  它们支持 print。
- [ ] 可行的 example 必须可执行，并运行全部 docs contract tests。

### 任务 8.2：设定 0.10.0 并冻结兼容 fixture

**文件：**修改 Cargo metadata/lockfile、compatibility fixtures、repository/release
contract tests。

- [ ] 添加 Cargo metadata、lockfile、README、changelog、`ckc --version`、verbose ABI
  revision、release tag rule 与 archive metadata 的版本一致性红灯测试。
- [ ] 所有行为存在后才设置 `0.10.0`。为设计列出的每项有意 compatibility change
  添加 fixture，并保持不受影响的 V0.9 source 行为。

### 任务 8.3：执行总验收并准备审查分支

**文件：**只能修改由真实 implementation/contract defect 导致失败所需的文件。

- [ ] 在 clean worktree 和最新 bootstrap evidence 下执行[总验收](final-acceptance.md)；
  不能用本地推断把 remote-host 项标为通过。
- [ ] 运行 `git diff --check`，审查完整 diff/commit graph，扫描 placeholder、ignored
  test、禁止的 external tool call，并确认无 LLVM build output 被跟踪。
- [ ] 提交完整分支。不得 merge、tag、publish、删除 worktree 或修改 `main`；向所有者
  报告 branch、final commit 和尚需外部 CI 提供的证据。
