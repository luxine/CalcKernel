import { f64Kernels } from "./f64-workloads.mjs";

export function benchmarkCommands(config, paths) {
  const itemArgs = `--items ${config.items} --iterations ${config.iterations}`;
  const callCount = config.mode === "quick" ? 1000 : 10000;
  const executable = (name) => `${paths.binDir}/${name}${paths.executableSuffix ?? ""}`;
  const pricingWasm = `${paths.generatedDir}/pricing.wasm`;
  const pricingWasmO3 = `${paths.generatedDir}/pricing_o3.wasm`;
  const pricingSoAWasmO3 = `${paths.generatedDir}/pricing_soa_o3.wasm`;
  const callOverheadWasm = `${paths.generatedDir}/call_overhead.wasm`;
  const f64WasmO3 = `${paths.generatedDir}/f64_kernels_o3.wasm`;

  return [
    nativeCase("pricing-c-unchecked-O0", "O0", "unchecked", `${executable("pricing-c-unchecked-O0")} ${itemArgs}`),
    nativeCase("pricing-c-unchecked-O2", "O2", "unchecked", `${executable("pricing-c-unchecked-O2")} ${itemArgs}`),
    nativeCase("pricing-c-unchecked-O3", "O3", "unchecked", `${executable("pricing-c-unchecked-O3")} ${itemArgs}`),
    nativeCase("pricing-c-unchecked-ck-O3", "CK-O3/clang-O3", "unchecked", `${executable("pricing-c-unchecked-ck-O3")} ${itemArgs}`),
    nativeCase("pricing-c-checked-O3", "CK-O3/clang-O3", "checked", `${executable("pricing-c-checked-O3")} ${itemArgs}`),
    nativeCase("pricing-helpers-c-unchecked-ck-O0", "CK-O0/clang-O3", "unchecked", `${executable("pricing-helpers-c-unchecked-ck-O0")} ${itemArgs}`),
    nativeCase("pricing-helpers-c-unchecked-ck-O2", "CK-O2/clang-O3", "unchecked", `${executable("pricing-helpers-c-unchecked-ck-O2")} ${itemArgs}`),
    nativeCase("pricing-llvm-unchecked-O0", "O0", "unchecked", `${executable("pricing-llvm-unchecked-O0")} ${itemArgs}`),
    nativeCase("pricing-llvm-unchecked-O2", "O2", "unchecked", `${executable("pricing-llvm-unchecked-O2")} ${itemArgs}`),
    nativeCase("pricing-llvm-unchecked-O3", "O3", "unchecked", `${executable("pricing-llvm-unchecked-O3")} ${itemArgs}`),
    wasmCase(
      "pricing-wasm-unchecked-total",
      "wasm",
      "CK-O0",
      `node bench/perf/cases/pricing-wasm.mjs --mode total --wasm ${pricingWasm} ${itemArgs}`,
      "total",
      pricingDataViewTotalInterop()
    ),
    wasmCase(
      "pricing-wasm-unchecked-total-O3",
      "wasm",
      "CK-O3",
      `node bench/perf/cases/pricing-wasm.mjs --mode total --wasm ${pricingWasmO3} ${itemArgs}`,
      "total",
      pricingDataViewTotalInterop()
    ),
    wasmCase(
      "pricing-wasm-unchecked-compute-only",
      "wasm",
      "CK-O0",
      `node bench/perf/cases/pricing-wasm.mjs --mode compute-only --wasm ${pricingWasm} ${itemArgs}`,
      "compute",
      pricingComputeOnlyInterop()
    ),
    wasmCase(
      "pricing-wasm-unchecked-compute-only-O3",
      "wasm",
      "CK-O3",
      `node bench/perf/cases/pricing-wasm.mjs --mode compute-only --wasm ${pricingWasmO3} ${itemArgs}`,
      "compute",
      pricingComputeOnlyInterop()
    ),
    wasmCase(
      "pricing-wasm-unchecked-memory-only",
      "memory",
      "n/a",
      `node bench/perf/cases/pricing-wasm.mjs --mode memory-only --wasm ${pricingWasm} ${itemArgs}`,
      "memory-only",
      {
        benchmarkLayer: "wasm-dataview-memory-only",
        dataViewHotPath: "yes",
        copyInput: "dataview-per-iteration",
        copyOutput: "dataview-per-iteration",
        outputOwnership: "js-owned-copy",
        memoryGrow: "host-memory-if-needed"
      }
    ),
    wasmCase(
      "pricing-wasm-unchecked-call-overhead",
      "call-overhead",
      "n/a",
      `node bench/perf/cases/pricing-wasm.mjs --mode call-overhead --wasm ${callOverheadWasm} --calls ${callCount}`,
      "call-overhead",
      {
        benchmarkLayer: "wasm-kernel-call",
        dataViewHotPath: "no",
        copyInput: "none",
        copyOutput: "none",
        outputOwnership: "none",
        memoryGrow: "no"
      }
    ),
    ...pricingSoABenchmarkCommands(itemArgs, pricingSoAWasmO3),
    jsCase("pricing-js-number", "host", `node bench/perf/cases/pricing-js-number.mjs ${itemArgs}`),
    jsCase("pricing-js-typedarray-number", "host", `node bench/perf/cases/pricing-js-typedarray-number.mjs ${itemArgs}`),
    jsCase("pricing-js-bigint", "host", `node bench/perf/cases/pricing-js-bigint.mjs ${itemArgs}`),
    ...f64BenchmarkCommands(f64Kernels, itemArgs, executable, f64WasmO3)
  ];
}

