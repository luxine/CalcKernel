# CalcKernel 发布检查清单

[English](../RELEASE_CHECKLIST.md)

发布或打 release tag 前使用这份检查清单。它是人工验收 gate；不要把 benchmark
结果当作语言语义，也不要当作跨机器性能真相。

## Release Scope

- 确认语言和项目名称使用 CK / CalcKernel。
- 确认 CLI 示例使用 `ckc`。
- 确认源码示例使用 `.ck`。
- 确认 `package.json` version 与计划 release 一致。
- 确认计划使用的 git tag 与 package version 一致。
- 确认 package metadata 只暴露 `ckc` bin entrypoint。
- 确认 release notes 只宣传已经实现的能力。
- Review `docs/releases/v0.7.0.md` 或计划版本对应的 release note。
- 确认 release notes 把 `CKWasmArena` 描述为 JS/WASM interop helper，而不是
  CK runtime。
- 确认 release notes 把 WASM 性能口径限制在已测 workload 内，且不把 `DataView`
  推荐为高吞吐路径。
- 确认 f64 被记录为 Phase 16 strict support，而不是 fast-math 或 SIMD support。
- 确认 exact explicit `i32_to_f64` / `u32_to_f64` cast 被记录为 Phase 20
  support，而不是通用 cast system。
- 不要宣传未支持的 f32、implicit int/float conversion、`f64 %`、fast-math、
  SIMD、JIT、strings、IO、GC、runtime、checked WASM/LLVM arithmetic 或新 backend
  support。

## 必跑命令

- 运行 `pnpm test`。
- 运行 `pnpm typecheck`。
- 运行 `pnpm build`。
- 运行 `npm pack --dry-run`。
- 按下面的流程运行真实 package fresh-install smoke，其中包括 `npm pack`。
- 运行 `pnpm ckc --help`，或等价的已安装 `ckc --help`，并 review 输出。
- 运行 `pnpm ckc check examples/pricing.ck`。
- 运行 `pnpm ckc check examples/explicit_casts.ck`。
- 运行 `pnpm ckc emit-c examples/pricing.ck --out build/pricing.c --header build/pricing.h`。
- 运行 `pnpm ckc emit-mir examples/pricing.ck --out build/pricing.mir`。

## Naming Consistency

- 确认 `tests/naming-consistency.test.ts` 已作为 `pnpm test` 的一部分通过。
- Review README、docs、examples、snapshots 和 CLI help 的标准命名。
- 确认 package metadata、命令示例和 release docs 使用 CK / CalcKernel、`ckc`
  和 `.ck`。
- 确认 rename migration guide 记录 breaking rename、no-alias compatibility
  policy、package rename note 和 `v0.7.0` recommendation。
- 确认没有引入 compatibility alias 或替代源码后缀。

## C Backend Regression

- 确认 C emitter snapshot tests 通过。
- 确认 C scalar expression 覆盖通过。
- 确认 C if/else 和 while 覆盖通过。
- 确认 C function-call 覆盖通过。
- 确认 C short-circuit `&&` / `||` 覆盖通过。
- 确认 C ptr/index、struct field，以及 `items[i].price` 这类 combined access
  覆盖通过。
- 确认 `examples/pricing.ck` C e2e 通过。
- 确认 unchecked mode 仍是默认值，并保持 unchecked ABI。
- 确认 checked C mode 的 scalar、control-flow、short-circuit、function-call 和
  `examples/pricing.ck` e2e 覆盖通过。
- 确认 C explicit cast regression 通过 `i32_to_f64` 和 `u32_to_f64`，包括
  checked C mode 下 cast 仍是普通 exact f64 conversion。
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
- 确认 `examples/pricing.ck` WASM e2e 通过。
- 确认 WASM explicit cast regression 通过，并且 WAT output 包含
  `f64.convert_i32_s` / `f64.convert_i32_u`。
- 确认 `examples/node-wasm-f64-array/` 的 WASM f64 `Float64Array` example smoke
  通过。
- 确认文档说明 `ptr<f64>` 是 `i32` byte offset、`f64` size 是 8、
  `byteOffset / 8` typed-array index，以及 memory management 由 host 负责。
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
- 确认 `examples/pricing.ck` LLVM e2e 通过。
- 确认 LLVM explicit cast regression 通过，IR output 包含不带 fast-math flag 的
  `sitofp` / `uitofp`。
- 确认 `emit-llvm --overflow checked` 和 `build-llvm --overflow checked` 会以文档中的
  unsupported-mode message 失败。
- Review `docs/LLVM_BACKEND.md`，确认 backend limits 和 release notes。

## Cross-Backend Baseline

- 确认 C/WASM/LLVM backend regression comparison tests 通过。
- 确认 shared comparison 覆盖 scalar、control flow、function calls、
  short-circuit、memory 和 `examples/pricing.ck`。
