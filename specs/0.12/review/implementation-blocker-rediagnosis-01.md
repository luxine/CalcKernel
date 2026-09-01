# 0.12 实现阻断复诊 01：x86 target subset 与 COFF `_fltused`

## 证据

- 候选 `ae1a192d597fe7a75de21078196c97e8f27ecffa` 的 exact-SHA CI
  `33515106818` 首先证明 x86-64 baseline TTI 会在未改变的 20% 门槛下拒绝 strict-f64
  division 与 horizontal modular-multiply reduction；优化器正确保持 scalar，原测试错误地把
  supported source surface 当成跨 target 必然 accepted surface。
- 修订候选 `e0ba28a537144a8bd6936cebd960d3cdf7d79e70` 的 exact-SHA CI
  `33516366766` 进一步证明第一轮修订把 strict-f64 fallback 断言错放进相邻普通 i32 SIMD
  用例；Linux x86-64 实际结果是普通 loop `1`、strict-f64 division `0`。这是测试归属错误，
  不是 cost model 或 optimizer 错误。
- 同一轮 Windows x86-64 在 vector differential DLL 链接时报
  `undefined symbol: _fltused`。固定 LLVM 22.1.8 的 `X86AsmPrinter` 明确会为 MSVC 浮点
  module 发出该未定义引用；LLVM 自有无 CRT COFF 测试也显式定义 `_fltused`。现有定义只在
  embedded `format_float.obj`，但 public Native DLL 链接按契约不引入 runtime object，因此
  这是生产 compiler-support closure 缺口。

## 复诊结论

1. 保持 target profile、20% Loop SIMD 门槛、10% SLP/specialization 门槛及所有成本不变。
2. 普通 i32 SIMD 用例继续要求所有 pinned x86-64 host 接受；只在真正 strict-f64 division
   用例断言 stable profitability fallback。Reduction 仍按 target 断言精确 accepted subset。
3. x86-64 MSVC CK object 注入 non-exported `weak_odr`/COMDAT-any `_fltused = 0`；runtime 的
   既有定义改成语义等价 `selectany`，使 DLL-only 与 executable/runtime 两条路径都闭包且
   共同链接时可合并。五对象 runtime manifest、CRT-free policy、public Native ABI 1、Runtime
   ABI 2、bridge ABI 3 均不改变。
4. 必须由新的 exact-SHA Windows x86-64 real link、六 host native suite 与十项 CI 全绿证明；
   源码 contract 和本地 COFF experiment 只能作为补充，不能替代 Windows runner。
