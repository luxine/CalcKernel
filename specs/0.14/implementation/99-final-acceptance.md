# CK 0.14 总验收

> **状态：有效，尚未签署。** 本文件是 0.14 候选的唯一总验收清单；阶段 01–19
> 的 acceptance 是可追踪子集，不得独立代签最终候选。

所有项目必须由同一最终 candidate SHA 的真实结果支持；
阶段日志、旧 CI、partial report、降低阈值或 skipped required capability 不能代签。

## A. 分支、基线与版本

- [ ] 当前分支为 `design/v0.14-offline-autotuning`，独立 worktree 正确且 clean；`main` 未自动合并。
- [ ] 最终 accepted v0.13 revision `d5a2491672477634070b0c36b77cb8ad4bf7df56` 已完成逐差异审计与等价集成。
- [ ] Cargo/CLI/docs 为 0.14.0；CKCOBJ04/cache 5、tune schemas 1、KIR 3、bridge 4、Native ABI 1、Runtime ABI 2 一致。
- [ ] 未创建/移动 tag 或 GitHub Release；所有本地/远程证据绑定同一最终 SHA。

## B. 默认行为、安全语义与 ABI

- [ ] 调优显式 opt-in；普通 check/run/build/emit/release 无 harness、tune cache/decision I/O 或 optimizer 变化。
- [ ] language/source/strict-f64/checked first-error/effect/print/slice/C/WASM/public Native ABI/Runtime ABI 无回归。
- [ ] measurement 只授权收益，未建立 range/alias/alignment/effect/bounds proof，未移除 guard 或改变 target feature。
- [ ] predicated same-place update 只在 exact place、Memory SSA、unit stride、strict compare、无冲突副作用以及
  checked first-failure 全部可证时物化；否则保持原 scalar CFG。
- [ ] final executable/dynamic library self-contained，不含 runner/tune symbol/dispatch runtime 或新 shared dependency。

## C. CKTUNE01、Manifest 与 inspection

- [ ] decision exact framing/tag/type/order/bounds/digest/EOF 与五个 fixture 通过 encode/decode/re-encode/mutation/cross-endian。
- [ ] self-contained checker 重算全部 policy/search/measurement/selection/replay/cache 等式；source-aware checker 重算 KIR/artifact 等式。
- [ ] closed TOML、argv/env/cases/inputs、relative runner base、no-follow snapshots、CKTIMAP1 fresh staging 全部通过正负测试。
- [ ] JSON/text 完整 exact tree、stable ordering，无 absolute path/secret/timestamp/PID/localization。

## D. 候选、trial 与重放

- [ ] 七类 CK-owned alternatives、stable site/unit/variant/anchor/payload、bounds 与 canonical identity 完整。
- [ ] zero-based complete expansion、whole-plan rank、beam/diversity/compile selection/actual-size finalist 可独立重算。
- [ ] trial nonpublishable typestate，legal plan 每次从同一 pre-state 重放并由独立 checker 验 proof/effect/guard/growth。
- [ ] trials exact 等于 compile set，isolated rebuild 的 plan/object graph/link recipe/content/bytes 全部一致。

## E. Runner、测量与选择

- [ ] no-shell empty-env runner、Unix group/Windows Job、UCRT argv、fresh cwd/input、output bounds、termination/reap/empty 完整。
- [ ] calibration/confirmation/overshoot、完整 timeout+2250 ms admission、correctness digest 与 timeout typestate 精确。
- [ ] smoke、3 warmup、20×3 search、两轮 validation、rotation/skip/stream set/upper median/stability/Q32/paired wins 完整。
- [ ] selection 四行表与 outcome matrix 一致；tuned 必有 exact certificate，完整无收益产生 baseline reason，partial evidence 无输出。

## F. Publication、cache 与 CLI

- [ ] canonical destination/Windows alias、persistent locks、overlap closure、CKTJNL01、atomic update 与 barriers 完整。
- [ ] 每个 crash point 恢复完整 old/new set，primary-last；impossible state 保留证据 fail-closed，recovery 幂等。
- [ ] compile/measurement/completed cache 身份、salt、private path、checksum、atomic entry、LRU、4 GiB 与 no-splice 完整。
- [ ] tune build/inspect/tune-use option matrix、early failure、cold determinism、warm zero-work reuse、no-cache 与 stale replay negative 完整。

## G. 兼容、质量与平台根因修复

