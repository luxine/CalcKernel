# 阶段 14 验收：Predicated-Update 分析

## 本地必须通过

- [ ] `cargo test --locked --test optimizer predicated_update_discovery_ -- --nocapture`
- [ ] `cargo test --locked --test optimizer -- --nocapture`
- [ ] `cargo test --locked --test optimizer tuning_ -- --nocapture`
- [ ] `cargo test --locked --test contracts docs_v0_14_should_describe_only_the_implemented_optimizer_boundary -- --nocapture`

## 正向断言

- [ ] exact-place load 支配 compare/conditional store，Memory SSA old/new merge
  唯一，unit-stride 访问和 strict-Lt 被识别。
- [ ] 同一 scalar site 保留不同合法 VF/UF variant；stable key 绑定 shape 与
  compare/load/store/polarity。
- [ ] checked candidate 只有在每 lane bounds/overflow/first-failure 可证时出现。

## 负向断言

- [ ] 不同 place、别名不明、intervening write、双臂写、多写、ordered effect、
  非 unit stride、坏 memory phi、非 strict compare 全部保持 scalar。
- [ ] pure diamond/reduction 行为与 ordinary static-profitability gate 无回归。
- [ ] KIR 3、语言、ABI 与 CKTUNE01 没有字段变化。

## 完成证据

候选 stable text、正负用例计数、fallback reason 与命令输出写入
`target/acceptance/v0.14/stage-14/`。
