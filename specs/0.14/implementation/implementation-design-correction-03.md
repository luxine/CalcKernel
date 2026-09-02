# 实施期设计复诊 03：layout-only 不得抑制固定 KIR O3 后缀

## 阻断复诊

最终证据链审计构造了一个仅选择 layout 的计划，并把重放后的 KIR（移除 layout
元数据）与 empty-plan 的普通 O3 KIR 比较。原实现直接返回带 layout 的调优前状态，
两者不相等；这意味着布局候选会用未完成普通 O3 的 KIR 参加测量，违反固定流水线、
普通基线可比性和“layout 只改变布局”的安全边界，属于真实阻断项。

直接在调优前 KIR 上附加 layout 后再运行完整 O3 也不可行：普通 O3 可以内联并创建
新基本块，旧的完整排列随即不再是当前函数的完整排列，KIR 验证会正确拒绝。问题是
原设计没有明确调优前 block identity 穿过固定 KIR O3 后缀时的投影规则。

## 修订决议

1. layout-only（以及只包含 layout 前早期、未物化循环/向量改写的合法计划）先完成
   固定普通 KIR O3 后缀，再附加 layout 元数据。
2. 选定排列按原顺序保留仍存活的 block id；固定后缀新建且未被选择的 block 按后缀
   模块规范顺序追加，得到当前函数的完整排列。
3. 没有选定 block 存活，或最终排列等于当前规范顺序时，不附加元数据，候选作为可测量
   no-op 继续；不得引用消失 block，也不得跳过 O3。
4. 已经物化 short-slice、Loop SIMD、unroll 或 SLP 的状态仍不得重新进入完整 O3；与
   layout 不兼容的 whole-plan expansion 继续以既有非法 disposition 记录，不中止搜索。
5. source-aware replay 以相同输入重算该投影；计划、post-state 和 KIR digest 因而仍然
   确定且可独立验证。

## 验证与门槛

新增回归测试要求 layout-only 重放移除元数据后与 empty-plan 普通 O3 KIR 完全相同，
并保留原有 canonical layout、非法跨类 expansion、全量 optimizer/native/tune 测试。
该修订没有降低性能、安全、资源、身份或 CI 门槛，只补全固定流水线的逻辑闭环。
