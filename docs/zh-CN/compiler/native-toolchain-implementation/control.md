# CalcKernel 0.10 原生工具链实施总控

[English](../../../compiler/native-toolchain-implementation/control.md)

> **给 agentic worker：**必须使用 `superpowers:executing-plans`，逐项执行本计划。
> 用 checkbox 跟踪每项任务并完全行内执行；本轮实施不允许委派或使用子代理。

**目标：**在六个发布宿主上交付已批准的、自包含的 CalcKernel 0.10 原生工具链，
并且不削弱语言、Native C ABI、运行时、性能或发布门禁。

**架构：**保留当前 frontend、MIR、optimizer、C backend、WebAssembly backend、
轻量 CLI 和按职责划分的测试布局。把仅输出文本的 LLVM 路径替换为唯一的结构化
LLVM module builder，并让 verification、PassBuilder、TargetMachine、ORC、archive
writer、LLD 与 `emit-llvm` 共用它。LLVM/LLD 的 C++ 接口限制在窄 C ABI 后面。
运行时与平台链接输入由 compiler 持有并嵌入发布二进制。

**技术栈：**Rust 2024、LLVM/ORC/LLD 22.1.8、窄 C++20 bridge、用于固定原生
依赖构建的 CMake 与 Ninja、GitHub Actions、平台 object/dependency 检查工具，以及
仅用作开发 oracle 的固定 Clang。

---

## 权威与范围

获批的[原生工具链设计](../native-toolchain-design.md)是语义和架构权威。这里的实施
文档只把设计细化成可执行工作，不能扩张或削弱设计。如果代码暴露出真实矛盾，必须
先同步修订设计及其中英版本，说明原要求为何不可实现或不安全，然后重跑受影响的验收
门禁。实现失败不能成为降低门槛的理由。

本轮工作的直接授权要求提交总控、执行和验收文档，即使仓库的一般约定要求临时计划
保留在本地。因此这些文件是 0.10 线的 maintainer-facing 执行 contract，不是带日期
的 review 记录或历史叙事。只能在冻结 0.10 contract 时删除或转化它们。

工作在独立 `.worktrees/native-toolchain-0.10` worktree 的
`feat/native-toolchain-0.10` 上进行。任何阶段都不得 merge、tag、publish 或修改
`main`。完成后的分支保留给仓库所有者审查。

## 不可妥协的执行规则

- 严格按下面的阶段顺序工作。当前阶段的验收命令必须以最新输出通过后，才能开始下一
  阶段。
- 每项行为都使用 TDD：加入最小而有意义的失败测试，观察符合预期的红灯，实现最小
  production 变更，观察测试通过，然后在绿灯下重构。
- production code 绝不能依赖 Clang、`clang`、`llvm-config`、LLD executable、
  平台 linker、`ar` 或首次运行网络下载。
- 仓库 bootstrap 可以在生成 `ckc` 时执行构建工具；发布版 `ckc` 运行时不得执行或
  加载这些工具。
- 保持 `src/bin/ckc.rs` 轻量。compiler 工作位于 `src/frontend`、`src/ir`、
  `src/optimizer`、`src/backend` 和 `src/cli`；原生 bridge 与嵌入式运行时源码位于
  `native/`。
- production 路径不得使用 `unwrap` 或 `expect`，除非局部 invariant 已证明失败
  不可能。每个 `unsafe` block 都要有精确的安全说明，并在最近的 safe boundary
  放置聚焦测试。
- 不留下 placeholder、被 ignore 的验收测试、关闭的门禁、宽泛 lint 抑制或未跟踪
  工作产物。
- 只有阶段门禁通过后才提交。阶段 commit 要聚焦，并保持阶段之间 worktree clean。

## 构建 profile 与依赖边界

