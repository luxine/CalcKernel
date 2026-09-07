# CK 0.13 总验收

本文件是 0.13 候选的唯一总验收清单。阶段 acceptance 只证明局部完成，不能替代本清单。
所有项必须由同一最终 candidate SHA 的真实结果支持；不允许 ignored test、optional required job、
旧日志、历史数值、lowered threshold 或 partial bundle。

## A. 仓库与版本身份

- [ ] 当前分支为 `design/v0.13-pgo-multiversion`，worktree 是独立
  `.worktrees/v0.13-pgo-multiversion-design`，工作区干净。
- [ ] `main` 未自动合并；未创建/移动 tag，未创建 GitHub Release。
- [ ] Cargo/lock/CLI/docs 为 0.13.0；LLVM 22.1.8、bridge ABI 4、KIR 3、CKCOBJ03/key+manifest 4、
  Native ABI 1、Runtime ABI 2 与全部 private schema/runtime identity 一致。
- [ ] exact 0.12 replay 是 `af9aa37d262d9b447f407f07aa73e33ed63b4926`；最终 CI/report/证据
  全部绑定同一 candidate SHA。

## B. 默认行为、语义与 ABI

- [ ] PGO 默认关闭；普通 `check/run/build/test/release` 无训练、counter、profile I/O 或 dispatch。
- [ ] 0.12 language/source/diagnostic/semantic MIR/strict-f64/checked-first-error/effect/print/slice/public
  Native ABI/Runtime ABI/六平台 artifact contract 全部无回归。
- [ ] profile 只影响 profitability，从未建立 range/alias/alignment/effect/bounds safety；zero observation
  不删除路径，guarded specialization 保留 generic fallback。
- [ ] sanitizer 与 generate/use/multiversion 显式不兼容；C/Wasm/default inspection/JIT/public API 无
  隐式 PGO/multiversion 行为。

## C. Profile 格式、身份与生命周期

- [ ] CKPART01/CKPROF01 canonical big-endian/tag/order/UTF-8/length/digest/resource limits 通过 golden/
  mutation/fuzz-style tests；full identity hex 是 canonical identity bytes 的完整 lowercase SHA-256。
- [ ] merge 只接受 raw completed shards，拒绝 final/nested/symlink/duplicate/collision/mismatch；同 shard
  集顺序/路径不同 final bytes 相同，saturation/equation/unknown 规则正确。
- [ ] generate site/event exact；lock-free atomic64/overflow、directory no-follow+identity anchor、temp+
  fsync+validate+no-replace rename和 multi-process publication完整。
- [ ] executable normal exit/automatic child failure事务正确；library 64-hex flush exactly-one/sticky/
  quiescence，unload/`DllMain` 无 I/O；ordinary/use artifact 不含 generation runtime/path/flush。

## D. Profile analysis 与 KIR 3

- [ ] canonical site table覆盖 entry/selected edge/loop/length/constant；critical-edge/site ID/full descriptor/
  table digest稳定，profile op 是独立 effect且不 alias CK memory或被删/克隆/移动。
- [ ] complete identity/site mapping 首 mismatch 诊断、saturation propagation、edge equation、confidence/
  hotness/work 与 stable ranking边界正确。
- [ ] histogram 对 bucket 全值的 signed lower bound、fallback/guard、checked signed-magnitude `u128`
  与 independent checker完整；overflow/ambiguity/tie/unknown baseline。
- [ ] CFG change 只有独立验证的 closed mapping record 才转移 counts；profile annotation未进入 proof arena。

## E. O2 与 O3 优化权限

- [ ] O2 profile-on/off 在 `CkLateProfileLayout` 前 snapshot byte-identical且无 LLVM profile metadata；
  accepted delta仅 ordering/required terminator repair/relaxation/fixup/padding/emission。
- [ ] target repair allowlist闭合；未列 CFI/unwind/LOH/security/bundle repair回退 ordinary，AArch64
  accepted reorder重跑required branch relaxation。
- [ ] O3 PGO inlining/value/length/specialization/unroll/SLP/Loop SIMD从同一 immutable pre-state建议，
  独立 checker重算 proof/cost/mapping/growth/shared budget，transaction rollback/audit non-refund正确。
- [ ] fixed-seed differential/mutation证明 profile不能移除检查、扩大 footprint、改变 first error/effect/
  print/strict-f64、启用 fast math或伪造 mapping/metadata。

## F. Multiversion、dispatch 与 artifact/cache

