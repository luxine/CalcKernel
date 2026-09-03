# 阶段 15 验收：Predicated-Update 物化与 LLVM

## 本地必须通过

- [ ] `cargo test --locked --test optimizer predicated_update_should_ -- --nocapture`
- [ ] `cargo test --locked --test optimizer predicated_update_checker_ -- --nocapture`
- [ ] `cargo test --all-features --locked --test native predicated_update_llvm_ -- --nocapture`
- [ ] `cargo test --all-features --locked --test native predicated_update_differential_ -- --nocapture`
- [ ] `cargo test --locked --test optimizer -- --nocapture`

## 结构与语义

- [ ] vector body 为 strict compare + select(candidate,old) + 单一 unmasked
  store；same-place old load、Memory SSA 和 post-state digest 一致。
- [ ] runtime guard 与 scalar epilogue 保留；固定宽度 VF/UF 和 minimum 正确。
- [ ] 独立 checker 拒绝全部 compare/select/store/memory/guard/digest 变异。
- [ ] LLVM IR 无 fast flag、masked store 或额外 store，module verify 成功。
- [ ] O0/tuned 在 strict-f64 固定与 adversarial 输入上 bitwise equal；checked
  proof 完整时相等，缺 proof 时不改写。

## 回归

- [ ] pure diamond、reduction、ordinary Loop SIMD、SLP 与 vector transaction
  suite 无回归。
- [ ] C/WASM backend、KIR 3、Native ABI 1 与 Runtime ABI 2 不变。

## 完成证据

KIR/LLVM 摘要、mutation 表、differential digest 与命令输出写入
`target/acceptance/v0.14/stage-15/`。