crate 提供 `native-toolchain` Cargo feature。普通 frontend、MIR、C、WebAssembly
和 contract 测试不启用该 feature，也不 bootstrap LLVM。启用 native 的源码构建
要求 `CKC_LLVM_PREFIX` 指向仓库为 LLVM 22.1.8 生成的 bootstrap install。
`build.rs` 校验精确版本，并只静态链接 bootstrap manifest 列出的宿主组件；绝不
静默接受系统 LLVM。

发布 archive 始终用 `--features native-toolchain` 构建；未启用该 feature 的
binary 是开发 compiler，必须用一个明确的 availability error 拒绝 `run`、原生
`build` 和原生 `emit-llvm`。它仍支持 `check`、`emit-mir`、`emit-c`、
`emit-wat` 与 `emit-wasm`。任何 archive 或正式发布都不得包含开发版形态。

仓库持有：

- `native/llvm/manifest.toml`：LLVM tag、源码 URL、archive SHA-256、CMake switch、
  target-specific component allowlist 和 notice 输入；
- `scripts/bootstrap-llvm.sh` 与 `scripts/bootstrap-llvm.ps1`：将经过 checksum 校验的
  宿主 bootstrap 确定性安装到显式 prefix；
- `native/bridge/`：封闭 exception 的 C++ bridge 和 C header；
- `native/runtime/`：无 heap 运行时、entry object、平台 import metadata、export
  list、源码 provenance、hash 与 license。

manifest 还定义与发布隔离的 `oracle` bootstrap profile：从同一份经过 checksum 校验
的源码构建 Clang 22 driver，只供必需的 ABI、differential 与 performance 测试使用。
显式 `CKC_CLANG_ORACLE` 路径只能由 test/benchmark support 接受；不得从 `PATH`
搜索、链接进 `ckc`、复制进 release prefix，也不是任何产品命令的依赖。release
profile 继续断言排除 Clang。

bootstrap 输出位于被忽略的 `build/`，绝不提交。bootstrap 接受已下载的 source
archive 以支持离线构建；网络是显式 developer/CI 获取步骤，绝不隐含在 Cargo 或
`ckc` 内。

## 仓库映射

| 职责 | 当前锚点 | 0.10 目标 |
| --- | --- | --- |
| 源码规则与 builtin | `src/frontend/typeck.rs` | entry 校验与七个保留 print symbol |
| MIR effect 与 root | `src/ir/model.rs`、`src/ir/lower.rs`、optimizer pass | print effect、entry/library 可达性与保留规则 |
| LLVM lowering | `src/backend/llvm/` | 结构化 builder、checked lowering、ABI thunk、target/object/JIT owner |
| C ABI 权威 | `src/backend/c/layout.rs`、C header emitter | 共享 target ABI model 与原生 header 生成 |
| Artifact 组装 | `src/cli/commands.rs`、`src/cli/output.rs` | backend artifact API 与多输出 transaction commit |
| Run/cache/process | `src/cli/` | parent/child protocol、cache、signal/exception 映射 |
| 原生 foreign boundary | 无 | `native/bridge/` 与 safe Rust wrapper |
| Runtime/link 输入 | 无 | 由原生构建嵌入的 `native/runtime/` |
| 集成证据 | `tests/` 中的责任测试 | `tests/native/` 以及扩展的 CLI/backend/contracts/performance suite |
| 发布证据 | 两个 GitHub workflow | 固定的原生集成 job 与六宿主发布 job |

当前 C layout 与 checked-status 行为是测试 oracle，不能盲目复制代码。只有两个
backend 都需要同一 invariant 时才抽取共享 ABI concept；backend-specific emission
保持分离。

## 阶段图

各阶段有意保持串行，因为每一阶段都消费前一阶段稳定下来的 artifact 或 contract。