- [ ] target set schema 1精确覆盖 x86 baseline/v3/v4、Linux AArch64 baseline/SVE/SVE2、Darwin/
  Windows AArch64 baseline-only；完整硬件+OS state predicate与 canonical digest。
- [ ] eligible root benefit >=10%且>=2 units，每 root最多两个 enhanced，全部从同一 baseline pre-state；
  additional unique normalized KIR-body <= baseline units，仅 profile/tier hidden name 不同且规范化后逐字节
  一致的 member 共享 body charge，真实结构差异完整计费；独立 target module/object/audit 与 artifact gate
  不共享，预算/order/checker/proof/feature audit正确且无 cross-variant LTO。
- [ ] baseline 与每个 enhanced module 的独立 KIR 复验携带同一 verified contract facts；缺失 facts
  fail closed，LLVM parameter/alias/effect strengthening仍由 Native fact audit逐项约束。
- [ ] detector fail-closed且 baseline-safe；slot 初始指向 cold ABI-preserving resolver entry，稳态
  thunk 不含 null branch；concurrent first calls 可重复 detection，但 per-root
  acquire/release slot 是唯一 publication/cache layer且只发布一个 compatible verified pointer；public
  thunk address/ABI稳定，后续 atomic load+indirect tail call；variant/runtime symbols hidden/namespaced，
  production无强制 unsupported feature seam。
- [ ] resolver entry 与 steady dispatcher 的 fact ledger 各自只复制其实际继承的参数/函数属性；
  assume/range/no-wrap/alias-scope 等函数体证据不得重复归属任一 call layer，增强 tier 平台的
  pre-LLVM fact audit 必须精确相等。
- [ ] executable/dynamic/static named-object bundle真实链接；multiversion object拒绝，single-version use
  object支持；final artifact自包含且无 profile/LLVM/compiler/new shared dependency。
- [ ] CKCOBJ03/key+manifest 4覆盖全部 profile/physical kind/target/variant/dispatch/runtime/budget identity；
  任一 missing/extra/reordered/redirected/mismatched object使全 bundle miss，generate不cache。
- [ ] final output transaction与不同 cwd/order/cache hit/miss reproducibility通过。

## G. 性能、尺寸与编译耗时

- [ ] schema 8 在 x86-64/AArch64各有完整 training/held-out/adversarial、0.12 replay、Clang/Rust PGO
  oracle、profile/target/variant/capability/sample/digest report；累计 schema 7 JSON 及其引用的
  `measurement-*` 目录在 schema 8 evidence root 内自包含且通过 redirect/symlink 审计。
- [ ] Linux schema 7 每个三通道 case 从 conditioning 到计时固定在 inherited affinity 允许的一颗 CPU，
  case 结束恢复；runtime sample 使用当前线程 CPU time 排除共享宿主 descheduling；32 批 conditioning、
  timed work、样本数、统计量、阈值、语料与平台矩阵均未改变。
- [ ] ordinary no-PGO相对0.12 replay geo slowdown <=2%、individual <=5%，并保留全部0.12累计门槛。
- [ ] PGO use相对同policy ordinary geo improvement >=5%，held-out individual slowdown <=3%；generate
  execution <=5x ordinary。
- [ ] Generation initialization guard 保留 Native `NoInline`；hot instrumented path 不包含重复展开的
  runtime initialization 参数准备，且未减少 instrumentation site/counter 或 timed work。
- [ ] Candidate-constant hit/miss 使用 saturated function-local batching，function exit 发布的 bucket
  count 精确，hot comparison 无逐 observation atomic runtime call。
- [ ] dispatch相对portable baseline eligible suite geo improvement >=8%、individual slowdown <=3%；
  相对selected-direct geo >=98%、individual slowdown <=5%，resolver once。
- [ ] AArch64 generic SVE/SVE2 member 显式携带既有 `target-cpu=generic`、精确 feature string 与固定
  `tune-cpu=neoverse-n2`，真实 object 使用预期机器调度；compatibility、feature audit 与允许 ISA 不变，
  cache codegen contract 明确包含修订后的 tuning identity。
- [ ] x86 Native profile 在既有封闭 `UF <= 4` frontier 内至少暴露四路 interleave；strict-f64 与
  integer-cast 选择 checked `VF2` 多独立链计划，`UF2/UF4` 由真实 target cost 决定，且继续通过
  不变的性能、legality、profitability、proof 与 budget gate。
