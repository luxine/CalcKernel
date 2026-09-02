# 实施期设计复诊 04：Schema 9 必须在原生 runner 内计时

## 阻断复诊

最终性能证据审计发现，首版 full collector 虽然在计时前完成动态加载和符号解析，
但用 Python 循环逐次经 `ctypes` 调用 kernel。这样每个“原生迭代”都夹带 Python
循环与 FFI 边界开销；尤其 `contract-fixed-length` 的 16 元素 kernel 会由调用开销
主导，既不能证明机器码吞吐，也会把真实的领域优化差异稀释到 8% 门槛以下。该问题
违反“同算法、同硬件机器码比较”和“harness I/O 不进入稳态计时”，属于方法学阻断。

## 修订决议

1. 保留的 `ckc-tune-runner` 增加与调优 `CKTUNE/1` 隔离的 `--ck-perf` 单次协议。
2. Collector 以参数传入 artifact、case、case-id、长度、seed、参数和迭代数；直接启动
   runner，不使用 shell，并清空环境。
3. Runner 在动态加载、符号解析和输入/输出分配完成后启动单调时钟，仅围绕原生函数
   指针迭代循环计时；循环结束后才生成结果字节和摘要。
4. Runner 输出唯一的 `CKPERF/1` 行，回显 case-id、seed、请求/完成迭代数、正 u64
   纳秒和摘要。Collector 对字段数、回显、范围与冻结正确性摘要全部 fail-closed。
5. 校准、确认、3×7 预热、20×7 测量、轮转、稳定性、上中位数与所有阈值不变；
   进程启动成本只增加验收墙钟，不进入被比较的 `elapsedNs`。

## 验证与门槛

新增真实 C 动态库探针，要求 runner 在空环境下完成 1,000 次原生调用并返回冻结的
`contract-fixed-length.release` 摘要与正计时；Python 契约测试同时锁定 collector
使用 `--ck-perf` 且不再用 Python 循环形成计时值。该修订移除测量污染，没有降低
任何性能、正确性、资源、身份或 CI 门槛。
