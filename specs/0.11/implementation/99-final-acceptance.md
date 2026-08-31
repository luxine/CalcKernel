# CK 0.11 事实驱动优化器总验收

## 使用方式

只有 01–11 每份阶段验收都记录通过证据后，才能执行本清单。任何一项失败都返回对
应阶段修复；不得在本清单修改阈值、排除 case 或增加 ignore。

## A. 设计与范围

- [x] 双语规范语义一致，第一轮 B01–B08 均有修订，第二轮 0 blocker。
- [x] 0.12+ SIMD/unroll/specialization/PGO/Auto-Tuning/fast-math 未进入实现。
- [x] main、safe/unsafe boundary、immediate-UB 与 sanitizer 调试语义符合规范。

## B. 编译器闭环

- [x] consumer roots/capability check 先于 mode-specific KIR。
- [x] KIR scalar SSA、region Memory SSA、ordered guard/effect 全部通过 structural mutation。
- [x] Fact origin/scope、closed certificate 与 independent checker 通过错误 producer mutation。
- [x] scalar/alias/effect/loop analysis 超预算保守且确定。
- [x] O0/O1/O2/O3 exact pipeline 与 verifier-after-every-pass 成立。
- [x] Release 验证缓存独立核对完整状态；错误的 no-change 声明不能放过 IR、proof、
      guard rewrite 或 contract fact mutation。
- [x] unsafe inline/recursive contract instances 不逃逸。
- [x] C/WASM/Native 全部消费 verified KIR；无正式双优化路径。

## C. Backend、安全与 ABI

- [x] C/WASM/Native O0–O3 supported-mode differential 全绿。
- [x] checked first-error、print count/order、strict f64、break/continue、void/slice 保持。
- [x] pairwise noalias 不放大成 parameter-wide noalias/restrict。
- [x] CK LLVM fact audit 位于 LLVM optimize 前并拒绝所有注入 CK-owned strengthening。
- [x] canonical proof loops 的 KIR/backend hot loop 无冗余 bounds guard。
- [x] normal build 无 contract checks；sanitizer 精确 CKR0007/LF/246 且极值无 host UB。
- [x] Native ABI=1、Runtime ABI=2、private Bridge ABI=2，六 release artifact 仍零工具链依赖。
- [x] ELF producer metadata non-`ALLOC` 且绑定 pinned LLD；Darwin hardened entitlement 与唯一
      `allow-jit=true` policy canonical 等值，不以人类展示格式替代语义比较。
- [x] Darwin 实际 compiler 在严格签名审计前显式 ad-hoc hardened 签名，打包原件与
      被审计文件一致；签名验证未放宽。
- [x] Runtime 输入参与 bootstrap cache identity、验证后立即保存；Darwin Small+PIC 不产生
      internal-call absolute text fixup，dyld C-ABI `LC_MAIN` entry 通过完整执行验收。

## D. CLI、兼容与文档

- [x] emit-kir/facts/effects/explain byte-deterministic 且无环境泄漏。
- [x] sanitizer flag 仅 run/executable，cache 与 normal artifact 隔离。
- [x] unsafe header normalized comments 可映射 flattened slice fields，ABI shape 不变。
- [x] 所有 v0_10 与 v0_11 compatibility manifest evidence 可执行。
- [x] current English/zh-CN docs 同步为 0.11.0；version/help/licenses/release policy 一致。

## E. 质量、性能与分发

- [x] fmt、all-feature clippy、default/all-feature tests、sanitized ownership 全绿，无 ignore。
- [x] fixed-seed generated differential 与全部 KIR mutations 全绿。
- [x] Native/Clang >=95% geometric mean，individual regression <=10%。
- [x] 0.11/固定 0.10 的冻结 C-oracle Clang 配对归一化 runtime regression <=3%
      geometric mean、<=8% individual；分母来自同进程固定编译器重放，历史数字不改。
- [x] schema 6 核对独立 V0.10 bundle/源码/编译器/实际产物摘要、两版本两模式的八通道
      交错顺序与全部原始样本；不存在缺少重放证据时的历史数字 fallback。
- [x] proof-loop checked/unchecked throughput >=97% geometric mean。
- [x] KIR/MIR optimizer time median <=2x、individual <=3x，fallback 有稳定 reason。
- [x] 六 native host 与两 performance runner 对同一 commit 全绿；运行时比较采用
      同进程双版本重放与冻结 C oracle 校准，不假定跨 worker 比率或物理机器相同。

## F. Git 交付边界

- [x] worktree branch 最终提交完整，working tree clean。
- [x] feature branch remote（若用于 CI）指向同一最终 SHA。
- [x] `main` 未移动、未合并；未创建 PR/tag/Release；未 force-push。
- [x] 最终回复只交付 branch/SHA/证据与风险说明，等待用户审查。

## 最终证据

实现与证据候选 SHA `202f950195b4d1160c60c0a518149617705910e3` 已通过同 SHA workflow
[`33403148950`](https://github.com/luxine/Rust_CalcKernel/actions/runs/33403148950) 的十项
required jobs；详细 job ID、日志摘要与原生性能报告哈希见
`11-release-candidate-acceptance.md` 的“最终同 SHA 验收”。两架构关键数值为：

- x86-64：unchecked Native/Clang `0.9995`、V0.10 replay `1.0009`，checked
  `0.9938` / `1.0064`，proof-loop `0.9998`，optimizer suite `1.4556`；
- AArch64：unchecked Native/Clang `0.9994`、V0.10 replay `1.0012`，checked
  `0.9997` / `1.0001`，proof-loop `1.0023`，optimizer suite `1.3695`。

本机最终总审通过 default `484`、all-features `615`（Native `102`）、release lib `53`、
IR `58`、generated `3`、mutation `10`、fact `7`、verifier cache `5`、docs `18`、artifact
`5`，并通过 fmt、两套 clippy、prefix、release build/sign/compiler/artifact/JIT audit 与
diff gate。Apple 主机上的 sanitizer 按契约记录为 Linux-only unavailable，没有把不可用伪装为通过。

Git 边界核验：任务起点及当前本地 `main` 均为
`794250fab28e78c0cf1c944d9eb0f342bb093d9e`，`origin/main` 为
`df816502876fba41676f9ebc190e4fadd18cd5a5`；未创建 PR、tag 或 Release，未合并、未
force-push。由于提交不能在自身内容中预写自己的 SHA 与未来 workflow run ID，本次最终证据
文档提交只引用已经完成的实现候选 run；提交后必须在该最终 SHA 上再次跑完整十项 workflow，
且不再修改 tracked 文件。最终回复以远端 feature branch SHA 和该次复跑 URL 作为不可自指的
最后交付证据。
