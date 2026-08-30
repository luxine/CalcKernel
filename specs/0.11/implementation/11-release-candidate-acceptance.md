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

## I25 补充验收（未签收）

I25 另按 `11-windows-static-link-plan.md` 执行，未签收：

- [ ] LLVM/LLD、bridge、Rust 的实际 Windows CRT 均静态 release；compile commands
  与真实 COFF directives 双检查，dynamic/debug/mixed/损坏输入全部拒绝。
- [ ] COFF 的 LibDriver/WindowsManifest/DTLTO 闭包完整；配置覆盖成动态 Rust CRT
  必须在 bridge 编译前拒绝；两架构实际 MSVC 链接、Native/CLI 与发布依赖审计通过。
- [ ] 新 recipe cache 验证后保存，旧不合格 Windows cache 不复用；同一新 SHA 的
  完整本地门、首次性能与全部十项 required CI 通过，不拼接不同 SHA 的结果。

## I23 补充验收（未签收）

按 `11-interrupt-handoff-plan.md` 执行：

- [x] 原生产 Unix handler 的隔离 before-arm 与重复 pending regression 先 red 后 green；
  after-arm、guard 重装对照通过；使用真实 SIGINT 与真实自有 child。
- [x] public parent 仍只收到一次 SIGINT，精确输出 245/CKR0006；超时明确失败并清理
  自身进程 group，不能无限等待、不准 sleep/retry 掩盖竞态。
- [ ] 新实现通过完整本地、首次性能门与同一最终 SHA 全十项 CI；Windows 行为不变。

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

### I25 Windows 静态链接（2026-08-30，本地通过，远程未签收）

- 计划 `c17e1bf` 与 host-only fixture 修订 `45a88da` 先提交。实际旧 verifier 接受
  动态 COFF archive 的 red 已保留；新配置回归 2 项、真实 COFF 回归 3 项均通过。
  详见 `../review/implementation-blockers-01.md` 的 I25 逐项复诊、修复和日志摘要。
- 原本地门重跑：default 475 / all-feature 606（Native 102），release lib 53 / IR 58、
  generated 3 / mutation 10 / fact audit 7 / verifier-cache 5 / docs 16，0 failed/ignored。
  两种 Clippy、fmt/diff、Native release build、真实 compiler 签名/依赖、artifact/JIT
  audit、version/licenses、Unix prefix 验证均通过，没有改变原 gate/数值/工具链版本。
- default/all 日志 SHA-256 为
  `6ef13fc1481f2988873cff85207a5289e20b0994dc79e4ce72f4e3107398f3e1` /
  `1b25ae4bb075f7b1f7c8cd2eb2417611861f2240d6e57bcf3f93eabe7fffa696`。
  当前 Unix manifest 仍为 `b8b790dcfdd9652b1634d8d50075b1037298ec7cbcf3e7a5fefabb55d1f84874`；
  新 recipe 不复用旧不合格 Windows cache。原 run 33302635528 的部分绿灯不计新 SHA。
- 当时尚未签收性能与远程 CI；后续性能复诊/复验见下一项，两架构 MSVC 与完整
  十项 CI 仍未签收。

### I25 首次性能失败与同 SHA 环境复验（2026-08-30）

- 实现提交 `d4242700489e632cd3ef2d2d9c88610b683f1fbf` 的第一次完整 schema-6
  测量成功产出原始数据，但 checker 拒绝 unchecked/integer_accumulate 的 Clang
  样本稳定性；不是通过。原报告 SHA-256 为
  `8a9d24fcc5ecf743775b523e42f0a18c5b2be5e25a75df9ac9b12773739b8506`，
  原 checker 日志为 `de6d163d3f5c8c6d14b6e4b62ed85762a232a5d71c4af56fe371768298ed1d3d`。
- 复诊确认八个 candidate Native 完整库的摘要与已通过的 `99ffb34` 全部相同，固定
  0.10 replay compiler/bundle、Clang、冻结 C source 与 benchmark/checker 均未变。
  多个独立 integer 通道同时从约 8.7/10ms 跳到 15.2/17.5ms；不是仅候选发生变化。
  失败后 CPU 观测达约 91% busy，后续还看到外部 Rust 构建占用约 15 核。以上支持
  非代码环境干扰，但不是逐样本调度/core migration 的直接证据，未改任何测量代码。
