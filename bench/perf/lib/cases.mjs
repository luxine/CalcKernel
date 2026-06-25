import { f64Kernels } from "./f64-workloads.mjs";

export function benchmarkCommands(config, paths) {
  const itemArgs = `--items ${config.items} --iterations ${config.iterations}`;
  const callCount = config.mode === "quick" ? 1000 : 10000;
  const executable = (name) => `${paths.binDir}/${name}${paths.executableSuffix ?? ""}`;
  const pricingWasm = `${paths.generatedDir}/pricing.wasm`;
  const pricingWasmO3 = `${paths.generatedDir}/pricing_o3.wasm`;
  const callOverheadWasm = `${paths.generatedDir}/call_overhead.wasm`;
  const f64WasmO3 = `${paths.generatedDir}/f64_kernels_o3.wasm`;

  return [
    nativeCase("pricing-c-unchecked-O0", "O0", "unchecked", `${executable("pricing-c-unchecked-O0")} ${itemArgs}`),
    nativeCase("pricing-c-unchecked-O2", "O2", "unchecked", `${executable("pricing-c-unchecked-O2")} ${itemArgs}`),
    nativeCase("pricing-c-unchecked-O3", "O3", "unchecked", `${executable("pricing-c-unchecked-O3")} ${itemArgs}`),
    nativeCase("pricing-c-unchecked-ik-O3", "IK-O3/clang-O3", "unchecked", `${executable("pricing-c-unchecked-ik-O3")} ${itemArgs}`),
    nativeCase("pricing-c-checked-O3", "IK-O3/clang-O3", "checked", `${executable("pricing-c-checked-O3")} ${itemArgs}`),
    nativeCase("pricing-helpers-c-unchecked-ik-O0", "IK-O0/clang-O3", "unchecked", `${executable("pricing-helpers-c-unchecked-ik-O0")} ${itemArgs}`),
    nativeCase("pricing-helpers-c-unchecked-ik-O2", "IK-O2/clang-O3", "unchecked", `${executable("pricing-helpers-c-unchecked-ik-O2")} ${itemArgs}`),
    nativeCase("pricing-llvm-unchecked-O0", "O0", "unchecked", `${executable("pricing-llvm-unchecked-O0")} ${itemArgs}`),
    nativeCase("pricing-llvm-unchecked-O2", "O2", "unchecked", `${executable("pricing-llvm-unchecked-O2")} ${itemArgs}`),
    nativeCase("pricing-llvm-unchecked-O3", "O3", "unchecked", `${executable("pricing-llvm-unchecked-O3")} ${itemArgs}`),
    wasmCase("pricing-wasm-unchecked-total", "wasm", "IK-O0", `node bench/perf/cases/pricing-wasm.mjs --mode total --wasm ${pricingWasm} ${itemArgs}`),
    wasmCase(
      "pricing-wasm-unchecked-total-O3",
      "wasm",
      "IK-O3",
      `node bench/perf/cases/pricing-wasm.mjs --mode total --wasm ${pricingWasmO3} ${itemArgs}`
    ),
    wasmCase(
      "pricing-wasm-unchecked-compute-only",
      "wasm",
      "IK-O0",
      `node bench/perf/cases/pricing-wasm.mjs --mode compute-only --wasm ${pricingWasm} ${itemArgs}`
    ),
    wasmCase(
      "pricing-wasm-unchecked-compute-only-O3",
      "wasm",
      "IK-O3",
      `node bench/perf/cases/pricing-wasm.mjs --mode compute-only --wasm ${pricingWasmO3} ${itemArgs}`
    ),
    wasmCase(
      "pricing-wasm-unchecked-memory-only",
      "memory",
      "n/a",
      `node bench/perf/cases/pricing-wasm.mjs --mode memory-only --wasm ${pricingWasm} ${itemArgs}`
    ),
    wasmCase(
      "pricing-wasm-unchecked-call-overhead",
      "call-overhead",
      "n/a",
      `node bench/perf/cases/pricing-wasm.mjs --mode call-overhead --wasm ${callOverheadWasm} --calls ${callCount}`
    ),
    jsCase("pricing-js-number", "host", `node bench/perf/cases/pricing-js-number.mjs ${itemArgs}`),
    jsCase("pricing-js-typedarray-number", "host", `node bench/perf/cases/pricing-js-typedarray-number.mjs ${itemArgs}`),
    jsCase("pricing-js-bigint", "host", `node bench/perf/cases/pricing-js-bigint.mjs ${itemArgs}`),
    ...f64BenchmarkCommands(f64Kernels, itemArgs, executable, f64WasmO3)
  ];
}

