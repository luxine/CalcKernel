# CalcKernel V0.9 兼容性策略

[English](../../project/compatibility.md)

本文档是 `0.9.x` release line 的规范性兼容权威。

`0.9.x` patch release 对以下内容保持向后兼容：0.9.0 接受的 CK source program
及可观察 semantics；stable diagnostic ID 与触发类别；command、flag/alias/default、
argument precedence、stdout/stderr 类别、成功/失败退出与 artifact naming；textual
MIR syntax、deterministic printing 与 instruction meaning；已记录的 C/WASM/LLVM、
checked-mode、slice、void 与 exported function ABI；六个 native target/archive 名称
及 checksum sidecar。

Patch release 可以修复对 invalid input 的误接受、改善 diagnostic prose/caret、
增加非破坏文档、在不改变可观察 semantics 时优化，并增加默认行为保持兼容的 API
或 flag。

Internal Rust module/file path、private item、test organization、algorithm、benchmark
数值、build cache 与未记录 backend internal 不属于兼容承诺。仓库 test/embedding
明确使用的公共 Rust re-export 在 `0.9.x` 内保持稳定。

`0.10.0` 只有在明确记录并提供 migration guidance 时才可改变 language、diagnostic、
CLI、MIR 或 ABI。未来 `1.0.0` 才开始长期兼容承诺；V0.9 不宣称 1.0 stability。
