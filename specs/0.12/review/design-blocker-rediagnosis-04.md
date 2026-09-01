# CK 0.12 第四轮阻断项复诊

日期：2026-09-01
输入：`design-adversarial-review-04.md`
结论：**B8 成立。**

## B8 复诊：成立

Specialization 在 O2 前发生，loop frontier 在 O2 后发生，二者却消费同一 original function
冻结 ledger；“按 LoopId”无法排列 call candidate。Controlled-unrolling 与 O3 frontier 的
scalar partial variant 也确有文字覆盖差异。

修订使用编译流水线本身作为第一层 stage rank：

1. specialization stage：按 caller FunctionId、Call InstructionId、callee FunctionId、
   canonical fact-set digest；
2. loop frontier stage：按 FunctionId、LoopId、candidate kind
   `LoopSimd|FullUnroll|PartialUnroll`、scalar/SLP variant、递增 VF/UF；
3. residual SLP stage：按 FunctionId、BlockId、root InstructionId、递增 lane count。

同一 key 不得出现两次。每个 stage 在进入下一 stage 前结束，之前消耗的共享 ledger 不重置。
Loop frontier 把 full/partial unroll 都写成“scalar-only 或加 SLP”的 alternative；SLP variant
继续要求 unroll+pack 原子提交。这样既恢复原受控展开范围，也使预算耗尽可复现。

完成双语修订后进入第五轮审查。
