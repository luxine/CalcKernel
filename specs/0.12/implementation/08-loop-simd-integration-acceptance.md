# 阶段 08 验收：Loop SIMD、frontier 与端到端语义

## 默认特性必须通过

1. `cargo test --locked --test optimizer loop_simd_ -- --nocapture`
2. `cargo test --locked --test optimizer vector_checker_ -- --nocapture`
3. `cargo test --locked --test optimizer vector_frontier_ -- --nocapture`
4. `cargo test --locked --test optimizer vector_differential_ -- --nocapture`
5. `cargo test --locked --test cli vector_explanation_ -- --nocapture`
6. `cargo test --locked`

## Native 必须通过

在固定 `CKC_LLVM_PREFIX` 下：

7. `cargo test --all-features --locked --test native vector_ -- --nocapture`
8. `cargo test --all-features --locked --test native differential_ -- --nocapture`
9. `cargo test --all-features --locked --test native fact_audit_ -- --nocapture`
10. `cargo test --all-features --locked`
11. `cargo clippy --all-targets --all-features --locked -- -D warnings`
12. `cargo fmt --check`
13. `git diff --check`

过滤测试不得为 0 项。

## 结构与语义断言

- Accepted kernel 在 optimized KIR 和 pre-LLVM IR 都有预期 vector op；object disassembly 有
  pinned arch 的真实 SIMD，三层证据缺一不可。
- x86-64 MSVC f64 vector object 自带 non-exported coalescible `_fltused`，纯 Native DLL 在
  `/nodefaultlib` 下成功链接；与 embedded runtime 的 `selectany` 副本共同链接时无 duplicate。
- Accepted subset 由固定 target profile 和未降低的 20% 门槛决定；六 host 必须断言各自精确
  subset。合法但不盈利的 x86-64 strict-f64 division / horizontal multiply reduction 必须以
  稳定 `vector-profitability-threshold-not-met` 保持 scalar，不能被测试强制接受。
- Unknown-trip Loop SIMD 的 independently recomputed runtime admission threshold 必须至少为
  `max(computed break-even, target floor)`；x86-64 floor 是 `4 * VF * UF`，AArch64 floor 是
  `2 * VF * UF`。不得把 x86 control penalty 错误套到已证明双 chunk 盈利的 AArch64 path。
- Zero/short/overlap/misaligned/overflow predicate 走 original scalar fallback；exact/remainder
  coverage 无漏/重迭代或越界访问。
- Checked first-error、strict f64、print/effect order 与 O0 一致；unsupported reduction保持 scalar。
- Frontier 同 pre-state 只提交一个 winner；non-winner budget/audit 保留。
- C/Wasm、sanitizer、baseline feature containment 和全部 0.11 regression green。

## 完成证据

写入 `target/acceptance/v0.12/stage-08/`：实现 SHA、RED 摘要、fixture seed、各 VF/UF/winner/
rejection count、KIR/LLVM/disassembly 片段摘要、default/all-feature test count 与 toolchain identity。
