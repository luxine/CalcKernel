# 阶段 01 验收：profile identity、格式、merge/inspect 与 CLI

## 必须通过

1. `cargo test --locked --test profile -- --nocapture`
2. `cargo test --locked --test cli pgo_cli_ -- --nocapture`
3. `cargo test --locked --test contracts docs_ -- --nocapture`
4. `cargo test --locked --lib profile:: -- --nocapture`
5. `cargo fmt --check`
6. `cargo clippy --all-targets --locked -- -D warnings`
7. `git diff --check`

每个 filter 必须实际运行非零测试；命令输出及 test count 写入 stage-01 evidence。

## 结构断言

- `CKPART01`/`CKPROF01` parser 只有 checked cursor/length 路径，未检查的文件字段不能用于分配、
  索引或乘加；未知 tag、非 canonical order、duplicate/trailing/bad digest 在公开 record 前失败。
- full identity hex 恰为 canonical identity bytes 的 64 字符 lowercase SHA-256；topology 与物理
  artifact kind 不混淆，所有固定阈值/resource limit 都进入 identity/contract digest。
- merge 不接受 final profile、symlink、递归子目录或 duplicate run；同 shard 集的输出 byte-identical。
- `pgo inspect` 不改变文件；普通命令默认行为和 help 仍保持 0.12 兼容，PGO 非法组合不产生输出。

## 完成证据

记录实现 SHA、Rust identity、golden fixture digest、mutation 类别与命令结果。阶段 01 不得声称
profile generation、use 或 Native multiversion 已实现。
