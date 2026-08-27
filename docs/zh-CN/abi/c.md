# CalcKernel 0.10 C Source ABI

[English](../../abi/c.md)

本文档定义 `ckc emit-c` 生成的 C 与 header。该路径为 source-only，永不编译或链接；Native
`ckc build` 直接生成 LLVM object，并遵守 [Native C ABI](llvm.md)。

CK 定宽 integer、`f64`、`bool` 分别映射为对应 C fixed-width integer、`double`、`bool`；
`ptr<T>` 映射为 `T*`；struct 保持 field declaration order；unchecked void return 为 C
`void`。Header 提供 `CK_API` visibility 与 C++ `extern "C"` guard。Host 必须 include 该
header，不得猜测 target padding。

Pointer/`slice<T>` memory 为 caller-owned 且允许 alias。Stored slice descriptor 是 `T* data` 后接
`uint32_t len`；slice parameter 按 data,length flatten；exported slice return 非法。

Unchecked export 直接返回 source result。启用任一 checked mode 后，全 module 使用
`CK_Status`：non-void function 追加名为 `ck_return` 的 result pointer，void function 不追加；call 传播 first
non-OK status。`--bounds checked` 仅覆盖 slice index 与半开 sub-slice；raw pointer、
`slice(data, len)`、`.data` index、memory validity/alignment/lifetime 仍由 caller 负责。

C backend 不实现 runtime output。任一 C artifact root 可达的 print 会在输出前被拒绝。
Internal `main` 可作为普通 function lowering，但 `emit-c` 不创建 process entry。
