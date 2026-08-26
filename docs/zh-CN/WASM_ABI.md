# CK / CalcKernel WASM ABI

[English](../WASM_ABI.md)

本文定义 `native ckc` 生成的 WASM ABI。

## 生成

```sh
ckc emit-wat examples/wasm_scalar.ck --out build/scalar.wat
ckc emit-wasm examples/wasm_scalar.ck --out build/scalar.wasm
```

Rust backend 生成 WAT，并通过 Rust `wat` crate assembly WASM。不需要外部
`wat2wasm` executable。生成的 `.wasm` 文件需要 WebAssembly runtime 才能运行。

## 范围

当前 WASM backend 支持：

- `i32`、`u32`、`i64`、`u64`、`bool`、`f64`
- `ptr<T>` 作为 linear-memory byte offset
- exported functions
- non-exported internal functions
- exported linear memory
- deterministic CK struct layout
- unchecked arithmetic
- scalar control flow 和 function call
- `ptr<T>` load/store、index access、struct field access

当前不提供：

- checked overflow lowering
- WASI imports
- heap allocation
- string
- bounds check
- `slice<T>`
- runtime library
- SIMD、threads、GC、exceptions

## Type Mapping

| CK type | WASM value type |
| --- | --- |
| `i32` | `i32` |
| `u32` | `i32` |
| `bool` | `i32` |
| `i64` | `i64` |
| `u64` | `i64` |
| `f64` | `f64` |
| `ptr<T>` | `i32` byte offset |

除法、取余和比较通过 instruction choice 区分 signedness。`bool` 用 `0` 表示 false，
非零表示 true。

## Function ABI

exported CK function 会按源码名称导出：

```ck
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

生成的函数使用 WASM `i64` 参数和返回值。CK `f64` 使用 WASM `f64`，pointer 使用
`i32` byte offset。

CK void function 没有 `(result ...)`（no `(result ...)`）。Valueless return 不留下 stack value；
targetless void call 生成 `call`，后面没有 `local.set`。此规则同时适用于
single-block function、dispatcher fallback 和 structured-while path。

## Memory ABI

module 导出一个 linear memory：

```wat
(memory (export "memory") 1)
```

CK 使用 caller-owned memory。host 负责：

- 选择互不重叠的 input/output offset
- 把 input struct 和 array 写入 memory
- 将 `ptr<T>` 作为 byte offset 传入
- 调用后读取 output buffer
- 在 layout 需要更多 page 时增长 memory

compiler 不分配、不验证、不增长、不释放 host buffer。

## Struct Layout

WASM 使用 deterministic CK layout，不依赖 host C compiler。

primitive layout：

| Type | Size | Alignment |
| --- | ---: | ---: |
| `i32` | 4 | 4 |
| `u32` | 4 | 4 |
| `bool` | 4 | 4 |
| `ptr<T>` | 4 | 4 |
| `i64` | 8 | 8 |
| `u64` | 8 | 8 |
| `f64` | 8 | 8 |

struct field 按声明顺序布局。每个 field offset 按该 field alignment 对齐，最终 size
补齐到 struct alignment。

## Host Interop

任何有 WebAssembly runtime 的 host 都可以实例化 module。homogeneous buffer 使用
typed-array view，mixed-width struct 或 byte-exact check 使用 `DataView`。
WebAssembly memory 是 little-endian。

对于 `ptr<f64>` buffer：

- pointer argument 是 `i32` byte offset
- `f64` size 是 8 bytes
- `ptr<f64>[i]` 地址是 `base + i * 8`
- `Float64Array` index 是 `byteOffset / 8`
- `memory.grow` 后需要重新创建 typed-array view

## Checked Overflow

WASM backend 目前只支持 unchecked。如果需要 checked arithmetic，使用 C backend：

```sh
ckc emit-c input.ck --overflow checked --out build/input.c --header build/input.h
ckc build input.ck --overflow checked --out build/input
```

## Slice ABI

Logical `slice<T>` parameter 会展平为两个 collision-safe `i32` parameter：linear
memory data address，随后是 `u32` length。Local、temporary、call 与 dispatcher
return 都携带同一对值。Internal slice return 使用 data/length 顺序的 WASM
multi-value `(result i32 i32)`。

Stored descriptor 占 8 bytes、alignment 4：address 位于 offset 0，length 位于
offset 4。Slice index 与 sub-slice 使用 deterministic element size。内存仍由
caller owned，descriptor 可以 alias。WASM 只支持 `--bounds unchecked`；
`--bounds checked` 会在 emission 前被拒绝，不会隐式添加 guard 或 trap。
