# 阶段 01 任务：CKTUNE01 格式、检查器与 inspection

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

建立独立于 CLI、runner 与 Native backend 的调优数据核心：冻结 schema 常量、完整 Rust record model、
checked big-endian TLV codec、自包含 cross-record checker，以及 exact JSON/text tree renderer。此阶段只用
deterministic fixtures，不产生真实候选。

## 仓库落点与接口

- 新建 `src/tune/{mod.rs,schema.rs,decision.rs,inspect.rs}`，从 `src/lib.rs` 导出 `TuneDecision`、
  `TuneDecisionError`、`decode_tune_decision`、`encode_tune_decision`、`inspect_tune_json`、
  `inspect_tune_text`。
- `schema.rs` 集中定义 `CKTUNE01`、五个 schema 1、32 MiB/collection bounds、全部 enum discriminant、
  `BudgetPreset::contract()` 和 domain-separated digest helper；不得在 CLI/backend 复制常量。
- `decision.rs` 使用显式 record structs 与 `Cursor`/`Writer`；decoder 先检查长度/count/checked arithmetic，
  再分配，并提供 `TuneDecision::validate_self_contained()`。
- 新建 `tests/tune.rs` 以及 `tests/tune/{decision_format.rs,inspection.rs}`；新增规范列出的五个
  `tests/fixtures/tune/decision-schema1-*` fixture，并在测试中固定 SHA-256。

## TDD 顺序

1. 写 `decision_schema_one_rejects_noncanonical_framing_and_limits` RED，覆盖 endian、tag 顺序、
   duplicate/unknown、truncation、trailing、optional discriminant、UTF-8/NFC/NUL、count/length overflow、
   32 MiB 和每项上限；运行 `cargo test --test tune decision_schema_one_rejects`，预期因
   `decode_tune_decision` 缺失而失败。
2. 定义完整 record model 和 schema constants，实现只接受唯一编码的 primitive/list/record/optional
   cursor；再次运行同一测试，预期 framing mutation 全部通过。
3. 写 `decision_round_trip_matches_five_normative_fixture_digests` RED，构造 baseline/tuned 决策，要求
   encode→decode→validate→encode byte-identical，并固定五个 fixture 摘要；实现所有 tag 1..8 codec。
4. 写 `decision_checker_rederives_every_self_contained_equality` RED，逐项伪造 policy/session/plan/
   permutation/minimum/median/Q32/stability/rank/outcome/certificate/replay/cache-origin digest；实现 checked
   `u128` 派生和完整 terminal-state/selection table。
5. 写 `inspection_schema_one_is_exact_and_path_free` RED，要求 JSON key/node/type/order/escaping 与 text
   DFS path 完全匹配 attachment，且无绝对路径/时间/PID；实现两个 renderer 共享同一 validated tree。
6. 运行 `cargo test --test tune decision_ -- --nocapture`、`cargo test --test tune inspection_ -- --nocapture`
   和 `cargo test --locked`，预期全绿；将 RED/GREEN 与 fixture digest 写入阶段证据。

## 实现边界

- 自包含 checker 只能重算 decision 内部闭包；依赖 source/KIR/artifact 的等式由阶段 04 checker 完成。
- 不通过 serde 的宽松 map 反序列化绕过 field order/duplicate 检查；不暴露接受未知 tag 的兼容模式。
- inspection 不能新增 friendly summary；每个 wire node 必须恰好出现一次。
