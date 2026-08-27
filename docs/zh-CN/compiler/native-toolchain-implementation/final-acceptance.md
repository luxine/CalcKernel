# CalcKernel 0.10 原生工具链总验收

[English](../../../compiler/native-toolchain-implementation/final-acceptance.md)

这是由[总控](control.md)管理的实施总 release-candidate 门禁。每一项都必须通过。
platform 项只有在指定宿主基于 candidate commit 产生证据后才算完成。

## Candidate 身份

- [ ] `feat/native-toolchain-0.10` worktree clean，candidate commit 已记录在 CI run。
- [ ] 本工作没有移动 `main`；未发生 merge、tag、GitHub Release 或 publish。
- [ ] build evidence 含 LLVM bootstrap manifest digest、LLVM 22.1.8 source checksum、
  runtime input hash、bridge ABI、Native ABI revision 与 runtime ABI revision。
- [ ] 八个阶段门禁都有最新通过证据，且没有 waiver 改变设计要求。

## 源码质量与仓库 contract

适用时分别在 feature-disabled developer profile 与 native-enabled profile 运行：

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo test --all-features --locked
cargo doc --all-features --no-deps
cargo build --release --locked
cargo build --release --features native-toolchain --locked
git diff --check
git status --short
```

- [ ] production source 不调用或探测 Clang、LLVM/LLD executable、platform linker、
  `ar` 或 network downloader。
- [ ] 无 placeholder、`todo!`、`unimplemented!`、无解释 lint exception、ignored
  acceptance test、generated bootstrap output 或未跟踪的必要 source。
- [ ] 每个 unsafe block 说明局部 invariant，并在最近 safe boundary 有测试。
- [ ] 中英 Markdown tree 镜像且所有链接可解析。

## 语义差分矩阵

在有意义的 O0-O3 与 checked 四组合下验收：

- [ ] scalar signed/unsigned integer、f64 与 bool operation；
- [ ] branch、loop、`break`、`continue`、void call 与 nested call；
- [ ] struct field 与所有 target layout boundary；
- [ ] raw pointer、slice、index、`.data`、`.len` 与 sub-slice；
- [ ] overflow、division/modulo zero 与 minimum/-1 ordering；
- [ ] slice bounds 与 `start <= end <= len` ordering；
- [ ] entry void/i32 return 与 checked propagation；
- [ ] 七个 print function 的精确顺序、格式、failure 与 forbidden-backend reachability。

只要 C contract 定义了行为，每行都必须在 structural LLVM 与 strict C/Clang 开发
oracle 间一致。每个 fixture 在 optimization 前后都必须通过 LLVM verifier。

## Native ABI 与 artifact 矩阵

下列每个 target 在空 external-tool PATH 下测试 object、static、dynamic 与 executable，
编译 generated header，并通过 system FFI harness 加载 dynamic library。

| Target | Host runner | ABI family | ORC layer | 必须通过 |
| --- | --- | --- | --- | --- |
| `darwin-arm64` | macOS 15 AArch64 | Darwin AAPCS64 | JITLink | [ ] |
| `darwin-x64` | macOS 15 Intel | Darwin x86-64 | JITLink | [ ] |
| `linux-arm64` | Ubuntu 24.04 AArch64 | SysV AAPCS64 | JITLink | [ ] |
| `linux-x64` | Ubuntu 24.04 x86-64 | SysV AMD64 | JITLink | [ ] |
| `win32-arm64` | Windows 11 AArch64 | Windows ARM64 | RuntimeDyld | [ ] |
| `win32-x64` | Windows Server 2025 x86-64 | Windows x64 | JITLink | [ ] |

每个 target 均要求：

- [ ] ABI classifier/generated thunk 匹配固定 Clang fixture；
- [ ] baseline/native CPU policy 与 host-only target rejection 通过；
- [ ] dynamic library 只导出 requested CK symbol 与 required metadata；
- [ ] dependency audit 无 CK、LLVM、ORC、LLD、Clang、formatting runtime 或 non-system
  C++ runtime dependency；
- [ ] executable 与 `ckc run` 的 stdout、stderr、normal/checked status 和 numeric
  formatting 一致；
- [ ] cache miss/hit/bypass/corruption/permission/concurrency/eviction/clean 通过；
- [ ] JIT permission 与 instruction-cache finalization 通过。

Darwin 还要求 LLD ad-hoc-signed output 可运行/加载，以及 signed hardened `ckc run`
只使用批准的 JIT entitlement。Windows 要求 computation DLL `/noentry`、import lib
校验与正确 exception 映射。Linux 要求 syscall-only runtime import 证据。

## Process、cache 与 transactional failure injection

- [ ] public `run` 只能 self-spawn 同一 candidate binary 的 private child protocol；
  无 persistent compiler process 执行生成代码。
- [ ] signal/Windows exception 精确映射 `CKR0006`，normal/checked status 不变。
- [ ] 成功 run 的 stdout 完全属于 CK program，不输出 status message。
- [ ] object cache canonical vector 覆盖全部规定输入；unsafe/corrupt entry 视为 miss，
  绝不改变执行语义。
- [ ] cache/output write 抵御已测试 symlink、permission 与 concurrent replacement。
- [ ] 注入 pre-commit failure 后 destination 全部不变；注入 multi-file commit failure
  后恢复 backup，或报告每个未恢复路径。

## Runtime 与安全边界

- [ ] runtime 不做 heap allocation，只在需要 import 时导入批准的稳定 OS process API。
- [ ] `CKR0001` 至 `CKR0006` message/status byte-exact。
- [ ] numeric edge vector 全部通过，包括 shortest finite f64 round trip、subnormal、
  infinity、NaN spelling 与 negative zero。
- [ ] Linux/Windows 证明 relocation 时 writable/non-executable，最终 code read/execute、
  data non-executable；Windows AArch64 覆盖 reserve-enabled RuntimeDyld。
- [ ] Darwin 证明 per-thread JIT write protection，不能错误拒绝 `MAP_JIT` 最大权限。
- [ ] raw pointer/unchecked failure 由 child process 隔离，但不能描述成 memory safety
  或 sandbox。

## 性能门禁

受控 x86-64 与 AArch64 worker 分别运行语义严格等价的 native 和 Clang C O3，且
checked/unchecked 分开：

- [ ] reference source、input、CPU feature、float rule、iteration 与 output validation
  相同；
- [ ] native geometric mean throughput 至少为 strict Clang C O3 的 95%；
- [ ] 无单个 kernel 慢超过 10%，除非 candidate evidence 附带获批且可复现的 target
  limitation；
- [ ] compilation latency、cold run、warm cache hit、peak memory、artifact size 与
  steady-state runtime 分开报告。

## 发布与法律门禁

- [ ] 每 target 的 `ckc --version --verbose` 报告 compiler 0.10.0、LLVM 22.1.8、
  ABI revision、target、backend、CPU policy 与 active ORC layer。
- [ ] `ckc licenses` 包含每个 embedded/statically linked third-party notice，并与
  repository provenance hash 一致。
- [ ] 正好产生现有六个 archive 名与六个 checksum sidecar；每 archive 只含一个完整
  native-enabled `ckc`。
- [ ] archive 解压后无需 LLVM、Clang、LLD、linker、SDK lookup、runtime download 或
  first-run setup 即可通过测试。
- [ ] release workflow 拒绝 partial set、checksum mismatch、version/tag mismatch、
  已存在 GitHub Release 或 dependency audit failure。

## Contract 冻结与移交

- [ ] Cargo metadata、lockfile、README、changelog、CLI output、规范文档、compatibility
  fixture、workflow tag logic 与 archive 全部声明 0.10.0。
- [ ] V0.9 standalone LLVM exported-shape promise 已退出，唯一 Native C ABI 成为权威，
  且未改变不受影响的 C/WASM 行为。
- [ ] 最终 branch commit 包含全部实现/evidence 变更，`git status --short` 为空，完整
  diff 已审查。
- [ ] branch/worktree 保持未 merge，供所有者审查。

本文档通过后可以报告 review candidate 完成；它不授权 merge、tag、发布 release 或
删除 worktree。
