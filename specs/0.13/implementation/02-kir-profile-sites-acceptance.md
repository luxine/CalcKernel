# 阶段 02 验收：KIR 3 profile site、effect 与 mapping

## 必须通过

1. `cargo test --locked --test pgo_kir -- --nocapture`
2. `cargo test --locked --test ir profile_ -- --nocapture`
3. `cargo test --locked --test optimizer profile_ -- --nocapture`
4. `cargo test --locked --test optimizer transaction_ -- --nocapture`
5. `cargo test --locked --test contracts kir_ -- --nocapture`
6. `cargo test --locked`
7. `cargo fmt --check`
8. `cargo clippy --all-targets --locked -- -D warnings`
9. `git diff --check`

所有 filter 必须非零，default-feature 全套回归必须真实通过。

## 结构断言

- print/schema 明确为 KIR 3；site ID 只是索引，full descriptor/table digest 才是 collision authority。
- generate/use 从同一 canonical pre-profile KIR 重建同一 table；off 不含 profile op，use 不写 counter。
- profile effect 与 CK memory/effect 正交但顺序受约束；任何 DCE/clone/motion/伪造 transfer mutation
  均由独立 verifier 拒绝。
- count/annotation 从未进入 fact/proof arena，不能改变 checked failure、strict f64 或 effect order。

## 完成证据

记录实现 SHA、KIR 3 golden digest、topology determinism/mutation 结果和全套 test count。不得用
阶段 02 的内存 fixture 代签真实 runtime collection。
