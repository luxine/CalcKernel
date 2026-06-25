import { readFileSync } from "node:fs";
import { assertWithinTolerance } from "../lib/f64-correctness.mjs";
import {
  expectedF64ComputeSink,
  expectedF64MemorySink,
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
    case "compute-only":
      return runComputeOnly(config);
    case "total":
      return runTotal(config);
    case "memory-only":
      return runMemoryOnly(config);
  }
}

function requireMode(value) {
  if (value === "compute-only" || value === "total" || value === "memory-only") {
    return value;
  }
  throw new Error("--mode must be compute-only, total, or memory-only");
}

async function instantiate(path) {
  const bytes = readFileSync(path);
  const { instance } = await WebAssembly.instantiate(bytes);
  return instance;
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

async function runTotal(config) {
  const { view, exports, layout } = await f64Instance(config);

  let actual = 0.0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    writeInputs(view, layout, config.items, config.kernel);
    actual += callKernel(exports, config.kernel, layout, config.items);
  }

  const expected = expectedF64ResetSink(config.kernel, config.items, config.iterations, f64Factor);
  assertWithinTolerance(actual, expected, label(config));
  return `${label(config)} items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

function runMemoryOnly(config) {
  const layout = requiredBytesFor(config.items);
  const memory = new WebAssembly.Memory({ initial: 1 });
  const view = ensureMemory(memory, layout.totalBytes);

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

  const layout = requiredBytesFor(config.items);
  return { view: ensureMemory(memory, layout.totalBytes), exports: instance.exports, layout };
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

function requiredBytesFor(len) {
  const elementSize = 8;
  const xOffset = 0;
  const yOffset = alignTo(xOffset + len * elementSize, 8);

  return {
    xOffset,
    yOffset,
    totalBytes: yOffset + len * elementSize
  };
}

function alignTo(value, align) {
  return Math.ceil(value / align) * align;
}

function ensureMemory(memory, requiredBytes) {
  const pageSize = 64 * 1024;
  const currentPages = Math.ceil(memory.buffer.byteLength / pageSize);
  const requiredPages = Math.ceil(requiredBytes / pageSize);

  if (requiredPages > currentPages) {
    memory.grow(requiredPages - currentPages);
  }

  return new DataView(memory.buffer);
}

function writeInputs(view, layout, len, kernel) {
  for (let index = 0; index < len; index += 1) {
    view.setFloat64(layout.xOffset + index * 8, initialF64X(index), true);
    if (usesF64Y(kernel)) {
      view.setFloat64(layout.yOffset + index * 8, initialF64Y(index), true);
    }
  }
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
  if (config.mode === "memory-only") {
    return `f64-${config.kernel}-wasm-memory-only`;
  }
  return `f64-${config.kernel}-ik-wasm-o3-${config.mode}`;
}
