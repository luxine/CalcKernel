# 0.12 实现阻断复诊 02：本地测量稳定性与 schema 7 失败诊断

## 证据

- 用户明确要求在本地重新运行完整性能验收。候选的 benchmark producer 正常完成并写出 schema 7
  报告，独立 checker 在任何 throughput 阈值判断前拒绝
  `scalar unchecked/branch_mix nativeSamplesNs is unstable around its median`。
- 该 stream 的 20 个样本形成约 7.55–9.87 ms 与 11.36–12.78 ms 的双峰；checked stream 也在
  8.39–13.32 ms 间双峰。相邻较长 corpus 的样本保持集中，说明报告、通道轮换和 runner 仍在
  工作，但短 kernel 受到系统调度干扰。
- 测量前后环境快照同时记录到高负载 WindowServer、虚拟机、VS Code、Docker 与系统守护进程。
  因此本轮报告是被测量质量门禁正确拒绝的诊断证据，不能证明 CK 达标或不达标，也不能通过
  反复抽样、删除异常值或修改 25%/80% 稳定性规则获得通过。
- 按冻结流程执行同 worker failure diagnostic 时，脚本读取已由 schema 7 删除的
  `report["runtimeReplay"]` 并以 `KeyError` 退出。现有 contract test 只搜索宽泛子串
  `runtimeReplay`，未证明 v0.11/v0.10 两个实际 replay 字段都被消费。

## 复诊结论

1. 保持每 kernel 90%、架构 geometric mean 95%、domain >5%、scalar regression、size、
   compile-time、采样顺序、upper median 与稳定性门槛全部不变。本轮不签署本地性能通过。
2. 诊断脚本改为显式要求 v0.11 与 v0.10 两个 replay bundle，并覆盖 schema 7 的 32 个
   candidate/current/replay-Clang measured artifacts 与两代各 8 个 replay Native artifacts。
3. 每个对象仍须使用报告中的固定 basename 与 SHA-256 验证后才可反汇编；诊断不重新构建、
   不重新计时、不调用 checker，也不替代 required gate。每轮先截断整库哈希清单，避免旧证据
   追加污染新证据。
4. 先以 contract RED 证明旧脚本缺少 v0.11/schema 7 支持，再以真实失败报告和两套固定 replay
   bundle 验证 48/48 对象闭环。该修复产生新候选 SHA，旧 exact-SHA CI 不能代签，必须重新运行。
