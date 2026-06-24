# IntKernel v0.4.0 发布检查清单

[English](../RELEASE_CHECKLIST.md)

发布或打 `v0.4.0` tag 前使用这份检查清单。它是人工验收 gate；不要把 benchmark
结果当作语言语义，也不要当作跨机器性能真相。

## Release Scope

- 确认语言和项目名称使用 IK / IntKernel。
- 确认 CLI 示例使用 `ikc`。
- 确认源码示例使用 `.ik`。
- 确认 `package.json` version 是 `0.4.0`。
- 确认计划使用的 git tag 是 `v0.4.0`。
- 确认 package metadata 只暴露 `ikc` bin entrypoint。
- 确认 v0.4.0 release notes 只宣传已经实现的 Phase 14 能力。
- 确认不宣传 floating point support：`f64` 在 v0.4.0 尚未实现，属于未来
  Phase 16。
- 不要宣传未支持的 f32、implicit int/float conversion、fast-math、SIMD、JIT、
  strings、IO、GC、runtime 或新 backend support。

## 必跑命令

- 运行 `pnpm test`。
- 运行 `pnpm typecheck`。
- 运行 `pnpm build`。
- 运行 `npm pack --dry-run`。
- 运行 `pnpm ikc --help`，或等价的已安装 `ikc --help`，并 review 输出。
- 运行 `pnpm ikc check examples/pricing.ik`。
- 运行 `pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h`。
- 运行 `pnpm ikc emit-mir examples/pricing.ik --out build/pricing.mir`。

## Naming Consistency

- 确认 `tests/naming-consistency.test.ts` 已作为 `pnpm test` 的一部分通过。
- Review README、docs、examples、snapshots 和 CLI help 的标准命名。
- 确认 package metadata、命令示例和 release docs 使用 IK / IntKernel、`ikc`
  和 `.ik`。
- 确认没有引入 compatibility alias 或替代源码后缀。

## C Backend Regression

- 确认 C emitter snapshot tests 通过。
- 确认 C scalar expression 覆盖通过。
- 确认 C if/else 和 while 覆盖通过。
- 确认 C function-call 覆盖通过。
- 确认 C short-circuit `&&` / `||` 覆盖通过。
- 确认 C ptr/index、struct field，以及 `items[i].price` 这类 combined access
  覆盖通过。
- 确认 `examples/pricing.ik` C e2e 通过。
- 确认 unchecked mode 仍是默认值，并保持 unchecked ABI。
- 确认 checked C mode 的 scalar、control-flow、short-circuit、function-call 和
  `examples/pricing.ik` e2e 覆盖通过。
- Review checked generated C/header snapshots。

## WASM Backend Regression

- 确认 `emit-wat` tests 通过。
- 确认 `emit-wasm` tests 通过。
- 确认 WAT snapshots 通过。
- 确认 WASM scalar e2e 通过。
- 确认 WASM control-flow e2e 通过。
- 确认 WASM function-call e2e 通过。
- 确认 WASM short-circuit e2e 通过。
- 确认 WASM memory / ptr e2e 通过。
- 确认 WASM layout tests 通过。
- 确认 `examples/pricing.ik` WASM e2e 通过。
- 确认 `emit-wat --overflow checked` 和 `emit-wasm --overflow checked` 会以文档中的
  unsupported-mode message 失败。
- Review `docs/WASM_ABI.md`，确认 type mapping、memory layout、examples 和
  safety boundary 准确。

## LLVM Backend Regression

- 确认 `emit-llvm` CLI tests 通过。
- 确认 `build-llvm` clang command tests 通过。
- 确认 `build-llvm --kind object` tests 通过。
- 确认 LLVM IR snapshots 通过。
- 确认 LLVM scalar e2e 通过。
- 确认 LLVM control-flow e2e 通过。
- 确认 LLVM function-call e2e 通过。
- 确认 LLVM short-circuit e2e 通过。
- 确认 LLVM ptr/index/field/store e2e 通过。
- 确认 LLVM bool ABI e2e 通过。
- 确认 `examples/pricing.ik` LLVM e2e 通过。
- 确认 `emit-llvm --overflow checked` 和 `build-llvm --overflow checked` 会以文档中的
  unsupported-mode message 失败。
- Review `docs/LLVM_BACKEND.md`，确认 backend limits 和 release notes。