- [ ] 无 profile multiversion 使用 8-instruction pure-helper inline budget，普通 O3/PGO-hot 仍为
  32/48；仍被调用的 9..32 instruction pure helper 在 Native member 中保持 `noinline`，小型
  hot-path helper 仍 inline，object cache identity 编码 `compact-multiversion-inline-v2`。
- [ ] x86 三流 noalias integer map 的 checked `VF4/UF4` 计划以共享 stride recurrence 与
  dominating MemorySSA reuse 在既有 aggregate `2x` KIR growth 上限内物化；独立 checker 验证
  全部 chunk start/backedge，object 主循环保留四条 128-bit 独立链。
- [ ] multiversion enhanced retained-set 有通过不变 profitability floor 的 trial 作为 eligibility
  witness；其 predicted non-regressing strict feature subset 可作为 compatibility companion。
  full-root budget 只容纳一个 variant 时保留兼容覆盖最广的 candidate，witness 无需同时物化；
  selected-direct 从独立加载的同字节 artifact 调用
  resolver 实际发布的 hidden member，而非 `--cpu native` 近似物。
- [ ] Linux AArch64 executable 从 startup stack 捕获 auxv；dynamic library 无 CK entry capture 时
  freestanding direct-syscall 读取 binary `/proc/self/auxv`。失败/不完整仍 baseline，且产物无新增
  libc/loader/allocator/compiler dependency。
- [ ] x86 constant-call scalar memory-map handoff按 IR semantics 使用固定 1×5 schedule；不得按 fixture
  名称特判，checked/reduction/pre-vectorized loop 与 cache identity保持各自闭环。
- [ ] combined相对faster PGO-only/multiversion-only geo slowdown <=2%、individual <=5%；相对等价
  Clang/Rust PGO oracle geo >=95%、accepted kernel individual >=90%。
- [ ] PGO/multi/combined compile geo <=1.5/2.5/3.5、individual <=2/3/4；artifact aggregate <=1.25/
  2/2、individual <=1.5/2.5/2.5；distributed archive <= exact 0.12 +15%。
- [ ] dynamic library final link 在 Mach-O/COFF/ELF 上回收未引用 private function/data section；
  user exports、实际 dispatch detector/member 与 referenced helper 保留，未使用的 startup capture/
  generic selector 不进入 multiversion shared artifact。
- [ ] ELF dynamic product 使用 LLD `--strip-all`，只保留 loader 所需 dynamic export；每个
  benchmark root 的唯一 pointer-width/aligned resolver slot 位于 private `.ck_dispatch_slot`
  `NOBITS` section，selected-direct 从同字节 artifact 的 `.dynsym` 与该 section 解析地址。
  Generated acquire/release slot 是唯一 publication layer；one-shot detector 不再维护第二份
  process cache，并以 size-first recipe 编译。

## H. 本地质量与审计

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --locked`
- [ ] `cargo test --all-features --locked`
- [ ] `cargo build --release --features native-toolchain --locked`
- [ ] sanitized ownership、release、native artifact、JIT memory audits全部通过。
- [ ] `git diff --check`；无 ignored tests、临时 profile/benchmark/LLVM prefix/CI artifact被提交。

## I. exact-SHA 十作业 CI

- [ ] quality、native integration通过。
- [ ] Darwin ARM64/x64、Linux ARM64/x64、Windows ARM64/x64六 host通过。
- [ ] x86-64与AArch64 performance通过，required enhanced tier/capability存在。
- [ ] workflow head SHA等于最终 candidate SHA；required job无 skip/cancel/continue-on-error替代。

## J. 文档与交付

- [ ] README/current docs/changelog/CLI help/release policy英中镜像与实现、限制、security/privacy一致。
- [ ] indirect calls、scalable KIR、adaptive JIT PGO、Auto-Tuning仍明确不属于0.13。
- [ ] 每阶段ignored evidence包含对应 SHA、RED、命令/count/toolchain；真实设计修订有复诊且未降门槛。
- [ ] 最终提交后工作树干净，只等待用户审查，不自动合并主分支。

## 最终执行记录

完成时把 candidate/parent/main/replay SHA、Rust/LLVM/Clang/host identities、default/all-feature/release
counts、schema 8 reports/checker/profile/variant/artifact digests、CI run URL/id/jobs、git status/worktree
状态写入 `target/acceptance/v0.13/final/`、CI artifact 与最终用户交付，不回写本文件。任何新提交
都会使 exact-SHA 证据失效，必须重跑受影响门禁。