| 阶段 | 交付物 | 进入依赖 | 退出证据 |
| ---: | --- | --- | --- |
| 1 | 固定 LLVM bootstrap、bridge、safe ownership wrapper | V0.9 baseline | bridge smoke test 与静态 component audit |
| 2 | `main`、print builtin、MIR effect 与 artifact root | stage 1 类型可用 | frontend/MIR/optimizer semantic suite |
| 3 | 结构化 LLVM lowering、verification、optimization 与 codegen | stage 1-2 | 结构化 IR、object validity 与 checked-CFG suite |
| 4 | Native C ABI thunk 和 object/static/dynamic build | stage 3 object emission | 六 ABI family oracle 与可执行 library differential suite |
| 5 | 最小运行时与 standalone executable | stage 4 LLD/artifact assembly | run-equivalent executable suite 与 dependency audit |
| 6 | ORC child execution 与 persistent cache | stage 5 runtime object | process、cache、memory-protection 与 output suite |
| 7 | 性能、CI、发布包装与 notice | stage 1-6 | 受控性能与六宿主发布矩阵 |
| 8 | 0.10 contract freeze | 所有实施阶段 | 完整总验收与版本一致性 |

精确的红/绿工作单元和文件列表见[阶段任务](stage-tasks.md)。阶段退出命令与证据见
[阶段验收](stage-acceptance.md)。只有[总验收](final-acceptance.md)通过，分支才算完成。

## 稳定内部边界

实施在增加高层行为前建立以下内部 API：

- `backend::llvm::NativeToolchain`：宿主专用 compiler owner，具有显式
  context/module/target/JIT lifetime 和 typed error；
- `backend::llvm::CodegenOptions`：optimization、checked mode、CPU policy、
  artifact intent 与 host triple，不允许自由 target/linker flag；
- `backend::llvm::VerifiedModule`：只能在 LLVM verification 通过后构造；
- `backend::llvm::OptimizedModule`：只能由选定 PassBuilder pipeline 运行并再次验证
  后构造；
- `backend::llvm::NativeObject`：verified object bytes 以及 target/ABI metadata，
  绝不是任意 user object；
- `backend::native_abi`：export thunk 与 integration fixture 共享的显式 target-family
  classifier 和 header contract；
- `backend::artifact`：由 compiler-owned input 完成 object/archive/LLD assembly；
- `cli::run`：public parent 与 private child protocol；
- `cli::cache`：canonical key、validated entry、atomic store、eviction 和 clean operation。

opaque LLVM/ORC pointer 保持私有。safe wrapper 不实现 `Clone`，在类型中携带 ownership
关系，并按逆序释放。C++ bridge 捕获所有 exception，通过配对释放函数返回 owned
error message。Rust 绝不跨 bridge unwind。

## 证据与提交纪律

每个红/绿任务都要在本地执行记录中保留命令与红灯原因；提交的证据是测试本身，而非
transcript。每阶段末尾运行聚焦门禁，然后运行：

```text
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
git diff --check
```

原生阶段还要使用固定 prefix 与 `--features native-toolchain` 运行同类检查。
`cargo clippy --all-features` 由具备 prefix 的 native integration CI 负责；fast job
不得假装验证不存在的 native bridge。

commit subject 使用 `native(stage-N): <completed capability>`。修复真实 contract
问题的文档变更要先于依赖它的代码提交。最终 commit 只包含集成/freeze 变更，不混入
无关 cleanup。

## 停止条件

遇到下列任一情况时，停止 implementation 并修复 governing document：

- 某个获批行为无法用固定 LLVM/ORC/LLD interface 在六宿主之一实现；
- 某平台要求设计禁止的 runtime 或 SDK dependency；
- 生成的 Native ABI 行为无法与文档中的 C ABI 协调；
- W^X、signing、cache ownership、transactional output 或 abnormal-child 规则无法由
  自动化平台测试证明；
- 排除 benchmark noise 与 reference 不等价后，性能结果仍不满足门禁。

缺少本地 release host 不能成为豁免证据的理由。host-specific 验收保持 pending，
直到所需 CI worker 提供结果。
