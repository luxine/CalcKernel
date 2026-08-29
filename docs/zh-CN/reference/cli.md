# `ckc` 0.11 CLI 参考

[English](../../reference/cli.md)

本文档定义原生 `ckc` 命令面。成功返回 0；usage、source、filesystem、unsupported-mode、
toolchain、backend 与 runtime failure 返回非零。错误与 diagnostic 写 stderr；成功状态和请求的
text output 写 stdout，另有说明时除外。

## 命令

| 命令 | 结果 |
| --- | --- |
| `ckc check <file>` | Parse 与 type-check，不生成 artifact。 |
| `ckc emit-mir <file>` | 输出 deterministic MIR。 |
| `ckc emit-kir <file>` | 输出 deterministic verified internal KIR。 |
| `ckc emit-c <file> --out <file.c>` | 只生成 C source 与 header。 |
| `ckc emit-wat <file>` / `emit-wasm` | 生成 textual/binary WebAssembly。 |
| `ckc emit-llvm <file>` | 生成 host triple 的 verified LLVM IR。 |
| `ckc build <file> --out <path>` | 进程内 Native build。 |
| `ckc build-llvm <file> --out <path>` | Native dynamic/object build 的 deprecated alias。 |
| `ckc run <file>` | 在隔离 child 中编译并执行 `main`。 |
| `ckc cache clean` | 仅删除解析出的 CK native cache。 |
| `ckc licenses` | 输出内嵌 third-party notice。 |
| `ckc --version --verbose` | 输出 compiler、ABI、LLVM、target、codegen 与 ORC identity。 |

`build` 接受 `--kind executable|dynamic|static|object`，省略时为 `dynamic`。Object、static、
dynamic 产物带 sibling Native C ABI header；Windows dynamic 还带 import library；executable
不带 header。Object suffix 为 `.o`/`.obj`，static 为 `.a`/`.lib`，dynamic 为
`.so`/`.dylib`/`.dll`。整个 output set 在替换目标前完成 staging；pre-commit failure
不改变任何 destination，multi-file commit failure 恢复 backup 或报告未恢复路径。

Compiler 在进程内使用 LLVM 22.1.8 与 LLD。产品命令不发现或启动外部 Clang、linker 或
archiver，Native build 不留下 `.c` 或 `.ll` intermediate。`emit-c` 永不编译或链接输出。

## 选项与默认值

- `--out`/`-o` 选择输出，`--header` 选择 C header。
- `--overflow` 与 `--bounds` 接受 `unchecked|checked`，默认 unchecked。
- `--opt-level 0|1|2|3` 和 `-O0`–`-O3` 控制 KIR/LLVM；执行命令默认 O3，inspection 默认 O0。
- `--cpu baseline|native` 用于 build；baseline 为 portable default，run 使用 host CPU。
- `--target <host-triple>` 只接受规范化后等于当前 host triple 的目标；不支持 cross compile。
- `--no-cache` 令 run 绕过 persistent cache 的读写。
- `--print-facts`、`--print-effect-summaries`、`--explain-optimization` 输出
  deterministic verified KIR evidence。
- `--sanitize-contracts` 只用于 `run` 与 `build --kind executable`，在每个 unsafe entry
  进行调试检查；它不是普通优化 mode。

从源码构建 `ckc` 时，`CKC_LLVM_PREFIX` 指向固定 LLVM 安装；release binary 运行时不依赖它。

## Backend 与 effect matrix

| Surface | Overflow checked | Bounds checked | 可达 Native print |
| --- | --- | --- | --- |
| Native `run` / executable | 接受 | 接受 | 接受 |
| Native dynamic/static/object | 接受 | 接受 | export root 拒绝 |
| C `emit-c` | 接受 | 接受 | artifact root 拒绝 |
| WASM | 拒绝 | 拒绝 | export root 拒绝 |
| `emit-kir` | 构造 KIR 前选择 | 构造 KIR 前选择 | inspection root 为 export 与 `main` 并集 |

Unsupported combination 在 artifact 创建前拒绝。`emit-llvm` 使用 Native lowering，接受四种
checked combination，是 inspection artifact 而不是独立 public ABI。

Run cache key 包含 exact source、compiler/Native ABI/runtime ABI/LLVM identity、target、完整
CPU feature、optimization、mode 与全部 object-affecting option。Entry 含 versioned manifest、
object 与 SHA-256 integrity digest。`--no-cache` 绕过读写。Corruption、unsafe ownership/
permission、symlink replacement 或 unparseable object 视为 miss。Same-user cache 仍属于 user
trust boundary，不是 security sandbox。

0.11 cache/codegen identity 使用 KIR v1，并包含 consumer、mode、contract sanitizer 等；
0.10 object 不会被复用。

Root 为 Linux 的 `$XDG_CACHE_HOME/ckc` 或 `$HOME/.cache/ckc`、macOS 的
`$HOME/Library/Caches/ckc`、Windows 的 `%LOCALAPPDATA%\CalcKernel\cache`。缺少 required base
时本次 run 禁用 cache。Write 使用 owner-only same-filesystem staging 与 atomic rename；默认
soft limit 为 1 GiB，采用 best-effort LRU eviction。Native checked failure 使用 240–243，
stdout failure 为 244，abnormal child termination 为 245。
Contract sanitizer failure 为 246，精确输出 `CKR0007: unsafe contract violation`。
