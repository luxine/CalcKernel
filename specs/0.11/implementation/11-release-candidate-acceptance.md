# 阶段 11 验收：0.11 候选硬化

## 本地必须通过

使用 pinned LLVM/Clang 环境：

执行 Native benchmark 前先按 `11-runtime-replay-plan.md` 准备固定编译器 bundle，
并设置 `CKC_V010_RUNTIME_BUNDLE`。schema-6 checker 必须验证其身份、原件摘要和
双版本/双模式的实际交错样本；原历史性能记录不替代此项新验收。

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
17. `cargo test --release --locked --lib verifier_cache_ -- --nocapture`
18. Unix Native 的真实 bridge COFF 分支 macro-header syntax regression 两种输入均通过；
    两架构 Windows 的真实 SDK/MSVC 编译另由必跑 host job 验收，禁止以 stub 代替。
19. Windows bootstrap 显式关闭 LLVM C API DLL，实际安装后 guard 与 cache verifier
    都拒绝 bin/lib 的 LLVM DLL 注入；新配方按原完整缓存身份重建，禁止删 DLL 掩盖问题。

## I20 补充本地验收

I20 补充实现必须满足 `11-ssa-phi-pruning-plan.md`，以下尚待执行签收：

- [x] 无根标量 phi 环消除，O0、公开参数、指令与 Memory SSA 保持不变。
- [x] 双 branch arm、跨块引用、区域元数据及全部契约 predicate 根保留；不按 slot 名识别。
- [x] 缺目标/标量实参数量不符时不发生部分改写；原 verifier 的拒绝门仍然有效。
- [x] 完整 C/WASM/Native 正例与 mutation、checked 首错/print/strict-FP 门通过。
- [x] 新 SHA 的原始完整性能门通过，保留首次报告，不调整阈值或删减语料。

## 远程必须通过

上述 phi 清理本地证据见 review 的“I20 无根标量 phi 实现验证”和 `930f18d` 首次门禁；
I20 本地已通过；`ae7a130` 的两个远程性能 job 已通过 I14/I19/I20 门禁，
但包含 Windows 修复的同一最终 SHA 全矩阵仍待签收。

- feature branch 上显式 `workflow_dispatch` 的 quality、native-integration、六个
  native-host、x86-64/AArch64 performance jobs 全绿。
- 每个 native-host 上传/记录 pre-LLVM fact audit evidence，并拒绝注入 mutation。
- performance artifacts 使用 schema 6，记录 pinned 0.10 digest/compiler identity、原样
  历史 `v010MedianNs`/`v010ClangMedianNs`、冻结 C-oracle digest，以及独立 bundle/实际
  重放产物摘要、样本和八通道顺序。四组原数值阈值全部通过，原始 proof ratio 不归一化。
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

### 同进程 replay 首次完整本机门禁（`ae7a130`，2026-08-30）

- `cargo +1.90.0 bench --features native-toolchain --bench ckc_perf -- --task check
  --cpu baseline` 与 schema-6 checker 均 exit 0；warmup=3、samples=20、minimum-of-7、
  batch=20000000，四 runtime kernels 与六 optimizer cases 全部保留。
- unchecked Clang throughput geo `0.9994` / actual replay V0.10 ratio `1.0004`；checked
  `0.9975` / `1.0001`；raw proof throughput `1.0010`；optimizer suite median `1.1285`，
  全部 individual gates 通过。测量期间本任务没有其他编译/测试，不声称共享主机独占。
- 首次 report / benchmark / checker 摘要分别为
  `4f8b037c8c38066837782fb180dec5f0bac0f20dc6f854ed5f03fa0d57e0ef3b`、
  `358498e77d56c8244c5b25d0dd112c165bab767efdb8c08964c0b35be0626a9e`、
  `c418bd73b730217b4568965dd9ed8abf85fe93c905f32351ca4456c2122db4a9`。
  完整实际库随报告归档；固定 0.10 bundle 的源码/recipe/组件清单 identity 与前述
  preparation 相同。没有重跑择优；最终 SHA 完整 CI 验收仍待完成。

### 首次 schema-6 远程矩阵（`ae7a130`，未整体通过）

- run `33302144688`：quality `99232168962`、native integration `99232168888`、
  Linux ARM `99232168986` / x64 `99232169015`、Darwin ARM `99232169047` /
  x64 `99232169032`、performance ARM `99232168961` / x86-64 `99232169033`
  均 success。Linux ownership sanitizer 与实际 Darwin 签名/JIT audit 在本轮通过。
  Windows ARM `99232168996` / x64 `99232169083` 均被 I22 的不合格静态缓存挡住，
  因此该 run 的结论仍为 failure；不能将八项成功记作阶段完成。
- ARM unchecked Clang / replay ratio `0.9983 / 1.0011`，checked
  `0.9995 / 1.0010`，raw proof `1.0001`，optimizer suite `1.3587`。
  x86-64 分别为 `1.0506 / 0.9996`、`1.0021 / 1.0096`、`0.9929`、`1.5072`。
  两架构所有 individual gates 也通过；没有调整数值门槛或改成 optional。
- 完整 performance artifacts（含固定编译器、32 个实际计时库、报告及准备/检查日志）
  已保留。x86-64 artifact `9729344974` zip 摘要为
  `1f6c755c13862b5a64be4538c2041c57d4bdc7a84b81080fe4d4557a9ac087fa`；
  ARM artifact `9729336729` 为
  `4d4d8e077885adfac7838ef057b32b9cf72dbdc89427e923dbe5656d655ab9b1`。
  原始 job 日志摘要分别为
  `31a246644c1bf6338126d5a45fc52508170eb744a76cb21eed77ba72e7bd88b5`、
  `28876d04cb69f1489d3e92d33f7b7b6b74d36b35049d9ec6042f01e2086e146f`。
- I22 新配方及测试本地复验为 default 470 / all-feature 593 / release lib 53 /
  release IR 58，Clippy/fmt/diff 均通过；详细 red/green 与自审见 review 的 I22。
  下一轮必须在同一最终 SHA 重跑全部十个 required jobs，不拼接不同 SHA 的成功项。

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
