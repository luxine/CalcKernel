# CK 0.14 总验收

> **状态：暂停且不可签署。** 本清单尚未吸收当前 predicated-update 优化兑现、独立性能
> Contract 1 与跨平台修复门槛；必须在本轮任务第 5 步整体重建后才恢复验收权威。

重建完成后，本文件是 0.14 候选的唯一总验收清单。所有项目必须由同一最终 candidate SHA 的真实结果支持；
阶段日志、旧 CI、partial report、降低阈值或 skipped required capability 不能代签。

## A. 分支、基线与版本

- [ ] 当前分支为 `design/v0.14-offline-autotuning`，独立 worktree 正确且 clean；`main` 未自动合并。
- [ ] 最终 accepted v0.13 revision 已明确集成或完成逐差异审计；v0.13 remote gates 未完成时 release 保持 blocked。
- [ ] Cargo/CLI/docs 为 0.14.0；CKCOBJ04/cache 5、tune schemas 1、KIR 3、bridge 4、Native ABI 1、Runtime ABI 2 一致。
- [ ] 未创建/移动 tag 或 GitHub Release；所有本地/远程证据绑定同一最终 SHA。

## B. 默认行为、安全语义与 ABI

- [ ] 调优显式 opt-in；普通 check/run/build/emit/release 无 harness、tune cache/decision I/O 或 optimizer 变化。
- [ ] language/source/strict-f64/checked first-error/effect/print/slice/C/WASM/public Native ABI/Runtime ABI 无回归。
- [ ] measurement 只授权收益，未建立 range/alias/alignment/effect/bounds proof，未移除 guard 或改变 target feature。
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

## G. 兼容、质量与平台

- [ ] CKCOBJ03/schema4 clean miss；v0.13 profile mismatch 和 future tune schema fail-closed，不就地升级。
- [ ] 双语 docs、CLI help、diagnostics、architecture、optimizer、performance、compatibility、release 文档一致。
- [ ] fmt、clippy `-D warnings`、rustdoc `-D warnings`、feature-disabled/all-features tests 全绿。
- [ ] 六 Native host 的 ABI、runner、filesystem、journal、cache、artifact 和 ordinary-isolation tests 无 required skip。

## H. Schema 9 性能与证据

- [ ] exact 七 case、三 partitions、CK/C/Rust oracle、recipe/evidence identities 与 historical/fresh schema8 closure 完整。
- [ ] held-out 相对 faster v0.13 baseline geo >=5%，selected each >=2%，validation/held-out each slowdown <=2%。
- [ ] 相对 faster hand SIMD geo >=98%、each >=92%；两个 domain case 相对 generic C/Rust geo >8%。
- [ ] artifact <=110%；tune-use compile <=10% geo/20% each；ordinary <=3%/8%；archive <=110%。
- [ ] standard <=30 min/bounds、RSS <=2x ordinary、cache <=4 GiB；wait4 receipts、cold/warm determinism 和 final dependency audit 完整。

## I. exact-SHA CI 与交付

- [ ] quality、native-integration、六 Native host、两 stable performance 共十 job 对同一最终 SHA 全部成功。
- [ ] schema9 report candidate SHA、compiler bytes、decisions/outputs/receipts/archive 与 CI checkout SHA 相等。
- [ ] generated decisions/reports/cache/temp/run ids 未进入 source commit；worktree 最终 clean。
- [ ] 最终实现已提交在独立 worktree，未合并 main，等待用户审查。

## 最终执行记录

动态 SHA、命令输出、test count、toolchain/host identity、CI run/job 与 artifact digest 只写
`target/acceptance/v0.14/final/` 和 CI artifact，不能回写本文件制造自引用提交。
