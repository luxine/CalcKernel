# 阶段 04 验收：trial 与 source-aware replay

## 必须通过

- [ ] `cargo test --test tune trial_ -- --nocapture`
- [ ] `cargo test --test tune replay_ -- --nocapture`
- [ ] `cargo test --test native object_ -- --nocapture`
- [ ] `cargo test --test native artifacts_ -- --nocapture`
- [ ] `cargo test --all-features --locked`

## 结构断言

- [ ] trial typestate 无生产 publish/ordinary cache 转换，runner 只能取得 private staged artifact。
- [ ] object graph、link recipe、primary/header/import identity 有唯一 canonical 派生并被 replay 重算。
- [ ] decision trial 集与 compile selection 精确相等；每个 trial 在隔离 cache 独立重建。
- [ ] size rejection 和 finalist selection 从完整 trial 集重算，最高排名成员不能被遗漏。
- [ ] selected/baseline replay 代码身份匹配，最终 artifact 无 tune/runner/dispatch/runtime 依赖增长。

## 完成证据

保存被测 SHA、typestate negative、isolated rebuild、artifact digests、dependency audit 与测试计数到 `target/acceptance/v0.14/stage-04/`。
