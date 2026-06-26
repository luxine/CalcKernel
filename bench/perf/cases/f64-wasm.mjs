import { readFileSync } from "node:fs";
import { assertWithinTolerance } from "../lib/f64-correctness.mjs";
import {
  checksumOutputFloat64Array,
  createLowCopyF64Inputs,
  ensureDataView,
  ensureFloat64Array,
  requiredBytesFor,
  writeInputsFloat64Array
} from "../lib/f64-wasm-memory.mjs";
import {
  expectedF64ComputeSink,
  expectedF64MemorySink,
  expectedF64OutputReadbackSink,
  expectedF64ResetSink,
  f64Factor,
  initialF64X,
  initialF64Y,
  positiveInteger,
  requireF64Kernel,
  usesF64Y
} from "../lib/f64-workloads.mjs";

const config = parseArgs(process.argv.slice(2));
const result = await runMode(config);
console.log(result);

function parseArgs(argv) {
  const config = {
    items: 100000,
    iterations: 1000,
    kernel: "axpy",
    label: null,
    mode: "compute-only",
    copyMode: "data-view",
    wasm: "build/perf/generated/f64_kernels_o3.wasm"
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--items") {
      config.items = positiveInteger(argv[index + 1], "--items");
      index += 1;
    } else if (arg === "--iterations") {
      config.iterations = positiveInteger(argv[index + 1], "--iterations");
      index += 1;
    } else if (arg === "--kernel") {
      config.kernel = requireF64Kernel(argv[index + 1]);
      index += 1;
    } else if (arg === "--label") {
      config.label = argv[index + 1];
      if (!config.label) {
        throw new Error("--label requires a value");
      }
      index += 1;
    } else if (arg === "--mode") {
      config.mode = requireMode(argv[index + 1]);
      index += 1;
    } else if (arg === "--copy-mode") {
      config.copyMode = requireCopyMode(argv[index + 1]);
      index += 1;
    } else if (arg === "--wasm") {
      config.wasm = argv[index + 1];
      if (!config.wasm) {
        throw new Error("--wasm requires a path");
      }
      index += 1;
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }

  return config;
}

async function runMode(config) {
  switch (config.mode) {
    case "setup":
      return runSetup(config);
    case "input-marshal":
      if (config.copyMode === "float64array") {
        return runInputMarshalLowCopy(config);
      }
      return runInputMarshal(config);
    case "compute-only":
      if (config.copyMode === "float64array") {
        return runComputeOnlyLowCopy(config);
      }
      return runComputeOnly(config);
    case "output-readback":
      if (config.copyMode === "float64array") {
        return runOutputReadbackLowCopy(config);
      }
      return runOutputReadback(config);
    case "total":
      if (config.copyMode === "float64array") {
        return runTotalLowCopy(config);
      }
      return runTotal(config);
    case "memory-only":
      return runMemoryOnly(config);
  }
}

function requireMode(value) {
  if (
    value === "setup" ||
    value === "input-marshal" ||
    value === "compute-only" ||
    value === "output-readback" ||
    value === "total" ||
    value === "memory-only"
  ) {
    return value;
  }
  throw new Error("--mode must be setup, input-marshal, compute-only, output-readback, total, or memory-only");
}

function requireCopyMode(value) {
  if (value === "data-view" || value === "float64array") {
    return value;
  }
  throw new Error("--copy-mode must be data-view or float64array");
}

async function instantiate(path) {
  const bytes = readFileSync(path);
  return instantiateBytes(bytes);
}

async function instantiateBytes(bytes) {
  const { instance } = await WebAssembly.instantiate(bytes);
  return instance;
}

