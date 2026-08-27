# CalcKernel 0.10 原生工具链阶段验收

[English](../../../compiler/native-toolchain-implementation/stage-acceptance.md)

本文档是[阶段任务](stage-tasks.md)每个阶段的强制退出门禁。所有命令从仓库根目录运行。
只有当前 commit 的最新命令输出才算通过；旧结果、推断的平台结果或 ignored test 都不是
证据。

## 每阶段共同门禁

non-native 阶段或 feature-disabled 兼容检查运行：

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
git diff --check
```

native-enabled 阶段把 `CKC_LLVM_PREFIX` 设为当前宿主 checksum-verified 22.1.8
bootstrap，并额外运行：

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --features native-toolchain --locked
```

任何命令都不能访问未固定的系统 LLVM。测试与发布 log 必须打印 bootstrap manifest
digest 和 bridge 报告的 LLVM version。

## 阶段 1 退出 — dependency 与 bridge

```bash
cargo test --locked --test contracts native_toolchain
cargo test --all-features --locked --test native bridge
cargo test --all-features --locked --test native ownership
./target/release/ckc --version --verbose
./target/release/ckc licenses
```

通过条件：

- feature-disabled 构建/测试不需要 `CKC_LLVM_PREFIX`；
- feature-enabled 构建拒绝缺失、错误版本或 shared-only prefix；
- 合法构建报告 LLVM 22.1.8、normalized host、正确 code generator，以及 JITLink 或
  Windows AArch64 RuntimeDyld layer；
- exception injection 与重复 lifetime 测试在平台 sanitizer 下无 unwind、leak、
  double-free 或 stale handle；
- 链接后的 `ckc` 无 dynamic LLVM/LLD/Clang dependency；
- 外部文件全部缺失时仍能输出 embedded notice。

## 阶段 2 退出 — source 与 MIR 语义

```bash
cargo test --locked --test frontend
cargo test --locked --test ir
cargo test --locked --test optimizer
cargo test --locked --test backend control_void_slice
```

通过条件：

- entry 只接受无参数、非 export 的 `main -> void|i32`；
- 七个 print builtin 签名精确且不可重定义；
- MIR 显式表示 print effect 与 entry metadata 并验证它们；
- O0-O3 在 call、loop 与 inline 后保留 print 数量和源码顺序；
- root analysis 在输出前拒绝 C/WASM/native non-executable artifact 可达 print，
  run/executable 则允许；
- 现有 frontend、MIR、optimizer、C 和 WebAssembly 测试全部通过。

## 阶段 3 退出 — 结构化 LLVM 与 object code

```bash
cargo test --all-features --locked --test backend llvm
cargo test --all-features --locked --test native llvm_ir
cargo test --all-features --locked --test native object
cargo test --all-features --locked --test cli emit_llvm
```

通过条件：

- 每个代表 fixture 在 PassBuilder 前后都通过 verification；
- `emit-llvm` 是 object emission 共用结构化 module 的 LLVM rendering，并在写入前
  拒绝 non-host target；
- O0-O3 选择一致 MIR/LLVM pipeline，O3 无 fast-math flag；
- baseline object 只使用规定的 mandatory ISA，native object 记录完整 CPU feature；
- checked 四组合具有 verified status CFG、first-error order 与预期 guard absence/
  presence；执行差分证据明确由阶段 4-6 负责；
- 产品源码/可执行文件无 Clang probe、invocation 或 fallback。

## 阶段 4 退出 — ABI 与 library

每个 release host 运行：

```bash
cargo test --all-features --locked --test native abi
cargo test --all-features --locked --test native artifacts
cargo test --all-features --locked --test native libraries
cargo test --all-features --locked --test native differential
cargo test --all-features --locked --test cli build
```

通过条件：

- 宿主 ABI classifier 对所有 export shape 与 checked result 匹配固定 Clang 22；
- generated header 可作为 C11 编译，且描述真实 export thunk；
- object/static/dynamic 输出通过校验并使用正确平台名；
- 空工具 PATH 下经 system FFI 加载 dynamic library，所有 shape 与 checked 组合工作；
- exported scalar、control-flow、void、call、struct、pointer、slice 与 checked fixture
  匹配隔离的固定 Clang 22 oracle library；