## Cross-Backend Baseline

- 确认 C/WASM/LLVM backend regression comparison tests 通过。
- 确认 shared comparison 覆盖 scalar、control flow、function calls、
  short-circuit、memory 和 `examples/pricing.ik`。
- 确认没有 backend output snapshot 发生非预期变化。
- 如果 snapshot 有变化，必须先解释真实预期行为变化，再接受更新。不要为了让测试通过
  而更新 snapshots。

## Optimization Correctness

- Review `docs/OPTIMIZATION.md`。
- 确认 optimization pipeline 默认是 `-O0`。
- 确认 `-O0`、`-O1`、`-O2` 和 `-O3` tests 都作为 `pnpm test` 的一部分通过。
- 确认 optimizer correctness 覆盖 constant folding、copy propagation、dead code
  elimination、CFG simplify、local CSE、address CSE、small-function inlining
  和 loop optimization。
- 确认 checked mode 保留 overflow 和 division checks，除文档化的已证明安全 loop
  induction increment 外不移除检查。
- 确认 short-circuit 语义在优化前后保持一致。
- 确认 `examples/pricing.ik` 优化前后结果一致。
- 确认没有为 `examples/pricing.ik` 添加 benchmark-specific special case。

## Benchmark Smoke

- Review `docs/PERFORMANCE.md`、`bench/README.md` 和 `bench/README.zh-CN.md`。
- 运行 `node --test bench/perf/tests/perf-core.test.mjs`。
- 可选手动运行 `node bench/perf/run.mjs --quick`，作为 benchmark smoke。
- `node bench/perf/run.mjs --full` 是机器时间允许时的可选 tag-time 检查。
- 不要把 benchmark threshold 加进普通 `pnpm test`。
- 不要把本机 benchmark 结果当作跨机器绝对 baseline。
- 如果保存 baseline，遵循现有策略：本机 baseline 放在被忽略的 `build/perf` 输出中，
  `bench/perf/baselines/example.summary.json` 只是格式示例，不是真实阈值文件。
- 不要提交机器本地 benchmark output、真实本机 baseline、cache 或临时文件。

## Docs And Examples Review

- Review README 和 README.zh-CN，确认 v0.4.0 能力描述准确。
- Review language、architecture、MIR、ABI、checked arithmetic、WASM、LLVM、
  optimization、performance 和 roadmap docs。
- 确认 docs 说明 IK / IntKernel 是纯计算 DSL，不是通用语言。
- 确认 docs 说明当前 v0.4.0 计算能力主要面向整数。
- 确认 docs 没有宣称 floating point、SIMD、JIT、runtime、IO、strings 或 GC 支持。
- Review `examples/` 下的示例，确认命令使用 `ikc` 和 `.ik`。
- 确认 `examples/pricing.ik` 仍是 release e2e fixture。
- 保持文档双语：英文为默认入口，修改文档时同步更新中文译本。

## Package Contents Review

- 确认 `npm pack --dry-run` 报告 package `intkernel@0.4.0`。
- 确认 package 包含 `dist/src`、docs、examples、bench files、README.md、
  README.zh-CN.md 和 package.json。
- 确认 package 包含 `ikc` bin mapping 指向的 built CLI entrypoint。
- 确认 package `exports`、`files` 和 scripts 与已发布的 IK / IntKernel surface
  一致。
- 确认没有误包含本地 build artifact、benchmark output、真实本机 baseline、cache、
  editor state 或临时日志。

## Tag Gate

- 确认上面的必跑命令和人工 review 都已完成。
- 确认 working tree changes 都是有意且已理解的。
- 确认 release notes 只总结已实现的 v0.4.0 能力：lexer、parser、type checker、
  MIR、MIR validation、保守 MIR optimization levels、C/WASM/LLVM backends、checked
  C integer arithmetic、backend regression 覆盖、`examples/pricing.ik` e2e 覆盖和
  手动 performance suite。
- 确认 known limitations 已列出：v0.4.0 没有 floating point、没有 implicit
  int/float conversion、没有 fast-math、没有 SIMD、没有 JIT、没有 IO、没有 strings、
  没有 GC、没有 runtime，也没有 checked WASM/LLVM arithmetic。
- 只有 checklist 完成后才创建 tag `v0.4.0`。
