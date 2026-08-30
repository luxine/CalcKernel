# 阶段 11 验收：0.11 候选硬化

## 本地必须通过

使用 pinned LLVM/Clang 环境：

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features --locked -- -D warnings`
3. `cargo test --locked`
4. `cargo test --all-features --locked`
5. `cargo build --release --features native-toolchain --locked`
6. Linux：`scripts/test-sanitized-ownership.sh`，执行 ASan+UBSan+LSan；Apple 本地只记录 capability unavailable，不替代 Linux CI 的必跑门（当前 Apple Clang 17 runtime 在 macOS 26.6.2 上连最小 C ASan 程序也无法完成初始化）。
7. `cargo test --locked --test optimizer generated_ -- --nocapture`
8. `cargo test --locked --test ir mutation_ -- --nocapture`
9. `cargo test --all-features --locked --test native fact_audit_ -- --nocapture`
10. `cargo bench --features native-toolchain --bench ckc_perf -- --case proof --task check --cpu baseline`
11. `python3 scripts/check-native-performance.py target/ckc-perf/results.json`
12. `scripts/audit-native-artifact.sh target/native-acceptance`
13. `scripts/audit-jit-memory.sh target/release/ckc`
14. `./target/release/ckc --version --verbose`
15. `./target/release/ckc licenses`
16. `git diff --check`

## 远程必须通过

- feature branch 上显式 `workflow_dispatch` 的 quality、native-integration、六个
  native-host、x86-64/AArch64 performance jobs 全绿。
- 每个 native-host 上传/记录 pre-LLVM fact audit evidence，并拒绝注入 mutation。
- performance artifacts 使用 schema 5，记录 pinned 0.10 digest/compiler identity、每项
  `v010MedianNs`/`v010ClangMedianNs` 与冻结 C-oracle digest；配对归一化后的四组阈值全通过。
- 不允许 skipped/neutralized required job；重跑必须保留失败日志并说明非代码 flake 证据。
- Windows job 的 bootstrap/compiler/archive identity 必须为 MSVC/`.lib`。Darwin JIT audit
  必须接受且只接受与 runtime capability 一致的安全 tuple：`map-jit=yes / thread-wx-supported=yes / thread-wx=yes`，
  或 `map-jit=no / thread-wx-supported=no / thread-wx=no`；两者共同满足
  relocation=RW/NX、code=RX、data=NX，不得出现 RWX。
- Windows checkout 必须保留 provenance 原始字节；真实 `core.autocrlf=true` Git filter
  regression 与未修改 expected checksum 的 provenance validation 均通过。
- Cache key 必须随任一 runtime 编译/链接输入变化，且成功 bootstrap 即保存验证过的
  release/oracle prefix。Darwin 必须通过 Small+PIC 下的 O0 internal-call text relocation
  检查，以及 dyld C-ABI `LC_MAIN` entry 的 standalone、sanitizer 与 O0–O3 differential
  全部 tests；不得通过 writable code pages 修复 loader failure。
- Cache-boundary verifier 的真实 Unix/COFF fixture 正例通过，runtime/import hash corruption、
  traversal、重复/错误大小写字段与 false static flag 均失败；release cache save 先于
  oracle build，两个 profile 各自在验证成功后保存。
- Linux artifact audit 必须同时拒绝 loader-visible dependency/undefined executable symbol/
  unexpected export，并只在 `.comment` 为 non-`ALLOC` 且包含 pinned LLD 22.1.8 marker 时接受
  provenance。Darwin entitlement 必须与仓库唯一 `allow-jit=true` policy canonical 等值。
- Darwin 实际 compiler 在严格签名审计前显式 ad-hoc hardened 签名；未签名副本必须被
  原审计拒绝，签名后的实际文件通过 compiler/JIT audit，release workflow 打包相同文件。
- `CALCKERNEL_TS_ROOT` 在 CI workflow 只属于实际 checkout/build oracle 的 quality job；Native
  jobs 的 CLI suite 在无该变量时完整通过，不能指向不存在的目录。

## 仓库判定

- `Cargo.toml`/lock/version tests 为 0.11.0；无 tag/Release。
- current docs English/zh-CN 同路径同契约；0.10 migration 作为兼容段保留而非当前标题。
- backend/CLI 无正式 optimized-MIR path；`emit-mir` bytes 仍兼容。
- `git status --short` 只包含预期提交前变更；无 target/build/Ai_repository/LLVM prefix。

## 完成证据

追加本地命令结果、远程 workflow run URL/commit、六 host job IDs、两架构 performance 摘
要与最终阶段 SHA。

首轮候选 CI 的真实阻断、原始 job 与不降门槛的修订边界见
`../review/implementation-blockers-01.md`；必须在修复后的完整 matrix 全绿后补写复审结论。

### 本机固定 V0.10 optimizer 基线补采（2026-08-30，尚非阶段通过）

- 固定 `df816502876fba41676f9ebc190e4fadd18cd5a5`，仅使用摘要固定的 benchmark
  adapters，Rust `1.90.0`、LLVM/Clang `22.1.8`、AArch64 macOS baseline CPU；执行
  `cargo +1.90.0 bench --features native-toolchain --bench ckc_perf -- --task check --cpu baseline`
  返回 `0`。测量期间本机先前占用多核的其他项目任务已退出，未修改其他项目或进程。
- 原 macOS 六个 optimizer 字段尚为 frontend-inclusive 计时，现改为与两 Linux runner
  相同的 fixed MIR-pass timer；冻结在运行当前候选的本机 performance gate 之前，不用
  候选结果反向选择 baseline。八项已正确采集的 Native/Clang runtime baseline 保持不变。
- MIR-only upper median ns：pricing `83334`、pricing-soa `68583`、f64-kernels
  `162375`、proof `40375`、example-pricing `66709`、example-dijkstra `350000`。
- 原始 `v0-10-mir-optimizer.tsv` SHA-256：
  `676755f73d5bb698caa09f8f8af314db1a347909999d9d5fa79d38a5b33c3ca3`；同轮
  runtime report SHA-256：`fb10e7f360b98f09639c2a400cc3a5d5b909c29c7d07e0bf7efca3bff81d307f`。
  原始文件仅保留在 baseline worktree 的 ignored `target/ckc-perf/`。

### 本机 performance gate（`d266f68`，2026-08-30）

- 使用 runtime hash 自洽的临时 LLVM prefix 与 pinned Clang，在上述基线冻结后执行本文件
  第 10、11 项，二者均返回 `0`；Rust 显式使用 `cargo +1.90.0`。
- unchecked Native/Clang throughput geo `1.0005`，对 V0.10 的配对归一化耗时 geo
  `1.0003`；checked 分别 `1.0074`、`0.9911`；每组均 4 cases，individual gate 全通过。
- proof-loop checked/unchecked throughput `0.9973`；6 个 optimizer cases 的
  V0.10 suite-median ratio `0.7908`，individual gate 全通过。
- 原始 schema-5 report SHA-256：
  `bcbf384f4dc765e33c2984a5d07271246af428770166c0078264fd058894bc97`；summary JSON
  `278fa39cf9d607169a4e43e983e59ee77a2d45c410d179b9a6c05664e7769383`，summary Markdown
  `a313b0ad0c0494fab44d2b193a678228f11fef75ac82129f1d4b439a7ce8ca3f`。
- 固定 V0.10 worktree 的三份 adapter 修改及两份临时 fixture 已恢复，working tree clean；
  adapter 原件仍在本分支 `benches/baselines/*.patch`，原始报告未删除。
