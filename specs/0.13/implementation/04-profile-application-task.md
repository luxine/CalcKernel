# 阶段 04 任务：profile application、confidence、cost 与 mapping transfer

## 目标

把已验证 `.ckprof` 导入为 immutable non-proof analysis：精确 identity/site compatibility、饱和传播、
edge reconstruction、confidence/hotness/dynamic-work、signed histogram lower bound 与 checked `u128`
收益计算，并形成后续 O2/O3 共用的 profile decision sidecar/explanations。此阶段不改变 KIR。

## 仓库落点

- 新增/修改 `src/profile/{analysis.rs,apply.rs,cost.rs,identity.rs,inspect.rs}`。
- 修改 `src/optimizer/{analysis/mod.rs,kir_pipeline.rs,audit.rs}`，新增 immutable profile analysis
  handle、closed mapping-transfer record 与 stable explanation，但不写 fact/proof arena。
- 修改 `src/cli/commands.rs` 的 Native build/emit-kir use path 与 `--explain-optimization` 输出。
- 新建 `tests/pgo_analysis.rs`，扩展 profile mutation、optimizer preservation/transaction 和 CLI
  inspection/explanation tests。

## TDD 顺序

1. 写 compatibility RED：逐字段比较完整 identity/site table，报告首个 stable field path 及 expected/
   observed digest；topology、target、safety、O-family、CPU/target-set 或 compiler contract 不匹配拒绝。
2. 写 counter RED：saturated site 为 unknown；依赖它的 reconstructed edge 全部 unknown；未饱和方程
   必须唯一、非负、一致，否则 malformed；trip `>u32::MAX` 只进末桶并带 saturation flag。
3. 写 confidence RED：128 observations、90% branch/constant、85% histogram、1% cold、90% module
   work coverage 与 per-root 1% 门槛全部用 checked cross multiplication，边界等号/零样本固定。
4. 写 work RED：target static cost × exact counts 使用 saturated `u128`；任一 contributing saturated/
   overflow 使该 work/rank unknown，stable function identity tie-break，不从部分值猜测。
5. 写 PGO cost RED：每个 histogram bucket 对所有 `u32` 值证明 signed lower bound；full fallback、
   misses、guard cost 全计入 checked signed-magnitude sum；overflow/tie/indeterminate/fractional ambiguity baseline。
6. 写 mapping-transfer RED：任何 CFG change 使旧 mapping invalid；只有独立 checker 认可的 closed
   one-to-one/sum mapping 可转移 counts，否则 affected site unknown；伪造/遗漏记录是 compiler error。
7. 写 explanations/determinism RED：coverage、ignored reason、work rank、counter IDs、fallback 原因
   稳定且不依赖 map/order/path；low confidence 是 normal fallback，identity/mapping forge withholding output。
8. 实现 parser 后的 validation/application 层、整数算法与 independent cost/mapping checkers；接入
   Native emit-kir/build 与 `pgo build` final-use handoff 的 read-only sidecar，证明 ordinary/off artifact
   仍 byte-stable。

## 实现边界

- profile analysis 不能移除检查、改变 alias/range/alignment/effect/strict-f64 proof 或自行 clone CFG。
- 阶段 04 只计算/解释 layout 与 O3 candidate 输入；阶段 05/06 才允许 verified transform。
- 所有 threshold、bucket 与 cost formula version 都集中定义并进入 profile/cache identity。

## RED/GREEN 证据

记录 identity mismatch、saturation propagation、bucket counterexample、checked arithmetic overflow、
forged mapping RED 及 ordinary off reproducibility digest到 `target/acceptance/v0.13/stage-04/`。
