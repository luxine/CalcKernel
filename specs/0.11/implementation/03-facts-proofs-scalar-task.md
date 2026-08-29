# 阶段 03 任务：事实、证明与标量分析

## 目标

建立 Proven/TrustedContract facts、封闭 proof certificate、独立 checker，以及组合
interval/affine/congruence/known-bits 的 path-sensitive 标量分析。此阶段不删除 guard。

## 仓库落点

- `src/optimizer/facts.rs`：稳定 FactId、origin、scope、derivation DAG。
- `src/optimizer/proof.rs` 与 `src/optimizer/verify.rs`：certificate model 与独立 checker。
- 将 `src/optimizer/analysis.rs` 拆为
  `src/optimizer/analysis/{mod.rs,scalar.rs,affine.rs,congruence.rs,known_bits.rs,budget.rs}`。
- `tests/ir/proofs.rs`、`tests/optimizer/scalar.rs`。

## TDD 顺序

1. 写 FactId/order/origin/scope red tests；实现 arena 和 deterministic serialization。
2. 写 contract import red tests：entry substitution、unsafe call-instance、inline clone scope、
   recursive fresh instance、事实不得逃逸到 caller entry。
3. 对每个 scalar domain 写 lattice/transfer/property tests，包括 signed/unsigned extremes、
   modular wrap、checked may-fail、strict comparisons、branch refinement。
4. 写 loop widening/narrowing convergence red tests；实现 KIR-size 派生的固定预算和
   `unknown` fallback，不读取 wall clock。
5. 写 certificate checker red tests；实现 dominance/path/transfer/invariant 检查。测试中
   使用一个“分析器故意给错结论”的 fake producer，证明 checker 不重新信任 producer。
6. 写 stale fact、错误 origin、错误 call instance、错误 invariant、预算篡改 mutation；所有
   都必须编译器错误且不产 artifact。

## Checker 边界

checker 可以验证声明的 block transfer invariant，但不能调用生产 range analyzer 询问
“结论是否成立”。TrustedContract 叶只验证来源、类型、实参替换和 dominance。所有 pass
preservation 声明默认不可信。

## 明确不做

不实现 alias/Memory SSA effect，不删除 guard，不执行 O1 pass，不映射 backend 属性。
