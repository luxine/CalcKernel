# 阶段 02 任务：Manifest、路径、环境与 immutable snapshot

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

实现 closed schema-1 TOML workload 输入及其完整身份，安全捕获 runner、declared inputs 与 effective
environment，并为每次调用生成 fresh flat input files 和 exact `CKTIMAP1` map。

## 仓库落点与接口

- 新建 `src/tune/{manifest.rs,path.rs,snapshot.rs,input_map.rs}`。
- `TuneManifest::parse(bytes, manifest_path)` 返回规范化但仍保留 manifest-order inputs 的闭合模型；
  `capture_workload(&TuneManifest)` 返回 `CapturedWorkload`，只暴露 snapshot handle/bytes、公开 value
  length/digest 和 manifest identity，不公开 secret value 给 inspection。
- `stage_invocation_inputs(&CapturedWorkload, run_root)` 使用 create-new/no-follow 建立 exact basename，
  rehash 后返回 read-only `CK_TUNE_INPUT_MAP`；`encode_input_map`/`decode_input_map` 共用 golden vectors。
- 扩展 `tests/tune/{manifest.rs,snapshot.rs,input_map.rs}`，平台 path/alias 测试放入同模块的 cfg 分支；
  fixture 放 `tests/fixtures/tune/workload/`。

## TDD 顺序

1. 写 closed TOML RED：unknown/duplicate/missing/type/range、1..16 cases、search+validation、id/seed
   唯一、argv/env/input aggregate bound、non-NFC/NUL 全部 fail；实现专用 schema parser，不能接受隐式 coercion。
2. 写 path RED：relative runner 以 canonical manifest parent 为基准、input root 以打开的 manifest 目录
   handle 为锚；absolute/traversal/symlink/reparse/nonregular/duplicate handle identity 均拒绝。Windows 另测
   authoritative long/short leaf 与 ASCII-fold collision。
3. 写 environment RED：Unix 空环境加 allowlist；Windows 仅自动加入 canonical `SystemRoot`/`WINDIR`，
   union 最多 16；缺失、case collision、NUL/size/unrepresentable 值拒绝，公开 record 仅 name/bytes/digest。
4. 写 snapshot race RED：捕获后替换原 runner/input 不改变 session bytes；runner 必须是 host ELF/Mach-O/
   PE executable 且不是 script；复制后 digest/length/format 再验证。
5. 写 `CKTIMAP1` RED：8-byte magic、U32_BE count、manifest-order Text/Text/U64/D32、exact EOF；fresh
   invocation 使用 `00000000-<digest>.bin` 形式，readonly、rehash、无 exact/ASCII-fold/short-name alias。
6. 运行 `cargo test --test tune manifest_ -- --nocapture`、`snapshot_`、`input_map_` 和 `cargo test --locked`；
   保存 RED/GREEN、平台能力和 fixture digest。

## 实现边界

- operational absolute path 与 secret value 永不进入 decision canonical identity。
- schema 1 不提供 shell、cwd、额外 protocol env、glob、directory recursion、script runner 或 sandbox 承诺。
- snapshot 完成前 wall budget 不开始；阶段 05 才执行 runner。