- 确认已实现 backend 的 f64 scalar、ptr、struct-field、arithmetic、comparison、
  unary minus 和 backend parity regression 通过。
- 确认 explicit `i32_to_f64` / `u32_to_f64` backend regression 在 C、WASM、LLVM
  上通过。
- 确认 cross-backend f64 behavior matrix 覆盖有限值、NaN、infinity、`-0.0`、
  f64 comparison、`ptr<f64>` 和 struct f64 field。
- 确认有限 f64 值使用 tolerance，NaN、infinity 和 signed zero 使用分类判断，而不是
  exact bit comparison。
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
- 确认 `examples/pricing.ck` 优化前后结果一致。
- 确认没有为 `examples/pricing.ck` 添加 benchmark-specific special case。
- 确认 f64 strict-safety tests 通过：
  - 不做 f64 constant folding
  - 不做 f64 reassociation
  - local CSE 不排序 f64 operand
  - 只允许 same-order f64 `+`、`-`、`*` 和 unary `-` local CSE
  - 不做 f64 LICM hoisting
  - 不做 f64 induction simplification
  - 不生成 LLVM fast-math flag
  - 不做 NaN、infinity 或 `-0.0` 敏感的代数重写
  - 不做 cast constant folding
  - explicit cast local CSE 只复用完全相同 kind 的 cast

## Float Semantics Lock

- 确认语言文档描述 f64 primitive、float literal、arithmetic、unary minus、
  comparison、`ptr<f64>` 和包含 `f64` 的 struct field。
- 确认文档写明 f64-only policy：`f64` 是唯一 floating point type，不规划 `f32`，
  且没有 docs drift 重新引入 f32 planning 语言。
- 确认文档明确拒绝 implicit int/float conversion、general numeric cast、
  `i64_to_f64`、`u64_to_f64`、f64-to-int cast、`f64 %`、fast-math、SIMD、JIT、
  runtime、IO、strings、GC、NaN literal syntax、infinity literal syntax 和 float
  suffix literal。
- 确认文档只把 `i32_to_f64` 和 `u32_to_f64` 描述为当前 explicit cast support，
  不承诺其他 cast 方向。
- 确认 checked arithmetic 文档说明 f64 不参与 integer overflow check，f64
  division by zero 不返回 `CK_ERR_DIV_BY_ZERO`，f64 overflow 不返回
  `CK_ERR_OVERFLOW`。
- 确认 ABI/backend 文档说明 C 使用 `(double)x`，WASM 使用 `f64.convert_i32_s` /
  `f64.convert_i32_u`，LLVM 使用 `sitofp` / `uitofp`，f64 value 的 JavaScript
  interop 使用 `Number`。
- 确认文档不承诺 NaN payload 稳定，也不承诺跨 backend 浮点结果 bit-identical。
- 确认 optimizer 文档要求未来 pass 在改变 f64 expression 前必须证明
  strict-float safety。

## WASM Interop Release Claims

- 确认 `docs/wasm-interop.md`、`docs/PERFORMANCE.md`、README 和 release notes
  都说明 `CKWasmArena` 是 JS/WASM interop helper，不是 CK runtime。
- 确认 docs 推荐 homogeneous buffer 使用 WASM memory 上的 TypedArray view。
- 确认 docs 推荐在 caller ownership 允许时使用 resident memory 做重复 WASM 调用。
- 确认 docs 推荐 JavaScript 能调整 input shape 时，对 mixed-width 批量数据使用 SoA
  layout。
- 确认 docs 推荐 output view 作为 fast path，并把 `copyOutF64` 描述为显式
  JS-owned copy path。
- 确认 docs 把 `f64-sum` 和 pricing SoA 描述为推荐 examples。
- 确认 docs 把 `f64-axpy` view-output 结果描述为接近、且在当前本机 run 中略快于
  JavaScript `Float64Array` baseline，但不能写成跨机器保证。
- 确认 DataView path 被描述为 fallback/debug 或 byte-exact ABI comparison，而不是
  高吞吐路径。
- 确认 benchmark docs 说明结果依赖 hardware、Node.js/V8、OS scheduling、power
  state、system load、hyperfine 和 workload size。
- 确认 benchmark threshold 没有加入普通 `pnpm test`。
- 确认金额、税费、POS 总价和 pricing-rule docs 继续推荐 `i64` fixed-point，而不是
  用 `f64` 表示财务精确金额。

## Benchmark Smoke

- Review `docs/PERFORMANCE.md`、`bench/README.md` 和 `bench/README.zh-CN.md`。
- 运行 `node --test bench/perf/tests/perf-core.test.mjs`。
- 可选手动运行 `node bench/perf/run.mjs --quick`，作为 benchmark smoke。
- 确认 low-copy f64 WASM quick benchmark case 可通过
  `node bench/perf/run.mjs --quick --case f64` 覆盖。
