# IntKernel 发布检查清单

[English](../RELEASE_CHECKLIST.md)

发布或打 V0 tag 前使用这份检查清单。

## 必需验证

- 运行 `pnpm test`。
- 运行 `pnpm typecheck`。
- 运行 `pnpm build`。
- 运行 `pnpm ikc --help`，或等价的已安装 `ikc --help` 命令，并 review 输出。
- 运行 `pnpm ikc check examples/pricing.ik`。
- 运行 `pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h`。
- Review 生成的 `pricing.c` 和 `pricing.h`。
- 如果 clang 可用，运行 e2e clang compile 和 harness test。
- Review generated C/header snapshot diff。
- Review V0 language、compiler architecture、ABI 和 roadmap 文档准确性。
- 保持文档双语：英文为默认入口，新增或修改文档时同步更新中文译本。

## MIR / Default Pipeline 验证

- 运行 `pnpm ikc emit-mir examples/pricing.ik --out build/pricing.mir`。
- Review MIR 输出格式稳定，且没有绝对路径、时间戳或随机 ID。
- 确认 MIR validator tests 在 `pnpm test` 中通过。
- 确认 MIR lowering 和 MIR C emitter tests 在 `pnpm test` 中通过。
- 只要 legacy AST backend 仍在仓库中，就确认 AST vs MIR regression tests 通过。
- 确认默认 `emit-c` 和 `build` 输出由 MIR pipeline 生成。

## Checked Mode 验证

- 运行普通 unchecked test suite。
- 运行 checked arithmetic tests，并在 clang 可用时确认 checked e2e cases 通过。
- Review checked generated C/header snapshots。
- 确认 checked `emit-c` 和 `build` 命令使用 `--overflow checked`。
- 确认 checked dynamic library build 仍使用 strict clang flags。
- 当本地平台已有 generated checked dynamic library 时，手动运行 checked Python
  `ctypes` 示例。
- 当本地平台和 FFI dependency 可用时，手动运行 checked Node.js FFI 示例。
- Review `docs/CHECKED_ARITHMETIC.md`、`docs/ABI.md` 和 README checked-mode 章节，
  确认 ABI 和 safety boundary 准确。

## WASM Backend 验证

- 确认 `emit-wat` tests 在 `pnpm test` 中通过。
- 确认 `emit-wasm` tests 在 `pnpm test` 中通过。
- 确认 WASM scalar e2e tests 通过。
- 确认 WASM control-flow、function-call、short-circuit 和 memory e2e tests 通过。
- 确认 `examples/pricing.ik` WASM e2e tests 通过。
- 确认 `emit-wat --overflow checked` 和 `emit-wasm --overflow checked` 会以文档中的
  unsupported-mode message 失败。
- Review WAT snapshot diff。
- 当 `build/pricing.wasm` 可用时，手动运行 Node.js WASM 示例。
- 通过本地 HTTP server 手动运行 browser WASM 示例。
- 生成 `build/pricing.wasm` 后，手动运行
  `node bench/wasm_pricing_benchmark.mjs`。
- Review `docs/WASM_ABI.md`，确认 type mapping、memory layout、examples 和
  safety boundary 准确。

## 可选发布检查

- 如果计划发布到 npm，运行 `npm pack --dry-run`。
- 确认 package 包含已构建 CLI entrypoint。
- 确认 intended-for-users 的 examples 和 docs 被包含。
- 确认没有误包含本地 build artifact 或临时文件。

## Release Notes

发布前总结：

- V0 支持的语言特性
- 已知限制
- ABI compatibility notes
- diagnostics 和 CLI 变更
- 任何有意改变的 generated C/header output
