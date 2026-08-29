# 阶段 05 任务：O0/O1 携证优化

## 目标

建立正式 KIR pass manager。O0 只验证；O1 依次实现 CFG canonicalization、SCCP 与
range propagation、proof-carrying redundant-check elimination、DCE 和 cleanup。

## 仓库落点

- 重写 `src/optimizer/pipeline.rs` 以接收/返回 `KirModule`、pass record、analysis
  invalidation 和 deterministic explanation。
- 新建 `src/optimizer/passes/{cfg.rs,sccp.rs,check_elimination.rs,dce.rs,cleanup.rs}`。
- `tests/optimizer/kir_o1.rs` 与固定 KIR snapshots。

## TDD 顺序

1. 写 O0 pipeline red tests：validator-only、无 optional rewrite、错误输入拒绝。
2. 写 exact O1 pass-order red test，再实现 pass manager；每个 pass 后无条件 verifier。
3. 为 CFG canonicalization 写 empty block、constant branch、unreachable block、phi repair、
   ordered-effect barrier red tests。
4. 为 SCCP/range 写 constant、branch path、contract range、modular wrap、budget fallback red
   tests；实现事实和 certificate 输出，不静默修改。
5. 为 bounds/overflow/div-zero guard elimination 写最小正例与近邻反例。先观察 guard 仍
   存在，再只在 checker 接受 ProofId 后删除。
6. 为 DCE 写 pure dead value、unused region、runtime print、may-fail、call、contract
   sanitizer placeholder red tests；有序或可能失败 operation 不能因结果未用而删除。
7. 写 `--explain-optimization` 数据模型（尚不接 CLI）的 removed/retained/unknown reason
   determinism tests。

## 实现判定

- pass 不直接制造 Proven fact；它消费已验证 analysis result，并提交 rewrite certificate。
- rewrite 后旧 FactId/ProofId 根据 preservation 集合失效，再由 verifier 检查。
- guard elimination 保留 failure/print 相对顺序；无法证明即 retained，不作猜测。
- O1 不执行 inline、Memory SSA load/store rewrite 或 loop transform。

## 明确不做

不切换 backend，不实现 O2/O3，不将 LLVM 结果反馈进 KIR。