- 确认 f64 benchmark summary 区分 setup、input marshal、compute-only、output
  readback、total、memory-only 和 low-copy phase。
- 确认 f64 benchmark case 覆盖 axpy、dot product、sum 和 scale。
- 确认 f64 benchmark 文档覆盖 JavaScript `Array` `Number`、JavaScript
  `Float64Array`、CK C O3、CK LLVM O3 和 CK WASM O3 对比目标。
- `node bench/perf/run.mjs --full` 是机器时间允许时的可选 tag-time 检查。
- 不要把 benchmark threshold 加进普通 `pnpm test`。
- 不要把本机 benchmark 结果当作跨机器绝对 baseline。
- 如果保存 baseline，遵循现有策略：本机 baseline 放在被忽略的 `build/perf` 输出中，
  `bench/perf/baselines/example.summary.json` 只是格式示例，不是真实阈值文件。
- 不要提交机器本地 benchmark output、真实本机 baseline、cache 或临时文件。
- 确认 `git status --short` 没有显示 tracked 或 staged 的
  `build/perf/latest.*`。
- 确认 `git status --short` 没有显示 tracked 或 staged 的
  `build/perf/baseline.local.json` 或其他开发机器 benchmark baseline。

## Docs And Examples Review

- Review README 和 README.zh-CN，确认当前能力描述准确。
- Review language、architecture、MIR、ABI、checked arithmetic、WASM、LLVM、
  optimization、performance 和 roadmap docs。
- 确认 docs 说明 CK / CalcKernel 是纯计算 DSL，不是通用语言。
- 确认 docs 说明整数 kernel 仍是主要目标，strict f64 可用于数值 kernel。
- 确认 docs 推荐金额、税费、POS 总价和 pricing-rule 计算继续使用 `i64`
  fixed-point。
- 确认 docs 没有宣称 f32、implicit int/float conversion、`f64 %`、fast-math、
  SIMD、JIT、runtime、IO、strings 或 GC 支持。
- 确认 docs/spec/ABI updates 覆盖：
  - lexer/parser f64 和 float literal
  - checker f64 arithmetic/comparison 以及 mixed int/f64 rejection
  - MIR `const_float`
  - optimizer f64 safety gates
  - C f64 regression 和 checked f64 boundary
  - LLVM f64 regression 和 no fast-math
  - WASM f64 regression 和 JS `Number` interop
  - `ptr<f64>` 和 struct f64 layout rules
- Review `examples/` 下的示例，确认命令使用 `ckc`，source file 使用 `.ck`。
- 确认 `examples/pricing.ck` 仍是 release e2e fixture。
- `pnpm build` 后运行官方 WASM interop examples：
  - `node examples/wasm/f64-sum/run.mjs`
  - `node examples/wasm/f64-axpy/run.mjs`
  - `node examples/wasm/pricing-soa/run.mjs`
- 确认这些 examples 使用 `CKWasmArena`/`createCKWasmArena`，避免 DataView hot
  path，在适用场景下让 output 保持为 WASM memory view，并且 pricing 使用 `i64`
  fixed-point。
- 保持文档双语：英文为默认入口，修改文档时同步更新中文译本。

## Package Contents Review

- 确认 `package.json` 使用 `name: "calckernel"`。
- 确认 `package.json` version 与计划 release 一致。
- 确认 `package.json` `bin` 只包含 `ckc`。
- 确认 `package.json` 不暴露 legacy bin alias 或 legacy package entrypoint。
- 确认 `package.json` 是 ESM-first（`type: "module"`），且 `exports` 与该
  module format 一致。
- 确认 `package.json` 的 `main`、`types` 和 `exports["."].types` 都指向
  `dist/src` 下的 built files。
- 确认 `package.json` `files` 有意包含 docs 和 examples，包括
  `docs/wasm-interop.md` 和 `examples/wasm/**`。
- 确认 `package.json` `files` 不发布 `bench/docs/**` 或 `bench/plans/**`
  下的内部 benchmark 历史文档；这些文件可以保留在源码仓库中，但除非被明确更新为当前
  面向用户的文档，否则不要放进 npm package。
- `pnpm build` 后确认 `dist/src/cli.js` 存在、包含 Node shebang，且可执行。
- 确认 `dist/src/index.js` 和 `dist/src/index.d.ts` 存在。
- 确认 `dist/src/wasm/ck-wasm-arena.js` 和
  `dist/src/wasm/ck-wasm-arena.d.ts` 存在。
- 确认 `dist/src/index.d.ts` 导出 `CKWasmArena`、`createCKWasmArena`、
  `CKWasmArenaCopy`、`CKWasmArenaOptions`、`CKWasmInstanceLike` 和
  `CKWasmMemory`。