- 先记录唯一一次同 SHA qualification 的条件，再只读等待：连续三个当前 CPU 样本
  idle >=70%、未观测到高负载编译/索引任务。失败的 preflight 不启动 benchmark。
  实际启动前样本为 71.26%/71.94%/78.90%，保持全部原参数、样本、门槛与实际产物
  校验；期间使用轻量 iostat 留证，没有停止其他任务，也没有筛选或删除样本。
- 该唯一追加运行的 benchmark/checker 均 exit 0，原完整 gate 全通过：unchecked
  Clang geo=0.9985、配对 0.10 ratio=1.0022；checked 为 0.9964/1.0071；原始 proof
  throughput=1.0007；optimizer suite-median ratio=1.1289，全部 individual gate 通过。
  报告 SHA-256 为 `24f655af9613d024bbf3af235ee9722a568c597e1cfecb28aed0b060e2577a57`，
  checker 为 `dad8e28e8f42003ecfbd7036a6f01c0fa004ad0e35f750a76987a402ab808a60`。
- iostat 表明复验期间负载仍有变化（最低 19% idle），不能宣称全程无外部负载；
  判定来自原 checker 对全部原样数据的验收。首个失败报告、两轮实际产物、preflight、
  全程 telemetry 与诊断均保留。没有第三次计时，没有降低稳定性或其他数值门槛。