async function runSetup(config) {
  const bytes = readFileSync(config.wasm);
  const layout = requiredBytesFor(config.items);

  let actual = 0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    const instance = await instantiateBytes(bytes);
    const runtime = f64Runtime(instance, layout, config.copyMode);
    const byteLength = config.copyMode === "float64array" ? runtime.values.byteLength : runtime.view.byteLength;
    actual += byteLength >= layout.totalBytes ? 1 : 0;
  }

  if (actual !== config.iterations) {
    throw new Error(`${label(config)} setup did not provision memory for every iteration`);
  }
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runInputMarshal(config) {
  const { view, layout } = await f64Instance(config);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += writeInputs(view, layout, config.items, config.kernel);
  }

  const expected = expectedF64MemorySink(config.kernel, config.items, config.iterations);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runInputMarshalLowCopy(config) {
  const { values, layout } = await f64Instance(config);
  const inputs = createLowCopyF64Inputs(config.items, config.kernel);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += writeInputsFloat64Array(values, layout, inputs, config.kernel);
  }

  const expected = expectedF64MemorySink(config.kernel, config.items, config.iterations);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runComputeOnly(config) {
  const { view, exports, layout } = await f64Instance(config);
  writeInputs(view, layout, config.items, config.kernel);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += callKernel(exports, config.kernel, layout, config.items);
  }

  const expected = expectedF64ComputeSink(config.kernel, config.items, config.iterations, f64Factor);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runComputeOnlyLowCopy(config) {
  const { values, exports, layout } = await f64Instance(config);
  const inputs = createLowCopyF64Inputs(config.items, config.kernel);
  writeInputsFloat64Array(values, layout, inputs, config.kernel);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += callKernel(exports, config.kernel, layout, config.items);
  }

  const expected = expectedF64ComputeSink(config.kernel, config.items, config.iterations, f64Factor);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runTotal(config) {
  const { view, exports, layout } = await f64Instance(config);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    writeInputs(view, layout, config.items, config.kernel);
    actual += callKernel(exports, config.kernel, layout, config.items);
    actual += checksumMemory(view, layout, config.items, config.kernel);
  }

  const expected =
    expectedF64ResetSink(config.kernel, config.items, config.iterations, f64Factor) +
    expectedF64OutputReadbackSink(config.kernel, config.items, config.iterations, f64Factor);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runTotalLowCopy(config) {
  const { values, exports, layout } = await f64Instance(config);
  const inputs = createLowCopyF64Inputs(config.items, config.kernel);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    writeInputsFloat64Array(values, layout, inputs, config.kernel);
    const scalarResult = callKernel(exports, config.kernel, layout, config.items);
    actual += scalarResult;
    actual += checksumOutputFloat64Array(values, layout, config.items, config.kernel, scalarResult);
  }

  const expected = 2 * expectedF64ResetSink(config.kernel, config.items, config.iterations, f64Factor);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runOutputReadback(config) {
  const { view, exports, layout } = await f64Instance(config);
  writeInputs(view, layout, config.items, config.kernel);
  callKernel(exports, config.kernel, layout, config.items);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += checksumMemory(view, layout, config.items, config.kernel);
  }

  const expected = expectedF64OutputReadbackSink(config.kernel, config.items, config.iterations, f64Factor);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runOutputReadbackLowCopy(config) {
  const { values, exports, layout } = await f64Instance(config);
  const inputs = createLowCopyF64Inputs(config.items, config.kernel);
  writeInputsFloat64Array(values, layout, inputs, config.kernel);
  const scalarResult = callKernel(exports, config.kernel, layout, config.items);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += checksumOutputFloat64Array(values, layout, config.items, config.kernel, scalarResult);
  }

  const expected = expectedF64ResetSink(config.kernel, config.items, config.iterations, f64Factor);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

function runMemoryOnly(config) {
  const layout = requiredBytesFor(config.items);
  const memory = new WebAssembly.Memory({ initial: 1 });
  const view = ensureDataView(memory, layout.totalBytes);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    writeInputs(view, layout, config.items, config.kernel);
    actual += checksumMemory(view, layout, config.items, config.kernel);
  }

  const expected = expectedF64MemorySink(config.kernel, config.items, config.iterations);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function f64Instance(config) {
  const instance = await instantiate(config.wasm);
  const layout = requiredBytesFor(config.items);
  return f64Runtime(instance, layout, config.copyMode);
}

function f64Runtime(instance, layout, copyMode) {
  const memory = instance.exports.memory;
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("f64 benchmark wasm does not export WebAssembly memory");
  }

  const requiredExports = ["axpy_f64", "dot_f64", "sum_f64", "scale_f64"];
  for (const exportName of requiredExports) {
    if (typeof instance.exports[exportName] !== "function") {
      throw new Error(`f64 benchmark wasm does not export ${exportName}`);
    }
  }

  const runtime = { exports: instance.exports, layout };
  if (copyMode === "float64array") {
    return { ...runtime, values: ensureFloat64Array(memory, layout.totalBytes) };
  }
  return { ...runtime, view: ensureDataView(memory, layout.totalBytes) };
}

function callKernel(exports, kernel, layout, len) {
  switch (kernel) {
    case "axpy":
      return exports.axpy_f64(f64Factor, layout.xOffset, layout.yOffset, len);
    case "dot":
      return exports.dot_f64(layout.xOffset, layout.yOffset, len);
    case "sum":
      return exports.sum_f64(layout.xOffset, len);
    case "scale":
      return exports.scale_f64(f64Factor, layout.xOffset, len);
  }
}

function writeInputs(view, layout, len, kernel) {
  let checksum = 0.0;
  for (let index = 0; index < len; index += 1) {
    const x = initialF64X(index);
    view.setFloat64(layout.xOffset + index * 8, x, true);
    checksum += x;
    if (usesF64Y(kernel)) {
      const y = initialF64Y(index);
      view.setFloat64(layout.yOffset + index * 8, y, true);
      checksum += y;
    }
  }
  return checksum;
}

function checksumMemory(view, layout, len, kernel) {
  let checksum = 0.0;
  for (let index = 0; index < len; index += 1) {
    checksum += view.getFloat64(layout.xOffset + index * 8, true);
    if (usesF64Y(kernel)) {
      checksum += view.getFloat64(layout.yOffset + index * 8, true);
    }
  }
  return checksum;
}

function label(config) {
  if (config.label) {
    return config.label;
  }
  if (config.copyMode === "float64array") {
    return `f64-${config.kernel}-ck-wasm-o3-low-copy-${config.mode}`;
  }
  if (config.mode === "setup" || config.mode === "input-marshal" || config.mode === "output-readback") {
    return `f64-${config.kernel}-ck-wasm-o3-${config.mode}`;
  }
  if (config.mode === "memory-only") {
    return `f64-${config.kernel}-wasm-memory-only`;
  }
  return `f64-${config.kernel}-ck-wasm-o3-${config.mode}`;
}