function pricingSoABenchmarkCommands(itemArgs, wasmO3) {
  return [
    wasmCase(
      "pricing-wasm-soa-setup-copy-in-O3",
      "wasm-low-copy",
      "CK-O3",
      `node bench/perf/cases/pricing-wasm.mjs --mode soa-setup-copy-in --wasm ${wasmO3} ${itemArgs}`,
      "setup-copy-in",
      {
        benchmarkLayer: "wasm-soa-setup-copy-in",
        dataViewHotPath: "no",
        copyInput: "bigint64array-set-setup-once",
        copyOutput: "none",
        outputOwnership: "wasm-memory-view",
        memoryGrow: "pre-grown"
      }
    ),
    wasmCase(
      "pricing-wasm-soa-resident-total-O3",
      "wasm-low-copy",
      "CK-O3",
      `node bench/perf/cases/pricing-wasm.mjs --mode soa-resident-total --wasm ${wasmO3} ${itemArgs}`,
      "resident-total",
      {
        benchmarkLayer: "wasm-soa-resident-total",
        dataViewHotPath: "no",
        copyInput: "bigint64array-set-setup-once",
        copyOutput: "none",
        outputOwnership: "wasm-memory-view",
        memoryGrow: "pre-grown"
      }
    ),
    wasmCase(
      "pricing-wasm-soa-readback-cost-O3",
      "wasm-low-copy",
      "CK-O3",
      `node bench/perf/cases/pricing-wasm.mjs --mode soa-readback-cost --wasm ${wasmO3} ${itemArgs}`,
      "readback-cost",
      {
        benchmarkLayer: "wasm-soa-readback-cost",
        dataViewHotPath: "no",
        copyInput: "bigint64array-set-setup-once",
        copyOutput: "bigint64array-view-checksum-per-iteration",
        outputOwnership: "wasm-memory-view",
        memoryGrow: "pre-grown"
      }
    ),
    wasmCase(
      "pricing-wasm-soa-total-with-final-readback-O3",
      "wasm-low-copy",
      "CK-O3",
      `node bench/perf/cases/pricing-wasm.mjs --mode soa-total-with-final-readback --wasm ${wasmO3} ${itemArgs}`,
      "total-with-final-readback",
      {
        benchmarkLayer: "wasm-soa-total-final-readback",
        dataViewHotPath: "no",
        copyInput: "bigint64array-set-setup-once",
        copyOutput: "bigint64array-view-checksum-once",
        outputOwnership: "wasm-memory-view",
        memoryGrow: "pre-grown"
      }
    )
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
      name: "pricing-c-unchecked-ck-O3",
      optLevel: "O3",
      source: `${generated}/pricing_ck_o3.c`,
      harness: "bench/perf/cases/pricing-c-unchecked.c",
      output: executable("pricing-c-unchecked-ck-O3")
    },
    {
      name: "pricing-c-checked-O3",
      optLevel: "O3",
      source: `${generated}/pricing.checked.c`,
      harness: "bench/perf/cases/pricing-c-checked.c",
      output: executable("pricing-c-checked-O3")
    },
    {
      name: "pricing-helpers-c-unchecked-ck-O0",
      optLevel: "O3",
      source: `${generated}/pricing_helpers_ck_o0.c`,
      harness: "bench/perf/cases/pricing-helpers-c-unchecked.c",
      output: executable("pricing-helpers-c-unchecked-ck-O0")
    },
    {
      name: "pricing-helpers-c-unchecked-ck-O2",
      optLevel: "O3",
      source: `${generated}/pricing_helpers_ck_o2.c`,
      harness: "bench/perf/cases/pricing-helpers-c-unchecked.c",
      output: executable("pricing-helpers-c-unchecked-ck-O2")
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
      name: "f64-ck-c-o3",
      optLevel: "O3",
      source: `${generated}/f64_kernels.c`,
      harness: "bench/perf/cases/f64-native.c",
      output: executable("f64-ck-c-o3")
    },
    {
      name: "f64-ck-llvm-o3",
      optLevel: "O3",
      source: `${generated}/f64_kernels.ll`,
      harness: "bench/perf/cases/f64-native.c",
      output: executable("f64-ck-llvm-o3")
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
      `f64-${kernel}-ck-c-o3`,
      "CK-O3/clang-O3",
      "unchecked",
      `${executable("f64-ck-c-o3")} --kernel ${kernel} --label f64-${kernel}-ck-c-o3 ${itemArgs}`
    ),
    nativeCase(
      `f64-${kernel}-ck-llvm-o3`,
      "CK-O3/LLVM-O3",
      "unchecked",
      `${executable("f64-ck-llvm-o3")} --kernel ${kernel} --label f64-${kernel}-ck-llvm-o3 ${itemArgs}`
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-setup`,
      "wasm",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode setup --label f64-${kernel}-ck-wasm-o3-setup --wasm ${wasmO3} ${itemArgs}`,
      "setup",
      f64SetupInterop("data-view")
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-input-marshal`,
      "memory",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode input-marshal --label f64-${kernel}-ck-wasm-o3-input-marshal --wasm ${wasmO3} ${itemArgs}`,
      "input-marshal",
      f64InputMarshalInterop("data-view")
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-compute-only`,
      "wasm",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode compute-only --label f64-${kernel}-ck-wasm-o3-compute-only --wasm ${wasmO3} ${itemArgs}`,
      "compute",
      f64ComputeOnlyInterop("data-view", kernel)
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-output-readback`,
      "memory",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode output-readback --label f64-${kernel}-ck-wasm-o3-output-readback --wasm ${wasmO3} ${itemArgs}`,
      "output-readback",
      f64OutputReadbackInterop("data-view", kernel)
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-total`,
      "wasm",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode total --label f64-${kernel}-ck-wasm-o3-total --wasm ${wasmO3} ${itemArgs}`,
      "total",
      f64TotalInterop("data-view", kernel)
    ),
    wasmCase(
      `f64-${kernel}-wasm-memory-only`,
      "memory",
      "n/a",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --mode memory-only --label f64-${kernel}-wasm-memory-only --wasm ${wasmO3} ${itemArgs}`,
      "memory-only",
      {
        benchmarkLayer: "wasm-dataview-memory-only",
        dataViewHotPath: "yes",
        copyInput: "dataview-per-iteration",
        copyOutput: "dataview-per-iteration",
        outputOwnership: "js-owned-checksum",
        memoryGrow: "host-memory-if-needed"
      }
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-low-copy-setup`,
      "wasm-low-copy",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --copy-mode float64array --mode setup --label f64-${kernel}-ck-wasm-o3-low-copy-setup --wasm ${wasmO3} ${itemArgs}`,
      "setup",
      f64SetupInterop("float64array")
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-low-copy-input-marshal`,
      "memory-low-copy",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --copy-mode float64array --mode input-marshal --label f64-${kernel}-ck-wasm-o3-low-copy-input-marshal --wasm ${wasmO3} ${itemArgs}`,
      "input-marshal",
      f64InputMarshalInterop("float64array")
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-low-copy-compute-only`,
      "wasm-low-copy",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --copy-mode float64array --mode compute-only --label f64-${kernel}-ck-wasm-o3-low-copy-compute-only --wasm ${wasmO3} ${itemArgs}`,
      "compute",
      f64ComputeOnlyInterop("float64array", kernel)
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-low-copy-output-readback`,
      "memory-low-copy",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --copy-mode float64array --mode output-readback --label f64-${kernel}-ck-wasm-o3-low-copy-output-readback --wasm ${wasmO3} ${itemArgs}`,
      "output-readback",
      f64OutputReadbackInterop("float64array", kernel)
    ),
    wasmCase(
      `f64-${kernel}-ck-wasm-o3-low-copy-total`,
      "wasm-low-copy",
      "CK-O3",
      `node bench/perf/cases/f64-wasm.mjs --kernel ${kernel} --copy-mode float64array --mode total --label f64-${kernel}-ck-wasm-o3-low-copy-total --wasm ${wasmO3} ${itemArgs}`,
      "total",
      f64TotalInterop("float64array", kernel)
    ),
    ...f64OptimizedBenchmarkCommands(kernel, itemArgs, wasmO3)
  ]);
}

function f64OptimizedBenchmarkCommands(kernel, itemArgs, wasmO3) {
  if (kernel === "sum") {
    return [
      wasmCase(
        "f64-sum-ck-wasm-o3-optimized-low-copy-total",
        "wasm-low-copy",
        "CK-O3",
        `node bench/perf/cases/f64-wasm.mjs --kernel sum --mode optimized-total --label f64-sum-ck-wasm-o3-optimized-low-copy-total --wasm ${wasmO3} ${itemArgs}`,
        "optimized-total",
        {
          benchmarkLayer: "wasm-optimized-low-copy-total",
          dataViewHotPath: "no",
          copyInput: "float64array-set-setup-once",
          copyOutput: "scalar-return",
          outputOwnership: "scalar-return",
          memoryGrow: "pre-grown"
        }
      )
    ];
  }

  if (kernel === "axpy") {
    return [
      wasmCase(
        "f64-axpy-ck-wasm-o3-view-output-total",
        "wasm-low-copy",
        "CK-O3",
        `node bench/perf/cases/f64-wasm.mjs --kernel axpy --mode view-output-total --label f64-axpy-ck-wasm-o3-view-output-total --wasm ${wasmO3} ${itemArgs}`,
        "view-output-total",
        {
          benchmarkLayer: "wasm-view-output-total",
          dataViewHotPath: "no",
          copyInput: "float64array-set-x-once-y-per-iteration",
          copyOutput: "none",
          outputOwnership: "wasm-memory-view",
          memoryGrow: "pre-grown"
        }
      ),
      wasmCase(
        "f64-axpy-ck-wasm-o3-copy-output-total",
        "wasm-low-copy",
        "CK-O3",
        `node bench/perf/cases/f64-wasm.mjs --kernel axpy --mode copy-output-total --label f64-axpy-ck-wasm-o3-copy-output-total --wasm ${wasmO3} ${itemArgs}`,
        "copy-output-total",
        {
          benchmarkLayer: "wasm-copy-output-total",
          dataViewHotPath: "no",
          copyInput: "float64array-set-x-once-y-per-iteration",
          copyOutput: "copyout-f64-per-iteration",
          outputOwnership: "js-owned-copy",
          memoryGrow: "pre-grown"
        }
      )
    ];
  }

  return [];
}

function nativeCase(name, optLevel, overflowMode, command, phase = "total") {
  return {
    name,
    command,
    category: "native",
    phase,
    optLevel,
    overflowMode,
    ...interopMetadata({
      benchmarkLayer: "native-total",
      dataViewHotPath: "no",
      copyInput: "none",
      copyOutput: "none",
      outputOwnership: "native-process",
      memoryGrow: "no"
    })
  };
}

function wasmCase(name, category, optLevel, command, phase = "total", interop = {}) {
  return {
    name,
    command,
    category,
    phase,
    optLevel,
    overflowMode: "unchecked",
    ...interopMetadata(interop)
  };
}

function jsCase(name, overflowMode, command, phase = "total", interop = {}) {
  return {
    name,
    command,
    category: "js",
    phase,
    optLevel: "n/a",
    overflowMode,
    ...interopMetadata({
      benchmarkLayer: jsBenchmarkLayer(name),
      dataViewHotPath: "no",
      copyInput: "none",
      copyOutput: "none",
      outputOwnership: "js-owned",
      memoryGrow: "no",
      ...interop
    })
  };
}

function interopMetadata(overrides = {}) {
  return {
    benchmarkLayer: "unknown",
    dataViewHotPath: "unknown",
    copyInput: "unknown",
    copyOutput: "unknown",
    outputOwnership: "unknown",
    memoryGrow: "unknown",
    ...overrides
  };
}

function jsBenchmarkLayer(name) {
  if (name.includes("bigint")) {
    return "js-bigint-total";
  }
  if (name.includes("typedarray") || name.includes("float64array")) {
    return "js-typedarray-total";
  }
  if (name.includes("array-number")) {
    return "js-array-total";
  }
  return "js-number-total";
}

function pricingDataViewTotalInterop() {
  return {
    benchmarkLayer: "wasm-dataview-total",
    dataViewHotPath: "yes",
    copyInput: "dataview-per-iteration",
    copyOutput: "dataview-per-iteration",
    outputOwnership: "js-owned-copy",
    memoryGrow: "if-needed"
  };
}

function pricingComputeOnlyInterop() {
  return {
    benchmarkLayer: "wasm-compute-only",
    dataViewHotPath: "no",
    copyInput: "dataview-setup-once",
    copyOutput: "dataview-final-checksum",
    outputOwnership: "js-owned-copy",
    memoryGrow: "if-needed"
  };
}

function f64SetupInterop(copyMode) {
  return {
    benchmarkLayer: "wasm-setup",
    dataViewHotPath: "no",
    copyInput: "none",
    copyOutput: "none",
    outputOwnership: "none",
    memoryGrow: copyMode === "float64array" ? "if-needed-refresh-view" : "if-needed"
  };
}

function f64InputMarshalInterop(copyMode) {
  return {
    benchmarkLayer: "wasm-setup-copy-in",
    dataViewHotPath: copyMode === "data-view" ? "yes" : "no",
    copyInput: copyMode === "data-view" ? "dataview-per-iteration" : "float64array-set-per-iteration",
    copyOutput: "none",
    outputOwnership: "none",
    memoryGrow: "if-needed"
  };
}

function f64ComputeOnlyInterop(copyMode, kernel) {
  return {
    benchmarkLayer: "wasm-compute-only",
    dataViewHotPath: "no",
    copyInput: copyMode === "data-view" ? "dataview-setup-once" : "float64array-set-setup-once",
    copyOutput: f64ScalarKernel(kernel) ? "scalar-return" : "none",
    outputOwnership: f64ScalarKernel(kernel) ? "scalar-return" : "wasm-memory",
    memoryGrow: "if-needed"
  };
}

function f64OutputReadbackInterop(copyMode, kernel) {
  if (copyMode === "float64array") {
    return {
      benchmarkLayer: "wasm-readback-copy-out",
      dataViewHotPath: "no",
      copyInput: "float64array-set-setup-once",
      copyOutput: f64ScalarKernel(kernel) ? "scalar-return" : "float64array-view-checksum-per-iteration",
      outputOwnership: f64ScalarKernel(kernel) ? "scalar-return" : "wasm-memory-view",
      memoryGrow: "if-needed"
    };
  }

  return {
    benchmarkLayer: "wasm-readback-copy-out",
    dataViewHotPath: "yes",
    copyInput: "dataview-setup-once",
    copyOutput: "dataview-per-iteration",
    outputOwnership: f64ScalarKernel(kernel) ? "scalar-return-plus-js-checksum" : "js-owned-checksum",
    memoryGrow: "if-needed"
  };
}

function f64TotalInterop(copyMode, kernel) {
  if (copyMode === "float64array") {
    return {
      benchmarkLayer: "wasm-low-copy-total",
      dataViewHotPath: "no",
      copyInput: "float64array-set-per-iteration",
      copyOutput: f64ScalarKernel(kernel) ? "scalar-return" : "float64array-view-checksum-per-iteration",
      outputOwnership: f64ScalarKernel(kernel) ? "scalar-return" : "wasm-memory-view",
      memoryGrow: "if-needed"
    };
  }

  return {
    benchmarkLayer: "wasm-dataview-total",
    dataViewHotPath: "yes",
    copyInput: "dataview-per-iteration",
    copyOutput: "dataview-per-iteration",
    outputOwnership: f64ScalarKernel(kernel) ? "scalar-return-plus-js-checksum" : "js-owned-checksum",
    memoryGrow: "if-needed"
  };
}

function f64ScalarKernel(kernel) {
  return kernel === "dot" || kernel === "sum";
}