- [ ] CKCOBJ03/schema4 clean miss；v0.13 profile mismatch 和 future tune schema fail-closed，不就地升级。
- [ ] 双语 docs、CLI help、diagnostics、architecture、optimizer、performance、compatibility、release 文档一致。
- [ ] fmt、clippy `-D warnings`、rustdoc `-D warnings`、feature-disabled/all-features tests 全绿。
- [ ] Profile Runtime 在 MSVC 使用 Interlocked、在 Linux AArch64 使用内联 LL/SC、在其余 Unix
  使用 compile-time always-lock-free C11 原子；Runtime ABI 2、CKPROF01、CKPART01 和公开状态码未变。
- [ ] Linux 与 Darwin profile shard 完成 create-new/write/file-sync/no-replace/directory-sync/no-follow reopen；
  不依赖错误的 raw-stat 布局、`___error` 或未冻结 `_fstat$INODE64` 符号。
- [ ] dynamic primary/header/import-library 由 `NativeArtifactPaths` 决定；LLVM void call 不获得 SSA 名称，
  non-void call 名称与 Bridge ABI 4 保持不变。
- [ ] 六 Native host 的 ABI、runner、filesystem、journal、cache、artifact、profile publish、void call、真实
  executable/dynamic 和 ordinary-isolation tests 无 required skip。

## H. Schema 9 性能与证据

- [ ] exact 七 case、三 partitions、CK/C/Rust oracle、recipe/evidence identities 与 historical/fresh schema8 closure 完整。
- [ ] held-out 相对 faster v0.13 baseline geo >=5%，selected each >=2%，validation/held-out each slowdown <=2%。
- [ ] 相对 faster hand SIMD geo >=98%、each >=92%；两个 domain case 相对 generic C/Rust geo >8%。
- [ ] artifact <=110%；tune-use compile <=10% geo/20% each；ordinary <=3%/8%；archive <=110%。
- [ ] standard <=30 min/bounds、RSS <=2x ordinary、cache <=4 GiB；wait4 receipts、cold/warm determinism 和 final dependency audit 完整。

## I. Predicated-Update Performance Contract 1

- [ ] 冻结 source、training/validation/release TSV、manifest 的 exact bytes/path/digest 与 Contract 1 一致；
  SplitMix64 golden cells 和三 split 结果 digest 可独立重算。
- [ ] runner 的 tune/profile/oracle/perf 四协议具有 exact argv/env/receipt；profile 唯一 flush、timed call fresh
  matrix、timer 内仅 native kernel，所有负例 fail-closed。
- [ ] decision 恰有一个 selected `PlanChoice`、一个 Loop SIMD `UnitVariant`、一个目标 `SiteAlternative`；
  minimum <=128，N/slice 事实证明所有 guard 为真且 N=128/256/1024 都实际执行至少一个向量 chunk。
- [ ] source-aware attestation 精确绑定 source/KIR/site/candidate/unit/variant/VF/UF/minimum/pre/post digest；
  replay byte-equal，复合 plan、不可达 rewrite、错误 profile/target/post-state 均被拒绝。
- [ ] report 的 profile `DirectoryEvidence`、四个 XDG cache base 到实际 `cache/<command>/ckc` namespace、
  pgoTuned-only publication locks、七命令与完整 regular-file inventory 闭合。
- [ ] validation 与 release 分别保留 3 warmup、20 measured、每 row/channel 3 calls/min 的全部原始回执；
  独立 checker 重算 upper median、16-of-20、validation `tuned*100 <= pgoOnly*102` 与 release
  `tuned*100 <= pgoOnly*95`，collector 不含接受逻辑。
- [ ] Linux x86-64-v4 与 AArch64 SVE2 两个 stable host 各自真实通过 Contract 1；任一 host、样本、证据或
  schema 9 外键缺失均不能签署。

## J. exact-SHA CI 与交付

- [ ] quality、native-integration、六 Native host、两 stable performance 共十 job 对同一最终 SHA 全部成功。
- [ ] 阶段 01–19 的本地验收已按依赖顺序重放；所有 selector 非零，未以旧阶段记录代签。
- [ ] schema9 与 Contract 1 report 的 candidate SHA、compiler bytes、decisions/outputs/receipts/archive 与 CI checkout SHA 相等。
- [ ] generated decisions/reports/cache/temp/run ids 未进入 source commit；worktree 最终 clean。
- [ ] 最终实现已提交在独立 worktree，未合并 main，等待用户审查。

## 最终执行记录

动态 SHA、命令输出、test count、toolchain/host identity、CI run/job 与 artifact digest 只写
`target/acceptance/v0.14/final/` 和 CI artifact，不能回写本文件制造自引用提交。
