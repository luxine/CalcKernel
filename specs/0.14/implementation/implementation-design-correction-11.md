# 实施期设计复诊 11：远程失败必须触发修复

## 阻断复诊

首轮 exact-SHA CI 已明确给出四类真实阻断，继续轮询旧运行不能形成验收：

- v0.12 x86-64 modular reduction 只达到较快手写 SIMD oracle 的约 49.5%；
- v0.14 仍把已审计的 x86 horizontal-reduction LLVM handoff 计为 KIR vector loop；
- Linux AArch64 profile/dispatch runtime 的通用原子 builtins 会产生未闭合 compiler helper；
- Darwin profile runtime 通过 `errno` 与 `fstat` 引入 `___error`、`_fstat$INODE64`，而旧
  v0.13 replay 又固定在会触发 void-value LLVM assertion 与 profile flush 失败的提交。

这些都是实现或基线身份错误，不是性能噪声，也不能通过降低阈值、减少 corpus、跳过宿主或
等待旧运行解决。

## 决议

- x86-64 O3 在 LLVM pipeline 前对整数内存归约执行 mem2reg，并给已识别 loop 附加固定
  `llvm.loop.interleave.count = 8`；不改变 CK/KIR 安全语义和性能门槛。
- v0.14 接受 x86 horizontal reduction 的 audited native handoff，KIR vector loop 期望数按
  实际策略重算，最终机器码仍由 schema 7/9 性能门禁约束。
- Linux AArch64 的 profile 与 dispatch 原子使用内联 acquire/release LL/SC，避免
  `__atomic_*` helper；其余 Unix 继续使用 always-lock-free C11 atomics，Windows 继续使用
  Interlocked。
- Darwin 使用 retained directory descriptor 上的 `fgetattrlist(ATTR_CMN_DEVID |
  ATTR_CMN_FILEID)` 校验 dev/inode，以 create-new 失败后的 no-follow probe 区分 collision；
  不读取 `errno`，也不调用 `fstat`。`_fgetattrlist` 作为显式冻结的 libSystem import。
- v0.14 schema-8 replay 固定到包含跨宿主闭包及本轮归约修复的 v0.13 候选
  `4c46ac36e0c8c71535c0bcdba76bf1faf16e4836`，并重算 manifest SHA-256；旧失败提交不得代签。

本修订不改变语言、KIR 3、Bridge ABI 4、Native ABI 1、Runtime ABI 2、CKPROF01、CKPART01、
CKTUNE01、性能 corpus、统计规则、门槛或十作业拓扑。新提交必须重新执行本地全量验收与
exact-SHA 远程 CI；轮询仅用于发现结果，任何新失败都必须先复诊再修复。
