# CK 0.11 事实驱动优化器总验收

## 使用方式

只有 01–11 每份阶段验收都记录通过证据后，才能执行本清单。任何一项失败都返回对
应阶段修复；不得在本清单修改阈值、排除 case 或增加 ignore。

## A. 设计与范围

- [ ] 双语规范语义一致，第一轮 B01–B08 均有修订，第二轮 0 blocker。
- [ ] 0.12+ SIMD/unroll/specialization/PGO/Auto-Tuning/fast-math 未进入实现。
- [ ] main、safe/unsafe boundary、immediate-UB 与 sanitizer 调试语义符合规范。

## B. 编译器闭环

- [ ] consumer roots/capability check 先于 mode-specific KIR。
- [ ] KIR scalar SSA、region Memory SSA、ordered guard/effect 全部通过 structural mutation。
- [ ] Fact origin/scope、closed certificate 与 independent checker 通过错误 producer mutation。
- [ ] scalar/alias/effect/loop analysis 超预算保守且确定。
- [ ] O0/O1/O2/O3 exact pipeline 与 verifier-after-every-pass 成立。
- [ ] Release 验证缓存独立核对完整状态；错误的 no-change 声明不能放过 IR、proof、
      guard rewrite 或 contract fact mutation。
- [ ] unsafe inline/recursive contract instances 不逃逸。
- [ ] C/WASM/Native 全部消费 verified KIR；无正式双优化路径。

## C. Backend、安全与 ABI

- [ ] C/WASM/Native O0–O3 supported-mode differential 全绿。
- [ ] checked first-error、print count/order、strict f64、break/continue、void/slice 保持。
- [ ] pairwise noalias 不放大成 parameter-wide noalias/restrict。
- [ ] CK LLVM fact audit 位于 LLVM optimize 前并拒绝所有注入 CK-owned strengthening。
- [ ] canonical proof loops 的 KIR/backend hot loop 无冗余 bounds guard。
- [ ] normal build 无 contract checks；sanitizer 精确 CKR0007/LF/246 且极值无 host UB。
- [ ] Native ABI=1、Runtime ABI=2、private Bridge ABI=2，六 release artifact 仍零工具链依赖。
- [ ] ELF producer metadata non-`ALLOC` 且绑定 pinned LLD；Darwin hardened entitlement 与唯一
      `allow-jit=true` policy canonical 等值，不以人类展示格式替代语义比较。
- [ ] Darwin 实际 compiler 在严格签名审计前显式 ad-hoc hardened 签名，打包原件与
      被审计文件一致；签名验证未放宽。
- [ ] Runtime 输入参与 bootstrap cache identity、验证后立即保存；Darwin Small+PIC 不产生
      internal-call absolute text fixup，dyld C-ABI `LC_MAIN` entry 通过完整执行验收。

## D. CLI、兼容与文档

- [ ] emit-kir/facts/effects/explain byte-deterministic 且无环境泄漏。
- [ ] sanitizer flag 仅 run/executable，cache 与 normal artifact 隔离。
- [ ] unsafe header normalized comments 可映射 flattened slice fields，ABI shape 不变。
- [ ] 所有 v0_10 与 v0_11 compatibility manifest evidence 可执行。
- [ ] current English/zh-CN docs 同步为 0.11.0；version/help/licenses/release policy 一致。

## E. 质量、性能与分发

- [ ] fmt、all-feature clippy、default/all-feature tests、sanitized ownership 全绿，无 ignore。
- [ ] fixed-seed generated differential 与全部 KIR mutations 全绿。
- [ ] Native/Clang >=95% geometric mean，individual regression <=10%。
- [ ] 0.11/固定 0.10 的冻结 C-oracle Clang 配对归一化 runtime regression <=3%
      geometric mean、<=8% individual；分母来自同进程固定编译器重放，历史数字不改。
- [ ] schema 6 核对独立 V0.10 bundle/源码/编译器/实际产物摘要、两版本两模式的八通道
      交错顺序与全部原始样本；不存在缺少重放证据时的历史数字 fallback。
- [ ] proof-loop checked/unchecked throughput >=97% geometric mean。
- [ ] KIR/MIR optimizer time median <=2x、individual <=3x，fallback 有稳定 reason。
- [ ] 六 native host 与两 performance runner 对同一 commit 全绿；运行时比较采用
      同进程双版本重放与冻结 C oracle 校准，不假定跨 worker 比率或物理机器相同。

## F. Git 交付边界

- [ ] worktree branch 最终提交完整，working tree clean。
- [ ] feature branch remote（若用于 CI）指向同一最终 SHA。
- [ ] `main` 未移动、未合并；未创建 PR/tag/Release；未 force-push。
- [ ] 最终回复只交付 branch/SHA/证据与风险说明，等待用户审查。

## 最终证据

执行完成后在此追加最终 SHA、完整命令摘要、workflow run、性能四门槛数值和 `main`
对照 SHA。清单未全部勾选时不得宣称任务完成。