- 新候选 CI `33316188869` 已按上述确切 `d424270` SHA 触发，`publish=false`；
  14:17Z quality 已通过。16:00Z AArch64 performance、16:05Z Linux ARM host
  也已通过；16:15Z Darwin ARM host、16:35Z Linux x64 host 通过，当前为同 SHA
  5/10；16:44Z native integration 与 x86-64 performance 通过，当前为 7/10；
  17:17Z Darwin x64 host 通过，当前为 8/10；其余两架构 Windows 仍使用
  新完整 recipe 构建或验收。Windows
  两架构的实际 bootstrap 路径包含新 recipe 前缀 `452e16daafeb9644`。随后从实时
  日志确认 release profile 的 compile-commands guard 通过：x64 共 2478 个、ARM64
  共 2471 个 C/C++ 文件全部使用 `/MT`，分别见 [x64 实际记录](https://github.com/luxine/Rust_CalcKernel/actions/runs/33316188869/job/99269971157#step:4:295)
  与 [ARM64 实际记录](https://github.com/luxine/Rust_CalcKernel/actions/runs/33316188869/job/99269971150#step:4:293)。
  这不替代安装后 archive CRT、oracle profile、完整链接或 host suite；同 SHA 十项
  required jobs、两架构 MSVC 和总验收仍未签收。
- AArch64 performance 的原 schema-6 checker 通过：unchecked Clang / 0.10 ratio
  为 `0.9996 / 1.0016`，checked 为 `1.0001 / 0.9999`，raw proof 为 `0.9997`，
  optimizer suite-median 为 `1.3497`，全部 individual gate 通过。下载归档包含 24 个
  本次 measurement 产物与 8 个实际 replay 产物；逐文件 bytes/SHA-256 对报告复核为
  32/32 相符。完整 job log SHA-256 为
  `46b6911af3a98e90dd88ef72e77e31d1f9342af76f68d3f4a9f75c8424e9c594`。
- Linux ARM host 的 pre-LLVM fact audit 7 项、Native 101 项、CLI 22 项均通过，
  release compiler dependency、native artifact 与 JIT memory audit 通过；完整日志
  SHA-256 为 `e0ed7115fe078d549b9b07627d83dcd4d78005b622aa60986373a25faad69cc2`。
  该 job 的 release/oracle cache save 因同键被另一任务占用而警告；同一运行的
  AArch64 performance 已分别成功保存完全相同的两条 `452e16daafeb9644...` key，
  不是未验证 prefix 或旧键 fallback。两份 artifact 及日志已完整保存；全部 artifact
  checksum 清单继续随新增完成项更新。
- Darwin ARM host 的 fact audit 7 项、Native 102 项、CLI 22 项通过；实际 release
  compiler 先以 hardened runtime ad-hoc 签名，再通过严格 dependency audit；native
  artifact 和 JIT memory audit 通过，release/oracle 两个新 recipe cache key 均成功
  保存。完整 job log / fact-audit artifact SHA-256 分别为
  `b306bd4ac11f29051c6069f0a6ae74ab8c683a5bda0dc1b9aef4fb52f5706d7f` /
  `35667d9e6fe446f5a3cf38aa93bdc2d240ab2dcba075814ecfc272a1ae943549`。
  当时全部已下载 artifact checksum 清单随后随新增完成项更新。
- Linux x64 host 的 fact audit 7 项、Native 101 项、CLI 22 项及 release compiler
  dependency、native artifact、JIT memory audit 全通过；release/oracle 两个新 recipe
  cache key 均成功保存。完整 job log / fact-audit artifact SHA-256 分别为
  `4a34020d7f7e718869fe8ff989a3e3a4a69287c78dfba32b03f665717640465b` /
  `8ef444103d991eb8357e185f0f02d87c5fdafc614b212c872194d2e9f44f9714`。
  当时全部已下载 artifact checksum 清单随后随新增完成项更新。
- native integration 的完整 all-feature 测试合计 605 项通过，额外 artifact fixture
  5 项、Linux ASan+UBSan+LSan ownership 8 项通过，0 failed/ignored；release build、
  native artifact 与 JIT memory audit 通过。并发 cache warning 由已完成 Linux x64
  host 成功保存完全相同的 release/oracle keys 闭环。完整日志 SHA-256 为
  `bca12b20db3afb4237a7595aded968e006b7a5e4c38d3ca6c5da69743b5d2511`。
- x86-64 performance 的原 schema-6 checker 通过：unchecked Clang / 0.10 ratio
  `1.0516 / 0.9983`，checked `1.0184 / 1.0078`，raw proof `0.9944`，optimizer
  suite-median `1.5132`，全部 individual gate 通过。下载归档的 24 个 measurement
  与 8 个 replay 产物逐文件 bytes/SHA-256 为 32/32 相符；report / 完整 job log
  SHA-256 分别为 `4706ef0a31521c544eb997ac05be9119b736c97939957b50cf34f98610ea6c9e` /
  `1ca7eb3d936deeed46a9602b4e16c8cf91ad5467a9b9506d83aab7130986b39e`。
  当时全部已下载 artifact checksum 清单随后随新增完成项更新。
- Darwin x64 host 的 fact audit 7 项、Native 102 项、CLI 22 项及 hardened 签名、
  dependency、artifact、JIT audit 全通过。I23 的 before/after child registration、
  repeated pending、guard 重装、超时清理与 public SIGINT/245/CKR0006 回归均在 Native
  suite 内通过，不再复现旧运行卡住。release/oracle 新 recipe cache keys 均保存成功。
  完整 job log / fact-audit artifact SHA-256 分别为
  `41b992b64bb106ebb9f55d078b8e96b1a46b34899b0e8c65737455f87f86de35` /
  `de16e8d8b9d1db5c8c4718d7deb5573324b679da1d219c2efa3499fabc950660`。
  当前全部已下载 artifact checksum 清单 SHA-256 为
  `8d0e87c272f5f4cc4a5e1702b37bd8a6104e015dace7b217c8b40c08c6617a68`。
- Windows x64 job `99269971157` 随后完成 bootstrap、fact audit 与 artifact 上传，但
  Native suite 以 62 passed / 30 failed 终止，故本 run 已确定不能签收。完整日志
  SHA-256 为
  `2315bc4d21c60ea36ff12085864733a3879085102db34bdfc5086602ff89f0ba`；fact-audit
  artifact ID `9737051325`，zip / 原文件摘要分别为
  `bee37c361e63f616374a215f95a008e321d1e33756e0a10e40e2d2d4b90aab8f` /
  `27c2a74b0ed7af65bfea3706d849ac3bf01725a1e5f6ebe2ce8a8ecf289d780b`。
  复诊得到 I26 三项根因：COFF LLD 错用 Unix `-o`、x64 JITLink 缺内部
  `__ImageBase` anchor、两条 IR 测试未接受正确的 Windows `dllexport`。修订计划已写入
  `11-windows-static-link-plan.md` Task 5；在计划提交、TDD 修复、新 SHA 全十项 CI 前，
  I26/I25/阶段 11 与本文件总签收均保持未通过。Windows ARM64 继续保留自然终态证据，
  但无论其结果如何都不能覆盖该 x64 failure。

### I24 preservation 回归迁移（2026-08-30，本地通过）

- 计划提交 `c5652e0` 之后恢复原阶段 02 测试过滤器，实际 3 passed / 0 failed /
  0 ignored；全部 O0–O3 的控制流、打印顺序及 checked-bounds slice 调用/返回均覆盖。
  slice 的 data/len 经实际生成 C 调用校验，不以旧 MIR 文本或空测试代替。
- 完整 default 473 / all-feature 601（Native 99）、两种 Clippy、fmt/diff、docs 16
  项均通过；原始日志摘要与行内对照复审见 review 的 I24。
- 本轮无生产实现、benchmark、checker、阈值或 workflow 变更。保留下面 `99ffb34`
  首次性能原件；最终候选仍须通过同一 SHA 的全部十项 CI，不拼接历史成功项。

### I23 首次完整本机性能门（`99ffb34`，2026-08-30）

- 被测提交为 `99ffb34d6c58402a155bb4401c033513ca819462`，工作区干净；
  `cargo +1.90.0 bench --features native-toolchain --bench ckc_perf -- --task check --cpu baseline`
  exit 0。四 runtime kernels、六 optimizer cases、全部既定采样参数不变，未重新计时。
- 同一份首次报告在原始目录及完整归档目录均通过原 schema-6 checker，exit 0；
  unchecked Clang throughput geo / actual replay V0.10 ratio 为 `0.9995 / 1.0006`，
  checked 为 `0.9969 / 0.9990`，raw proof throughput 为 `0.9997`，optimizer suite
  median ratio 为 `1.1630`；全部 individual gates、产物/编译器摘要与采样顺序检查通过。
- report / benchmark / successful checker SHA-256 分别为
  `7feb314cb279204c6eb4bb0a67e82719762434818849a0e13eddeb56b5abc3e0`、
  `a172490cf9e38eb52bdad6986eb1197c331f887b58083472ca588b13505e8469`、
  `6934479b9ac06c2e7c2a84ab43ced593f673b4726bc55605313500ea59fa478b`。
  原始位置与归档报告逐字节相同；24 个本轮实际计时产物及固定 bundle 的 8 个产物均校验。
- 最初只复制报告、未复制相对 `measurement-*` 目录的归档校验失败；随后单独校验命令
  遗漏 `CKC_LLVM_PREFIX` 也被拒绝。两份操作失败日志原样保留。补齐产物原件及相同的
  pinned 环境后，对同一报告重做只读校验通过；没有改报告、checker、采样参数或重跑计时。
- 测量期间本任务没有其他编译/测试；共享主机的其他应用不受本任务控制，不声明独占。
  全矩阵仍待这个实现的真实平台证据，不将 `5895242` 的部分 CI 通过拼接为本提交验收。

### I22 提交首次完整本机门禁（`5895242`，2026-08-30）

- 被测提交为 `5895242cbd64b5212ecb61e24cb3ca1d43aa5502`。原完整 benchmark
  `--task check --cpu baseline` 与 schema-6 checker 均 exit 0；保持四 runtime kernels、
  六 optimizer cases、3 warm-up、20 samples、minimum-of-7 与 batch=20000000。
- unchecked Clang throughput geo / actual replay V0.10 ratio 为 `0.9991 / 1.0010`，
  checked 为 `0.9924 / 1.0077`，raw proof throughput 为 `1.0008`，optimizer suite
  median ratio 为 `1.5586`；全部 individual gates 通过。没有重跑择优，测量期间
  本任务未并行编译/测试，不声称共享主机独占。
- 原始 report / benchmark / checker SHA-256 分别为
  `03d255c655262c3a6e21550455c2dfe688b9b82c7414678b2b422d96dd492e00`、
  `601a37dee1f973c0febe6928198bdacf54256db8f359306b48537ee6b9c4ad20`、
  `146e390d8305b0c797f7e5735a091816f44de2a33eba6bd8dc37021593cb1f66`。
  实际计时库随报告归档；固定 0.10 独立 bundle 与历史 baseline 均未改变。

### I22 提交远程矩阵（`5895242`，最终取消，未整体通过）

- [run 33302635528](https://github.com/luxine/Rust_CalcKernel/actions/runs/33302635528)
  针对同一 `5895242cbd64b5212ecb61e24cb3ca1d43aa5502`，最终核实 7/10 必需项
  success：quality `99233477544`、native integration `99233477391`、Linux ARM
  `99233477579` / x64 `99233477538`、Darwin ARM `99233477589`、performance ARM
  `99233477492` / x86-64 `99233477564`。Darwin x64 `99233477608` 已完成 bootstrap，
  但 Native suite 暴露 I23 中断测试卡住。Windows x64 `99233477598` 完成 bootstrap
  后因 I25 CRT/COFF 问题失败。新候选 dispatch 按既有 concurrency 在 14:11Z 取消
  尚未完成的 Darwin x64 与 Windows ARM `99233477647`，整轮终态为 cancelled。
  ARM 在 oracle build 中取消，未执行 Native 验收；两项取消不是自然完成或成功。
  旧 Windows cache 的静态 CRT 不合格，不能复用；不能据此关闭阶段 11 或总验收。
- 已保存取消后的完整日志：Windows ARM SHA-256 为
  `a5b52581de28ae978edb42c1527ded549676777e071df2cdd3a91658db3bf4fd`，
  Darwin x64 为 `5b7e2ad5a3a0067e5620609c1e51f6db00f985ecd019c834357bc225abdf894c`。
  ARM 日志也两次报告旧 `LLVM_USE_CRT_RELEASE` 未使用，进一步证实旧配方不能
  作为合格静态 CRT 输入；没有删除或掩盖原失败/取消证据。
- Native integration 的全特性测试 592 项、artifact fixture 5 项、Linux ownership
  ASan/UBSan/LSan 8 项均通过，0 failed/ignored；Linux 分支实际启用
  `detect_leaks=1:halt_on_error=1`。fmt、all-feature Clippy、release build、native
  artifact 与 JIT audit 均通过。原始 job log SHA-256 为
  `aa15158d74717d546fa63941fcb2d8355cf779ff47c8f2b0854f57a7b074708d`。
- 三个已完成 host 均通过 pre-LLVM fact audit 7 项和 CLI 22 项；Linux 两架构各
  Native 93 项、Darwin ARM 94 项，均 0 failed/ignored。差一项来自 Darwin 专属
  Mach-O absolute-text-relocation 回归，不是跳过测试。compiler/native artifact/JIT
  audits 均通过，包括实际 Darwin compiler 的严格签名审计。
- ARM unchecked Clang / replay ratio `1.0025 / 0.9960`，checked
  `1.0002 / 0.9993`，raw proof `0.9920`，optimizer suite `1.3843`；x86-64 分别为
  `1.0505 / 1.0002`、`1.0050 / 1.0004`、`0.9946`、`1.5194`。两平台所有 individual
  gates 均通过；原语料、统计协议和数值门槛保持不变。
- ARM artifact `9730738249` zip / report SHA-256 为
  `151c1fd94973748040024c5bc45cc3c088416cd87f33ff9ba37147489e5e2733` /
  `8260d31dce9fd358171c03b9cd1d863de062a699308b1ef33f47ae3e66c229a8`；
  x86-64 artifact `9731341770` 为
  `95971f8bb0e91f38abf897869385680f4c8da71ad0b8129d4acef7e4c88b6e29` /
  `db686aed88e1180ce46c0242228666c7b84005e1230d7f0a95d17b7e2c878f24`。
  两份下载归档另行核对固定编译器、32 个实际库的字节数/摘要，以及全部采样顺序和
  median；这只是传输完整性检查，实际平台验收来自上述原生 CI，不冒充本机重测。
- 原 `ae7a130` 的失败及部分成功继续保留在下文，不能与本轮结果拼接。全部十项完成
  后仍须逐阶段总审、提交最终证据，并确认最终交付 SHA 的完整 CI。

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
