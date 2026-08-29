# 阶段 04 任务：Memory SSA、alias 与跨过程效果

## 目标

实现统一 region/alias query service、symbolic byte interval、Memory SSA 和 SCC effect
summary，并用同一 summary 完成源码 memory ceiling 的 CK2016 检查。

## 仓库落点

- `src/optimizer/analysis/{regions.rs,alias.rs,memory_ssa.rs,effects.rs}`。
- 新建 `src/ir/effects.rs`，放置 canonical effect lattice、parameter mapping、确定的 SCC
  solver 与输入 adapter trait；typed source graph 和 KIR call graph 都调用该 solver，避
  免 frontend 反向依赖 optimizer 或复制规则。
- `src/frontend/typeck.rs`/`CheckedProgram`：通过 typed-source adapter 保存已验证 summary
  与 CK2016 diagnostics，`check` 命令无需执行优化也能报告 ceiling 失败。
- `src/ir/kir/validate.rs`：验证 partition/version/memory phi。
- `tests/ir/memory_ssa.rs`、`tests/optimizer/alias_effects.rs`、扩展 frontend contract tests。

## TDD 顺序

1. 写 root/sub-slice/zero-length/symbolic interval red tests；实现 stable region identity 与
   byte interval，不把 descriptor copy 当新 allocation。
2. 写 pairwise noalias 与第三-root 反例；实现 shared alias query，查询只能返回
   NoAlias/MayAlias/MustAlias 并附 FactId。
3. 写 load/store/call/join/loop memory-version red tests；实现 partition merge 与 memory
   phi。未知 alias 只允许保守合并。
4. 写 effect SCC red tests：direct、transitive、recursive、print、may-fail、unsafe、unknown
   budget、sub-slice 参数回映射、raw pointer/all。
5. 写 CK2016 red tests：none/read/write/readwrite ceiling 的正反例、private local 排除、
   alias root 写入、transitive callee、unknown all；将共享 summary 接入 `check()`。
6. 写 malformed partition、错误 memory phi、错误 effect ceiling proof mutations。

## 实现判定

- source ceiling 只约束 externally reachable memory；summary 仍完整保存其他 ordered
  effects。
- pairwise noalias 不升级成 parameter-wide promise。
- effect fixed point 单调且确定；超预算精确退化为规范的保守 summary。
- frontend CK2016 与 KIR/backend 使用 `src/ir/effects.rs` 的同一个 lattice、mapping 和
  SCC solver，只允许输入 adapter 不同，不允许两套规则各自演化。

## 明确不做

不做 load forwarding/DSE/LICM，不发出 LLVM/C 属性，不删除 guard。