- LLD 只接收 compiler-produced/compiler-owned input；
- 注入 pre-commit 与 commit failure 证明无 partial file 且 rollback 成功；
- `build` 默认 dynamic，四种 kind 可解析；`build-llvm` 只为兼容形态输出一次警告。

## 阶段 5 退出 — runtime 与 executable

每个 release host 运行：

```bash
cargo test --all-features --locked --test native runtime
cargo test --all-features --locked --test native executable
cargo test --all-features --locked --test native artifacts
./scripts/audit-native-artifact.sh target/native-acceptance
```

Windows 使用 PowerShell audit。

通过条件：

- numeric spelling、newline、runtime message 与 status byte-exact；finite f64 可 round
  trip，且保留 `-0.0`；
- stdout failure 在无 heap/formatting runtime 下产生 `CKR0005`；
- void/i32 与 checked main wrapper 符合 contract；
- 空 external-tool PATH 下 executable 可运行，且不需要 CK、LLVM、LLD、Clang、libc
  formatting 或外部 compiler runtime；
- object/archive/library/executable/import metadata/runtime object/helper 均通过
  provenance 与 dependency audit；
- Darwin output ad-hoc signed 且按声明 platform version 可运行/加载；Windows
  computation DLL 无 runtime entry。

## 阶段 6 退出 — run、process、cache 与 JIT protection

每个 release host 运行：

```bash
cargo test --all-features --locked --test native jit
cargo test --all-features --locked --test native run
cargo test --all-features --locked --test native cache
cargo test --all-features --locked --test cli run
./scripts/audit-jit-memory.sh target/release/ckc
```

Windows 使用 PowerShell audit。

通过条件：

- ORC eager 执行与 AOT 相同的 O3 object，entry 前解析所有 symbol，object layer
  报告与平台一致；
- public parent/private child、stdio、interrupt、normal status、checked failure 与
  精确 `CKR0006` 映射均通过；成功 run 不写 compiler status；
- cold miss、warm hit、bypass、corruption、permission、symlink、concurrent writer、
  atomic store、eviction 与 clean 均不改变程序语义；
- cache key vector 覆盖所有影响 object 的输入并跨进程稳定；
- Linux/Windows 证明 RW-to-RX 与 NX data，包括 Windows AArch64；Darwin 在签名发布
  policy 下证明 thread-level JIT write protection。

## 阶段 7 退出 — 性能与发布

受控宿主运行：

```bash
cargo bench --features native-toolchain --bench ckc_perf
python3 scripts/check-native-performance.py target/ckc-perf/results.json
cargo test --locked --test contracts ci
cargo test --locked --test contracts release
cargo test --locked --test contracts native_toolchain
```

通过条件：

- 受控 x86-64/AArch64 上 strict native O3 geometric mean 至少为等价 strict Clang C
  O3 的 95%；
- 没有 kernel 慢超过 10%，除非 release evidence 中存在获批、可复现 target limitation；
- checked/unchecked 分别门禁，reference 使用等价 CPU/float 语义；
- required native integration 覆盖六宿主，fast job 不依赖 LLVM bootstrap；
- 六 archive 保持现有名称/checksum，各包含唯一完整 native-enabled `ckc`，可输出
  notice、通过 dependency audit，并只作为完整 immutable set 发布。

## 阶段 8 退出 — contract freeze

```bash
cargo test --locked --test contracts
cargo test --locked
cargo test --all-features --locked
cargo doc --all-features --no-deps
./target/release/ckc --version --verbose
./target/release/ckc licenses
```

通过条件：

- Cargo、lockfile、CLI、README、changelog、tag rule、ABI revision 与双语文档统一为
  0.10.0；
- 每项有意 compatibility change 有 fixture，未受影响 V0.9 source 保持兼容；
- 中英树镜像、local link 可解析且无失效 promise 保持 normative；
- 无 placeholder、ignored acceptance test、native external-tool invocation、tracked
  generated product 或无关 worktree change；
- 已准备在 clean commit 上执行[总验收](final-acceptance.md)。
