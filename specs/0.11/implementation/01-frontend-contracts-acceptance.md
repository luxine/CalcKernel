# 阶段 01 验收：unsafe 与契约前端

## 必须通过

1. `cargo test --locked --test frontend contract_ -- --nocapture`
2. `cargo test --locked --test frontend lexer_ -- --nocapture`
3. `cargo test --locked --test frontend parser_ -- --nocapture`
4. `cargo test --locked --test frontend checker_ -- --nocapture`
5. `cargo test --locked --test ir mir_ -- --nocapture`
6. `cargo fmt --check`
7. `git diff --check`

## 结构断言

- valid contract 的 AST/CheckedProgram snapshot 同时包含 unsafe bit、规范化 affine terms、
  noalias/alignment/multiple_of 与 memory ceiling。
- safe CK fixture 的 `emit-mir` snapshot 与基线逐字一致。
- CK2014/15 对每类非法边界只有一个主诊断，span 指向最窄违规 token/clause。
- unsafe main 的 `CheckedProgram.entry` 为 `None`。
- 没有 backend、runtime、CLI 或优化行为变化。

## 完成证据

执行时追加 SHA、命令输出摘要和 test count；任何 ignored test 都判失败。

### 2026-08-29 执行记录

- 实现提交：`60237dc`（`optimizer(stage-1): add unsafe contract frontend`）。
- `cargo test --locked --test frontend contract_ -- --nocapture`：21 passed，0 failed，
  0 ignored。
- `cargo test --locked --test frontend lexer_ -- --nocapture`：2 passed，0 failed，
  0 ignored。
- `cargo test --locked --test frontend parser_ -- --nocapture`：6 passed，0 failed，
  0 ignored。
- `cargo test --locked --test frontend checker_ -- --nocapture`：14 passed，0 failed，
  0 ignored。
- `cargo test --locked --test ir mir_ -- --nocapture`：23 passed，0 failed，0 ignored。
- `cargo fmt --check` 与 `git diff --check`：exit 0。
- 加严回归 `cargo test --locked`：248 passed，0 failed，0 ignored；包含既有 safe
  frontend、MIR、C、WASM、CLI、optimizer 与 repository contract suites。
- 加严静态检查（配置仓库固定 LLVM/Clang 资产）
  `cargo clippy --locked --all-targets --all-features -- -D warnings`：exit 0。
- 结构复核确认：checked contract 保存确定排序的任意精度 affine coefficients、完整
  slice-parameter effect lattice、noalias/aligned/multiple_of；非法 unsafe/contract main
  不形成 entry；unsafe block 在 MIR 中无运行时 marker；backend、runtime、CLI 和
  optimizer 行为未改变。
