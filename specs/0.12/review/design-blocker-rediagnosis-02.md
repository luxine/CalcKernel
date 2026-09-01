# CK 0.12 第二轮阻断项复诊

日期：2026-09-01
输入：`design-adversarial-review-02.md`
结论：**B5、B6 均成立。**

## B5 复诊：成立

设计已经为 Loop SIMD 固定 20% 收益门槛，为 SLP/specialization 固定 10% 加两个绝对
cost unit，但没有覆盖独立 scalar full/partial unroll。“cost driven”和“enough branch
cost”不是 checker 可执行的判定式；O3 frontier 也不能在候选进入比较前推导隐含门槛。

修订规定：scalar full-unroll、scalar partial-unroll，以及 unroll-plus-SLP 的组合方案都必须
相对同一 pre-state 预测至少 10% loop execution-cost reduction 且至少两个绝对 cost unit；
同时仍满足各自 trip/body/factor/growth 条件。Loop SIMD 保持 20%。每个方案先独立过门槛，
再由 frontier 比较所有已接受方案，不降低任何既有阈值。

## B6 复诊：成立

`KirValueType` 的 lane count 是一般正 u16，无法反向限定 0.12 profile builder 的探测全集。
若不给固定集合，profile 的 mandatory-entry 校验也不知道一个缺失 entry 是“不支持”还是
“根本未探测”。

修订把 schema 1 的 Native fixed-vector probe domain 固定为 lane count `{2, 4, 8, 16}`，
且总 vector width 不超过 512 bit；与五种 lane type 形成有限候选。每个候选都必须明确
记录 Legal 或 Unavailable，不能省略。Mask 使用同 lane count。TTI/legalization/cost 再从
这个全集筛选，target 不会因为进入全集就被承诺支持该 width。

## 复诊后的判定

修订范围只补齐 deterministic acceptance 与 profile universe，不改变源码、ABI、安全语义、
性能门槛或目标平台。完成双语修订后进入第三轮完整审查。