- 确认 `npm pack --dry-run` 报告 package `calckernel@<version>`。
- 确认 package 包含 `dist/src`、docs、examples、bench files、README.md、
  README.zh-CN.md 和 package.json。
- 检查 `npm pack --dry-run --json` 的 file list，确认不包含 `bench/docs/**`、
  `bench/plans/**`、`build/perf/**`、本机 benchmark baseline、生成的 tarball 或
  临时目录。
- 扫描 packed package surface 中 migration guide 记录的 legacy rename token set。
  只有 migration guide、Phase 21 rename/history report、明确 legacy compatibility
  notes 和 naming tests 可以包含这些字符串。
- 如果仓库根目录存在 license file，确认 package 包含该 license file。
- 确认 package 包含 `ckc` bin mapping 指向的 built CLI entrypoint。
- 确认 package 包含 public JS API output 和 declaration files，包括 WASM interop
  helper。
- 确认 package `exports`、`files` 和 scripts 与已发布的 CK / CalcKernel surface
  一致。
- 确认没有误包含本地 build artifact、benchmark output、真实本机 baseline、cache、
  editor state 或临时日志。
- 确认 package contents 不包含 `build/perf/latest.*`、
  `build/perf/baseline.local.json`、生成的 tarball、临时目录、`node_modules`、
  coverage output 或 cache directories。

## Package Fresh Install Smoke

- 在仓库根目录运行 `npm pack`，记录生成的 tarball 名称。
- 不要提交生成的 `.tgz` tarball。
- 在仓库外创建临时目录，并运行 `npm init -y`。
- 使用 `npm install /absolute/path/to/calckernel-<version>.tgz` 安装生成的 tarball。
- 确认 `node_modules/.bin/ckc --help` 可以运行，并且 help 使用 `ckc` 命令。
- 确认没有 legacy CLI bin wrapper；`ckc` 是 package 中唯一 compiler command。
- 在临时目录创建最小 `.ck` 文件，并运行：
  - `node_modules/.bin/ckc check smoke.ck`
  - `node_modules/.bin/ckc emit-mir smoke.ck -o build/smoke.mir`
  - `node_modules/.bin/ckc emit-c smoke.ck -o build/smoke.c`
  - `node_modules/.bin/ckc emit-wat smoke.ck -o build/smoke.wat`
  - `node_modules/.bin/ckc emit-wasm smoke.ck -o build/smoke.wasm`
  - `node_modules/.bin/ckc emit-llvm smoke.ck -o build/smoke.ll`
  - 如果环境有 clang，运行
    `node_modules/.bin/ckc build-llvm smoke.ck --kind object -o build/smoke.o`。
- 确认 emitted C source 和默认生成的 C header 都存在且非空。
- 确认 package JS import smoke 通过，且使用当前支持的 module format：
  `import { CKWasmArena, createCKWasmArena } from "calckernel"`。当前 package
  是 ESM-first，不提供 CommonJS `require` export。
- 确认 TypeScript consumer smoke 能使用发布包中的 declaration files，通过
  `CKWasmArena`、`createCKWasmArena`、`CKWasmArenaCopy` 和
  `CKWasmInstanceLike` 类型检查。
- 确认 fresh install 后 WASM interop smoke 通过：使用安装后的 `ckc` 生成
  `smoke.wasm`，在 Node.js 中 instantiate，通过 `createCKWasmArena(instance)`
  创建 arena，用 `copyInF64` 写入 input，调用生成的 WASM export，用 `viewF64`
  读取 output，并确认 `copyOutF64` 返回 JS-owned copy。
- smoke source 应覆盖 f64 params/returns、f64 arithmetic、unary minus、f64
  comparison、`ptr<f64>` 和包含 `f64` 的 struct field。
- smoke 完成后删除临时目录和生成的 tarball；如果有本地 artifact 留下，必须明确报告。

## Tag Gate

- 确认上面的必跑命令和人工 review 都已完成。
- 确认 working tree changes 都是有意且已理解的。
- 人工 review 后再创建 release commit；不要让 release tooling 自动 commit 这些改动。
- 确认 release notes 只总结已实现能力。
- 确认 known limitations 已列出：floating point 是 f64-only、不规划 f32、没有
  implicit int/float conversion、只有 exact explicit `i32_to_f64` / `u32_to_f64`
  cast、没有 `i64/u64` to f64 cast、没有 f64-to-int cast、没有 `f64 %`、没有
  fast-math、没有 SIMD、没有 JIT、没有 IO、没有 strings、没有 GC、没有 runtime、
  没有 float checked overflow，也没有 checked WASM/LLVM arithmetic。
- 只有 checklist 完成后才人工创建 release tag。
- 不要自动 npm publish；publish 必须是单独明确的人工 release action。
