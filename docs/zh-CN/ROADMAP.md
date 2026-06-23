# IntKernel 路线图

[English](../ROADMAP.md)

这份路线图记录 V0 之后可能的工作。它不承诺每一项都会按这个顺序发布。

## V0 Stable

- 保持语言刻意小而清晰。
- 稳定 lexer、parser、type checker、diagnostics、C backend、CLI 和测试。
- 维护 generated C/header golden snapshots。
- 在 clang 可用时保持 strict clang e2e 覆盖。

## C ABI Hardening

- 记录平台 ABI 假设。
- 增加更多 ABI-focused golden tests。
- 为更多示例增加 C harness。
- 在可行处验证 struct layout 预期。
- 改进宿主语言 binding 指南。

## Python 和 Node 示例

- 增加最小 Python loading 示例。
- 增加最小 Node.js loading 示例。
- 记录 64-bit integer 处理方式，尤其是 JS `BigInt`。
- 在 IntKernel 侧保持 examples runtime-free。

## Benchmarking

- 为生成的 kernel 增加可重复 microbenchmark。
- 对比不同 optimization level 下的 generated C build。
- 记录 benchmark input 和 host compiler version。

## Phase 10 Checked Arithmetic

Phase 10 checked arithmetic 已覆盖当前 V0 语言面。

- `--overflow unchecked` 仍是默认。
- `--overflow checked` 生成带 `IK_Status` 的 checked C/header output。
- Checked mode 报告 add、subtract、multiply、divide、modulo 和 unary minus
  arithmetic failure。
- Checked mode 在 IntKernel function call 间传播错误。
- Checked mode 保持 `&&` 和 `||` short-circuit behavior。
- Checked mode 支持 V0 control flow、pointer indexing 和 struct field access。
- Checked mode 不添加 bounds check 或用户 pointer validation。
- Python、Node.js 和 benchmark examples 包含 checked-mode entry points。

未来 checked arithmetic 工作：

- 为不支持 Clang/GCC `__builtin_*_overflow` 的编译器增加 portable overflow fallback。
- 如果项目支持无 clang-compatible builtins 的原生 MSVC 编译，增加 MSVC-specific
  checked arithmetic lowering。
- 除非未来 major version 明确改变契约，否则保持 unchecked overflow 为默认。

## Phase 11 Typed IR / MIR

Phase 11 Typed IR / MIR 已覆盖当前 V0 语言面。MIR v1 是 typed、three-address、
basic-block based，但不是 SSA。

- `docs/MIR.md` 记录 MIR v1。
- MIR types、printer 和 validator 已实现。
- Typed AST 降低到 MIR，且不改变语言语义。
- MIR-to-C unchecked code generation 已实现。
- MIR-to-C checked code generation 已实现。
- `ikc emit-mir` 暴露稳定 MIR text，用于 compiler debugging。
- 默认 `emit-c` 和 `build` pipeline 现在使用 MIR。
- 旧 AST C backend 在迁移期间保留为 legacy/internal fallback。

MIR v1 明确不包含 optimizer、constant folding、dead code elimination、register
allocation、bounds checks、runtime support 或新语言功能。

## Phase 12 WASM Backend

Phase 12 WASM backend 已覆盖当前 MIR 支持的 V0 语言面。

- `docs/WASM_ABI.md` 记录 WASM ABI。
- 目标是 `wasm32`。
- `ptr<T>` 映射为 `i32` linear-memory offset。
- module memory 以 `(memory (export "memory") 1)` 导出。
- Struct layout 是确定性的，且不依赖宿主 C 编译器。
- MIR-to-WAT code generation 有稳定 snapshot。
- `ikc emit-wat` 生成稳定 WAT text。
- `ikc emit-wasm` 通过捆绑的 `wabt` npm package assembly WAT。
- Node.js 和 browser WebAssembly 示例使用 `DataView` 和 `BigInt`。
- `pricing.ik` 有 WASM e2e 覆盖。
- Benchmark harness 包含 unchecked WASM benchmark。

Phase 12 v1 仍只支持 unchecked。WASM 的 `--overflow checked` 必须报告清晰的
unsupported-mode error，直到 checked WASM lowering 完成设计。

Phase 12 不增加 WASI、imports、allocator、runtime support、strings、bounds
checks、`slice<T>`、SIMD、threads、GC 或 exceptions。

未来 WASM 工作：

- checked WASM arithmetic
- 可选的 simple WASM allocator
- 更丰富的宿主语言示例
- 如果未来 use case 需要 imports 或 host services，增加 WASI integration
- 如果语言引入携带长度的 pointer type，再支持 `slice<T>` / bounds check

## Phase 13 LLVM Backend

Phase 13 以从 MIR 生成 textual LLVM IR 作为 LLVM backend 起点。

- 在 `docs/LLVM_BACKEND.md` 记录 LLVM backend 设计。
- 为 scalar subset 增加 MIR-to-LLVM IR text generation。
- 增加 `emit-llvm`，生成稳定 `.ll` output。
- 通过 clang 增加可选 `build-llvm` 或 `build --backend llvm`。
- 在 LLVM e2e 行为被证明前，保持 C backend 为 reference backend。
- LLVM v1 仅支持 unchecked。
- 在 checked LLVM lowering 完成设计前，LLVM 遇到 `--overflow checked` 应拒绝。
- 在认为 backend 可发布前，覆盖 scalar、control flow、function call、
  ptr/index/field/store 和 `pricing` e2e。

Phase 13 v1 不增加 LLVM C++ API、bitcode writer、JIT、optimizer、debug info、
runtime support、allocator、bounds check、`slice<T>`、strings、IO 或 modules。

未来 LLVM 工作：

- checked LLVM arithmetic
- direct SSA lowering
- optional optimizer pass pipeline
- target data layout hardening
- object/native-library build hardening

## Future Optimizer

- 只有在 MIR 至少经历一个 release 后仍稳定，才考虑单独 optimization phase。
- 候选 pass 包括 constant folding、dead code elimination、common subexpression
  elimination 和 range analysis。
- 任何 optimizer 都必须保持 checked/unchecked semantics 和 generated ABI。

## Future `slice<T>` / Bounds Checks

- Raw `ptr<T>` 保持 unchecked。
- Bounds check 应等待携带长度的类型，例如未来 `slice<T>` 或显式 pointer-plus-length
  metadata。
- 引入 bounds-safe lowering 前，先记录 ownership、nullability 和 aliasing 规则。
