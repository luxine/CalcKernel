# IntKernel 路线图

[English](../ROADMAP.md)

这份路线图记录 V0 之后可能的工作。它不承诺每一项都会按这个顺序发布。

## V0 Stable

- 保持语言刻意小而清晰。
- 稳定 lexer、parser、type checker、diagnostics、MIR、C backend、WASM backend、
  LLVM backend、CLI 和测试。
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

Phase 11 时，MIR v1 明确不包含 optimizer、register allocation、bounds checks、
runtime support 或新语言功能。Phase 14 后续增加了保守 MIR optimizer，同时保持这些
safety boundary。

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

Phase 13 LLVM backend 已覆盖当前 MIR 支持的 unchecked V0 语言面。

- `docs/LLVM_BACKEND.md` 记录 LLVM backend contract。
- MIR-to-LLVM IR text generation 已实现。
- `ikc emit-llvm` 生成稳定 `.ll` output。
- `ikc build-llvm` 可以通过 clang 构建 dynamic library。
- `ikc build-llvm --kind object` 可以通过 clang 生成 object file。
- LLVM IR snapshots 覆盖 scalar、control flow、function call、
  ptr/index/field/store、short-circuit 和 `pricing`。
- LLVM clang e2e tests 覆盖 scalar、bool ABI、control flow、function call、
  short-circuit、memory access 和 `pricing`。
- C/WASM/LLVM backend regression comparison tests 覆盖 scalar、control flow、
  function call、short-circuit、memory 和 pricing fixtures。
- LLVM v1 仍只支持 unchecked。
- 在 checked LLVM lowering 完成设计前，LLVM 会拒绝 `--overflow checked`。

Phase 13 v1 不增加 LLVM C++ API、bitcode writer、JIT、LLVM-specific optimizer
pipeline、debug info、runtime support、allocator、bounds check、`slice<T>`、
strings、IO 或 modules。

未来 LLVM 工作：

- checked LLVM arithmetic
- 更广泛的 direct SSA LLVM lowering
- target data layout hardening
- object/static library improvements
- debug info
- 如果未来产品场景需要，再考虑 JIT
- 语言具备携带长度的 pointer type 后，再支持 `slice<T>` / bounds check

## Phase 14 Optimization and Performance

Phase 14 optimization 和 performance 工作已经覆盖 v0.4.0。

- `ikc` 支持 `--opt-level 0`、`--opt-level 1`、`--opt-level 2`、`--opt-level 3`，
  以及 `-O0` 到 `-O3` alias。
- `-O0` 仍是保守默认值，并让输出最接近 lowered MIR。
- `-O1`、`-O2` 和 `-O3` 启用文档化的保守 MIR pass 分层。
- Checked C 保留业务 overflow 和 division check；只有证明安全的 loop induction
  increment 可以使用 checked C hot-path optimization。
- WASM 和 LLVM 仍然是 unchecked-only，并拒绝 `--overflow checked`。
- Performance suite 支持 quick/full run、private baseline、compare mode 和显式
  regression guard。
- Optimization 必须保持 checked/unchecked semantics 和 generated ABI，且不能对
  `examples/pricing.ik` 做特判。

未来 optimization 工作：

- 更广泛的 WASM structured control-flow lowering
- 更多 scalar control flow 的 direct SSA LLVM lowering
- target data layout hardening
- 默认 build 之外的可选 CPU-native/LTO 实验
- 更广泛的 f64 optimization 需要后续 Phase 先明确 strict-safe floating point
  optimization rules

Numeric roadmap lock：

- IK / IntKernel 的 floating point 保持 f64-only。
- 不规划 `f32`。
- Phase 20 从 exact `i32_to_f64` 和 `u32_to_f64` builtin 开始支持 explicit
  numeric cast。
- `i64_to_f64`、`u64_to_f64`、f64-to-int cast、overloaded cast 和 cast expression
  syntax 仍属于未来设计工作。
- fast-math 和 SIMD 不属于当前 numeric roadmap。

## Future `slice<T>` / Bounds Checks

- Raw `ptr<T>` 保持 unchecked。
- Bounds check 应等待携带长度的类型，例如未来 `slice<T>` 或显式 pointer-plus-length
  metadata。
- 引入 bounds-safe lowering 前，先记录 ownership、nullability 和 aliasing 规则。
