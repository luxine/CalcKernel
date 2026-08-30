# 阶段 02 验收：KIR 核心

## 必须通过

1. `cargo test --locked --test ir kir_ -- --nocapture`
2. `cargo test --locked --test ir mir_ -- --nocapture`
3. `cargo test --locked --test optimizer optimizer_should_preserve -- --nocapture`
4. `cargo test --locked --test backend -- --nocapture`
5. `cargo fmt --check`
6. `cargo clippy --all-targets --locked -- -D warnings`
7. `git diff --check`

## 结构断言

- O0 KIR 对所有有效 CFG 通过 validator；每种定向 mutation 被拒绝且错误稳定。
- checked 与 unchecked 的唯一差异来自 `KirBuildConfig` 规定的 guard/effect，不来自
  backend 私有补丁。
- artifact 裁剪发生在 effect/SSA 构造前；不可达 runtime call 不进入 KIR。
- KIR printer 确定且不改变 `emit-mir` bytes。
- 本阶段没有任何 guard elimination。

## 完成证据

阶段 11 切换后的 I24 复诊见 `../review/implementation-blockers-01.md`：原第 3 条
命令在旧 MIR tests 删除后匹配 0 项，当前尚未重新签收。必须将原三个 preservation
断言组迁到 verified KIR，并恢复该命令的 3 项实际测试；不能以 exit 0 的空运行代签。

- 日期：2026-08-29
- 实现提交：`9427203`
- `cargo test --locked --test ir kir_ -- --nocapture`：18 passed，0 failed，0 ignored。
- `cargo test --locked --test ir mir_ -- --nocapture`：24 passed，0 failed，0 ignored。
- `cargo test --locked --test optimizer optimizer_should_preserve -- --nocapture`：3 passed。
- `cargo test --locked --test backend -- --nocapture`：42 passed。
- `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、
  `git diff --check`：全部通过。
- 补充全仓验证：`cargo test --locked` 共 266 passed，0 failed/ignored；配置 LLVM
  环境后执行 `cargo clippy --locked --all-targets --all-features -- -D warnings` 通过。
- checked KIR snapshot 明确打印算术结果与 overflow condition，并立即跟随配对 guard；
  除法依次打印除零与有符号溢出 guard；slice/sub-slice 打印边界 condition/guard；
  region、memory version、block parameter 与 effect order 均使用稳定 ID。
- unchecked KIR snapshot 不含 checked guard，整数算术明确标记 modular；同一输入重复构建
  50 次得到完全一致的 KIR，printer 不包含路径、地址或时间戳。
- mutation 覆盖缺失 guard、先使用后定义、错误 edge arity/type、非法 region partition、
  未定义 memory version 等情形，validator 均稳定拒绝。
