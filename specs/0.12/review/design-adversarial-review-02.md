# CK 0.12 设计第二轮对抗性审查

日期：2026-09-01
审查对象：第一轮修订后的双语 CK 0.12 设计
结论：**暂不通过；2 个新暴露的阻断项。第一轮 B1-B4 与 M1 已正确关闭。**

## 已关闭项复核

- 专用化 trial 已改为完整 verified optimization state 的原子副本，且只运行 scalar
  finalization；clone 后续只经过一次正式 O2/O3，证据与 ID 无需跨 arena 拼接。B1 关闭。
- Native loop alternative 在同一 immutable scalar pre-state 上竞争，至多提交一个 winner，
  不再由 SLP 先后顺序抢占 Loop SIMD。B2 关闭。
- `emit-kir` 五个 consumer 与 `KirConsumer` 一一对应，Native CPU policy 和 main 要求明确。
  B3 关闭。
- TTI probe、cost kind、错误处理与 canonical digest 已固定。B4 的主体关闭。
- 规范明确没有 KIR artifact cache，并把已有 Native object/run cache 升级到独立 schema。
  M1 关闭。

## 阻断项 B5：scalar unroll 盈利门槛仍非封闭

Controlled-unrolling 只说 partial unroll “removes enough branch cost”，constant full-unroll
只限制 trip/body/growth；O3 frontier 又要求 checker 比较“accepted alternatives”。没有数字
门槛时，proposer 与 checker 无法独立得出相同的 accepted 集合，不同实现可能提交只有
一个 cost unit 收益却大幅增长代码的 unroll。

必须给不含 Loop SIMD 的 full-unroll、partial-unroll 和 unroll-plus-SLP 一个明确、共同且
可计算的最低收益规则，并说明先各自过门槛、再进入同 pre-state winner 比较。

## 阻断项 B6：Native TTI query 的 lane domain 仍循环定义

Target profile 先列出“legal fixed vector widths”，随后又把 TTI finite query domain 写成
“candidate fixed lane counts”。若 candidate 从 legal widths 得出，就必须先查询才能知道
legal；若由 bridge 自行枚举，不同 bridge 可选择不同最大宽度而仍声称合规。

必须在 CK schema 中直接固定 0.12 探测的 lane-count/bit-width 上限，再由 TTI legality、
legalization form 和 cost 把其中条目标成 Legal/Unavailable。这样 profile 缺项检查和 digest
才有唯一全集。

## 第二轮判定

B5-B6 都影响 independent checker 或 profile identity，不能降为实现细节。复诊成立后应
作一次小范围双语修订，再执行完整第三轮审查；第一轮已经关闭的契约不得回退。