export function nativeCompileJobs(paths) {
  const generated = paths.generatedDir;
  const executable = (name) => `${paths.binDir}/${name}${paths.executableSuffix ?? ""}`;

  return [
    {
      name: "pricing-c-unchecked-O0",
      optLevel: "O0",
      source: `${generated}/pricing.c`,
      harness: "bench/perf/cases/pricing-c-unchecked.c",
      output: executable("pricing-c-unchecked-O0")
    },
    {
      name: "pricing-c-unchecked-O2",
      optLevel: "O2",
      source: `${generated}/pricing.c`,
      harness: "bench/perf/cases/pricing-c-unchecked.c",
      output: executable("pricing-c-unchecked-O2")
    },
    {
      name: "pricing-c-unchecked-O3",
      optLevel: "O3",
      source: `${generated}/pricing.c`,
      harness: "bench/perf/cases/pricing-c-unchecked.c",
      output: executable("pricing-c-unchecked-O3")
    },
    {
      name: "pricing-c-unchecked-ik-O3",
      optLevel: "O3",
      source: `${generated}/pricing_ik_o3.c`,
      harness: "bench/perf/cases/pricing-c-unchecked.c",
      output: executable("pricing-c-unchecked-ik-O3")
    },
    {
      name: "pricing-c-checked-O3",
      optLevel: "O3",
      source: `${generated}/pricing.checked.c`,
      harness: "bench/perf/cases/pricing-c-checked.c",
      output: executable("pricing-c-checked-O3")
    },
    {
      name: "pricing-helpers-c-unchecked-ik-O0",
      optLevel: "O3",
      source: `${generated}/pricing_helpers_ik_o0.c`,
      harness: "bench/perf/cases/pricing-helpers-c-unchecked.c",
      output: executable("pricing-helpers-c-unchecked-ik-O0")
    },
    {
      name: "pricing-helpers-c-unchecked-ik-O2",
      optLevel: "O3",
      source: `${generated}/pricing_helpers_ik_o2.c`,
      harness: "bench/perf/cases/pricing-helpers-c-unchecked.c",
      output: executable("pricing-helpers-c-unchecked-ik-O2")
    },
    {
      name: "pricing-llvm-unchecked-O0",
      optLevel: "O0",
      source: `${generated}/pricing.ll`,
      harness: "bench/perf/cases/pricing-c-unchecked.c",
      output: executable("pricing-llvm-unchecked-O0")
    },
    {
      name: "pricing-llvm-unchecked-O2",
      optLevel: "O2",
      source: `${generated}/pricing.ll`,
      harness: "bench/perf/cases/pricing-c-unchecked.c",
      output: executable("pricing-llvm-unchecked-O2")
    },
    {
      name: "pricing-llvm-unchecked-O3",
      optLevel: "O3",
      source: `${generated}/pricing.ll`,
      harness: "bench/perf/cases/pricing-c-unchecked.c",
      output: executable("pricing-llvm-unchecked-O3")
    },
    {
      name: "f64-ik-c-o3",
      optLevel: "O3",
      source: `${generated}/f64_kernels.c`,
      harness: "bench/perf/cases/f64-native.c",
      output: executable("f64-ik-c-o3")
    },
    {
      name: "f64-ik-llvm-o3",
      optLevel: "O3",
      source: `${generated}/f64_kernels.ll`,
      harness: "bench/perf/cases/f64-native.c",
      output: executable("f64-ik-llvm-o3")
    }
  ];
}

function f64BenchmarkCommands(kernels, itemArgs, executable, wasmO3) {
  return kernels.flatMap((kernel) => [
    jsCase(
      `f64-${kernel}-js-array-number`,
      "host",
      `node bench/perf/cases/f64-js-array-number.mjs --kernel ${kernel} ${itemArgs}`
    ),
    jsCase(
      `f64-${kernel}-js-float64array`,
      "host",
      `node bench/perf/cases/f64-js-float64array.mjs --kernel ${kernel} ${itemArgs}`
    ),
    nativeCase(
      `f64-${kernel}-ik-c-o3`,
      "IK-O3/clang-O3",
      "unchecked",
      `${executable("f64-ik-c-o3")} --kernel ${kernel} --label f64-${kernel}-ik-c-o3 ${itemArgs}`
    ),
    nativeCase(
      `f64-${kernel}-ik-llvm-o3`,
      "IK-O3/LLVM-O3",
      "unchecked",
      `${executable("f64-ik-llvm-o3")} --kernel ${kernel} --label f64-${kernel}-ik-llvm-o3 ${itemArgs}`
    ),
    wasmCase(
      `f64-${kernel}-ik-wasm-o3-compute-only`,
      "wasm",
      "IK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode compute-only --label f64-${kernel}-ik-wasm-o3-compute-only --wasm ${wasmO3} ${itemArgs}`
    ),
    wasmCase(
      `f64-${kernel}-ik-wasm-o3-total`,
      "wasm",
      "IK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode total --label f64-${kernel}-ik-wasm-o3-total --wasm ${wasmO3} ${itemArgs}`
    ),
    wasmCase(
      `f64-${kernel}-wasm-memory-only`,
      "memory",
      "n/a",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode memory-only --label f64-${kernel}-wasm-memory-only --wasm ${wasmO3} ${itemArgs}`
    )
  ]);
}

function nativeCase(name, optLevel, overflowMode, command) {
  return { name, command, category: "native", optLevel, overflowMode };
}

function wasmCase(name, category, optLevel, command) {
  return { name, command, category, optLevel, overflowMode: "unchecked" };
}

function jsCase(name, overflowMode, command) {
  return { name, command, category: "js", optLevel: "n/a", overflowMode };
}
