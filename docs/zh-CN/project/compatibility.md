# CalcKernel 0.10 兼容性策略

[English](../../project/compatibility.md)

本文档是 `0.10.x` 的规范性 compatibility authority。

Patch release 保持 0.10.0 已接受 source 及 observable semantics、stable diagnostic ID/category、
documented CLI name/flag/default、stdout/stderr class 与 success/failure behavior、textual MIR、
public C/WASM/Native C ABI shape、checked first-error order、runtime diagnostic byte/exit status，
以及六个 release archive name 与 checksum sidecar。

Patch release 可以拒绝过去误接受的非法输入、改善 diagnostic prose/caret、添加 opt-in API、
修复 codegen，并在所有承诺语义不变时优化。Private Rust module、algorithm、test、cache content/
eviction、benchmark measurement 与 undocumented internal IR 不属于 public contract。

## 从 0.9.0 迁移

- `build` 不再调用 Clang，改用进程内固定 LLVM/LLD；默认仍为 dynamic library。
- `--kind executable|dynamic|static|object` 新增 Native artifact；library form 共用唯一
  generated-header Native C ABI。
- 新增 `run`、无参数 internal `main` 与七个 Native print builtin。`main` 和所有 print 名称
  已 reserved，冲突声明必须重命名。
- `build-llvm` 保留为 deprecated dynamic/object alias，每次调用输出一次 warning。
- Native 接受 checked overflow/bounds，使用已有 C status meaning。
- 旧 standalone textual LLVM export shape 已退出；Native library 使用 Native C ABI，
  `emit-llvm` 仅为 host-only inspection output。
- Native build 不再留下 `.c`/`.ll` intermediate；`emit-c` 仍为 source-only。
- C 与 WebAssembly 不增加 runtime print，artifact root 可达的 print 会被拒绝。

不涉及以上变化的 0.9.0 source 保持 source semantics。Compatibility fixture 覆盖每项有意变化。
未来 `1.0.0` 才开始长期 stability commitment；0.10 不宣称 1.0 compatibility。
