# 阶段 02 验收：workload identity 与 immutable snapshot

## 必须通过

- [ ] `cargo test --test tune manifest_ -- --nocapture`
- [ ] `cargo test --test tune snapshot_ -- --nocapture`
- [ ] `cargo test --test tune input_map_ -- --nocapture`
- [ ] `cargo test --locked`

## 结构断言

- [ ] Manifest 是 closed schema；所有 logical field、runner/input bytes 和 effective environment identity 唯一进入 digest。
- [ ] runner/path/input 使用 no-follow handle walk，relative runner base 不依赖 caller cwd。
- [ ] capture 后替换、symlink swap、duplicate handle、Windows short-name collision 均不能改变或别名 session 输入。
- [ ] public decision 不含环境值；runner process state 仍收到 exact private accepted bytes。
- [ ] CKTIMAP1 framing、basename、fresh copy、readonly、rehash 与 exact EOF 由 golden/negative tests 固定。

## 完成证据

把被测 SHA、平台 path capability、snapshot race、map digest 和测试计数写入 `target/acceptance/v0.14/stage-02/`。

