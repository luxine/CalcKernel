#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import {
  accessSync,
  constants as fsConstants,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const runnerPath = fileURLToPath(import.meta.url);
const optLevels = [0, 3];
const outputFilename = 'wasm-runtime-report.json';

function jsonScalar(value) {
  return typeof value === 'bigint' ? value.toString() : value;
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function hashFile(filePath) {
  return sha256(readFileSync(filePath));
}

function nowNs() {
  return process.hrtime.bigint();
}

function elapsedNs(start) {
  return Number(process.hrtime.bigint() - start);
}

function align(value, alignment) {
  return Math.ceil(value / alignment) * alignment;
}

function usage() {
  return `Usage: node benches/wasm/bench.mjs --ckc <path> [options]

Options:
  --ckc <path>       Explicit path to the ckc executable (required)
  --out <path>       JSON file, or output directory (default: build/wasm-perf/${outputFilename})
  --samples <count>  Measured rounds per case and optimization level (default: 20)
  --emission-samples <count>  Raw emit-wasm measurements per artifact (default: 3)
  --warmup <count>   Warmup rounds per case and optimization level (default: 5)
  --batch <count>    Independent kernel calls in each round (default: 10)
  --size <count>     Elements per memory-kernel call (default: 4096)
  --case <name>      Select a case; may be repeated (default: all cases)
  --wasm-features <profile>  Wasm target profile: baseline or simd128 (default: baseline)
  --help             Show this help

Cases: ${caseDefinitions.map((definition) => definition.name).join(', ')}
Each selected source is compiled at O0 and O3 with unchecked Wasm ABI settings.
`;
}

function parseCount(name, value, minimum) {
  if (!/^[0-9]+$/.test(value ?? '')) {
    throw new Error(`--${name} must be an integer greater than or equal to ${minimum}`);
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum) {
    throw new Error(`--${name} must be an integer greater than or equal to ${minimum}`);
  }
  return parsed;
}

function parseArgs(args) {
  const options = {
    ckc: null,
    out: `build/wasm-perf/${outputFilename}`,
    samples: 20,
    emissionSamples: 3,
    warmup: 5,
    batch: 10,
    size: 4096,
    cases: [],
    wasmFeatures: 'baseline',
    help: false,
  };
  const values = new Map([
    ['--ckc', 'ckc'],
    ['--out', 'out'],
    ['--samples', 'samples'],
    ['--emission-samples', 'emissionSamples'],
    ['--warmup', 'warmup'],
    ['--batch', 'batch'],
    ['--size', 'size'],
    ['--wasm-features', 'wasmFeatures'],
  ]);

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
    if (arg === '--case') {
      const name = args[++index];
      if (!name || name.startsWith('--')) throw new Error('--case requires a case name');
      if (!caseDefinitions.some((definition) => definition.name === name)) {
        throw new Error(`unknown case '${name}'. Available cases: ${caseDefinitions.map((item) => item.name).join(', ')}`);
      }
      if (options.cases.includes(name)) throw new Error(`--case '${name}' was specified more than once`);
      options.cases.push(name);
      continue;
    }
    const key = values.get(arg);
    if (!key) throw new Error(`unknown option '${arg}'`);
    const value = args[++index];
    if (!value || value.startsWith('--')) throw new Error(`${arg} requires a value`);
    if (key === 'samples') options.samples = parseCount(key, value, 1);
    else if (key === 'emissionSamples') options.emissionSamples = parseCount('emission-samples', value, 1);
    else if (key === 'warmup') options.warmup = parseCount(key, value, 0);
    else if (key === 'batch') options.batch = parseCount(key, value, 1);
    else if (key === 'size') options.size = parseCount(key, value, 0);
    else options[key] = value;
  }

  if (options.help) return options;
  if (!['baseline', 'simd128'].includes(options.wasmFeatures)) {
    throw new Error("--wasm-features must be 'baseline' or 'simd128'");
  }
  if (!options.ckc) throw new Error('--ckc is required; pass the ckc executable path explicitly');
  if (options.size > 0x7fffffff) throw new Error('--size must fit the Wasm32 signed length ABI');
  if (options.batch > 0x7fffffff) throw new Error('--batch is too large');
  if (options.cases.length === 0) options.cases = caseDefinitions.map((definition) => definition.name);
  return options;
}

function callSummary(name, returns, outputs = {}) {
  return { name, returns, outputs };
}

function scalarSmallCall() {
  return {
    name: 'scalar-small-call',
    source: 'examples/wasm/calls.ck',
    workspaceBytes: () => 0,
    prepare() {},
    invoke(instance) {
      return instance.exports.calc(17n, 25n);
    },
    expectedCall() {
      return 84n;
    },
    capture(_instance, _workspace, _size, _batch, returns) {
      return callSummary('calc', returns);
    },
    expected(size, batch) {
      return callSummary('calc', Array(batch).fill(84n));
    },
  };
}

function controlFlow() {
  return {
    name: 'control_flow',
    source: 'examples/wasm/control_flow.ck',
    workspaceBytes: () => 0,
    prepare() {},
    invoke(instance, _workspace, size) {
      return instance.exports.sum_to_n(BigInt(size));
    },
    expectedCall(size) {
      const n = BigInt(size);
      return (n * (n - 1n)) / 2n;
    },
    capture(_instance, _workspace, _size, _batch, returns) {
      return callSummary('sum_to_n', returns);
    },
    expected(size, batch) {
      const n = BigInt(size);
      return callSummary('sum_to_n', Array(batch).fill((n * (n - 1n)) / 2n));
    },
  };
}

function branchDiamond() {
  return {
    name: 'branch_diamond',
    source: 'examples/wasm/branch_diamond.ck',
    workspaceBytes: () => 0,
    prepare() {},
    invoke(instance, _workspace, size) {
      return instance.exports.branch_diamond(BigInt(size));
    },
    expectedCall(size) {
      let total = 0n;
      for (let index = 0; index < size; index += 1) {
        const i = BigInt(index);
        total += index % 2 === 0 ? i + 7n : 2n * i + 5n;
        total += index % 3 === 0 ? 13n : 17n;
      }
      return total;
    },
    capture(_instance, _workspace, _size, _batch, returns) {
      return callSummary('branch_diamond', returns);
    },
    expected(size, batch) {
      return callSummary('branch_diamond', Array(batch).fill(this.expectedCall(size)));
    },
  };
}

function nestedControl() {
  return {
    name: 'nested_control',
    source: 'examples/wasm/nested_control.ck',
    workspaceBytes: () => 0,
    prepare() {},
    invoke(instance, _workspace, size) {
      return instance.exports.nested_control(BigInt(size));
    },
    expectedCall(size) {
      let total = 0n;
      for (let outer = 0; outer < size; outer += 1) {
        for (let inner = 0; inner < 10; inner += 1) {
          if (inner === 7) break;
          if ((outer + inner) % 3 === 0) continue;
          total += BigInt(outer + 1) * BigInt(inner + 2);
        }
      }
      return total;
    },
    capture(_instance, _workspace, _size, _batch, returns) {
      return callSummary('nested_control', returns);
    },
    expected(size, batch) {
      return callSummary('nested_control', Array(batch).fill(this.expectedCall(size)));
    },
  };
}

const F64_MAP_EDGE_INPUTS = [
  1 + 2 ** -27, // Paired with the second slot to distinguish separate ops from FMA.
  -0,
  0,
  Number.NaN,
  Number.POSITIVE_INFINITY,
  Number.NEGATIVE_INFINITY,
  Number.MIN_VALUE,
  -Number.MIN_VALUE,
  Number.MAX_VALUE,
  -Number.MAX_VALUE,
  0.5,
  -0.5,
  1.000000000000001,
];

function f64MapArguments(slot) {
  if (slot % 3 === 1) return { factor: 1 - 2 ** -27, bias: -1 };
  if (slot % 3 === 2) return { factor: Number.POSITIVE_INFINITY, bias: 0 };
  return { factor: 1, bias: -0 };
}

function f64Map() {
  const guardValueBefore = 123456.5;
  const guardValueAfter = -789012.5;
  return {
    name: 'f64_map',
    source: 'examples/wasm/f64_map.ck',
    outputTypes: { b: 'f64' },
    workspaceBytes(size, batch) {
      const strideBytes = align((size + 2) * Float64Array.BYTES_PER_ELEMENT, 16);
      return 2 * strideBytes * batch;
    },
    prepare(workspace, size, batch) {
      const strideBytes = align((size + 2) * Float64Array.BYTES_PER_ELEMENT, 16);
      const regionBytes = strideBytes * batch;
      for (let slot = 0; slot < batch; slot += 1) {
        const aBase = workspace.start + slot * strideBytes + Float64Array.BYTES_PER_ELEMENT;
        const bBase = workspace.start + regionBytes + slot * strideBytes + Float64Array.BYTES_PER_ELEMENT;
        workspace.view.setFloat64(aBase - 8, guardValueBefore, true);
        workspace.view.setFloat64(aBase + size * 8, guardValueAfter, true);
        workspace.view.setFloat64(bBase - 8, guardValueBefore, true);
        workspace.view.setFloat64(bBase + size * 8, guardValueAfter, true);
        for (let index = 0; index < size; index += 1) {
          workspace.view.setFloat64(aBase + index * 8, F64_MAP_EDGE_INPUTS[index % F64_MAP_EDGE_INPUTS.length], true);
          workspace.view.setFloat64(bBase + index * 8, -17.25, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const strideBytes = align((size + 2) * Float64Array.BYTES_PER_ELEMENT, 16);
      const regionBytes = strideBytes * batch;
      const aBase = workspace.start + slot * strideBytes + 8;
      const bBase = workspace.start + regionBytes + slot * strideBytes + 8;
      const { factor, bias } = f64MapArguments(slot);
      return instance.exports.map_f64(aBase, size, bBase, size, size, factor, bias);
    },
    expectedCall(size, slot) {
      const { factor, bias } = f64MapArguments(slot);
      return Array.from({ length: size }, (_, index) => (
        F64_MAP_EDGE_INPUTS[index % F64_MAP_EDGE_INPUTS.length] * factor + bias
      ));
    },
    capture(_instance, workspace, size, batch, returns) {
      const strideBytes = align((size + 2) * Float64Array.BYTES_PER_ELEMENT, 16);
      const regionBytes = strideBytes * batch;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const bBase = workspace.start + regionBytes + slot * strideBytes + 8;
        if (!Object.is(workspace.view.getFloat64(bBase - 8, true), guardValueBefore) ||
            !Object.is(workspace.view.getFloat64(bBase + size * 8, true), guardValueAfter)) {
          throw new Error(`f64_map slot ${slot} wrote outside the destination slice`);
        }
        output.push(Array.from({ length: size }, (_, index) => workspace.view.getFloat64(bBase + index * 8, true)));
      }
      return callSummary('map_f64', returns, { b: output });
    },
    expected(size, batch) {
      return callSummary('map_f64', Array(batch).fill(undefined), {
        b: Array.from({ length: batch }, (_, slot) => this.expectedCall(size, slot)),
      });
    },
  };
}

const I32_MAP_EDGE_INPUTS = [
  0x7fffffff,
  -0x80000000,
  -1,
  0,
  0x40000001,
  -0x40000001,
  0x12345678,
  -0x12345678,
  0x6fffffff,
  -0x70000000,
  17,
  -23,
  1,
  -1,
  0x55555555,
  -0x55555556,
];

function i32Map() {
  const factor = 0x6f3a2b17;
  const bias = 0x6123bcde;
  const guardValueBefore = 0x13579bdf;
  const guardValueAfter = -0x2468ace;
  return {
    name: 'i32_map',
    source: 'examples/wasm/i32_map.ck',
    outputTypes: { b: 'i32' },
    workspaceBytes(size, batch) {
      const strideBytes = align((size + 2) * Int32Array.BYTES_PER_ELEMENT, 16);
      return 2 * strideBytes * batch;
    },
    prepare(workspace, size, batch) {
      const strideBytes = align((size + 2) * Int32Array.BYTES_PER_ELEMENT, 16);
      const regionBytes = strideBytes * batch;
      for (let slot = 0; slot < batch; slot += 1) {
        const aBase = workspace.start + slot * strideBytes + Int32Array.BYTES_PER_ELEMENT;
        const bBase = workspace.start + regionBytes + slot * strideBytes + Int32Array.BYTES_PER_ELEMENT;
        workspace.view.setInt32(aBase - 4, guardValueBefore, true);
        workspace.view.setInt32(aBase + size * 4, guardValueAfter, true);
        workspace.view.setInt32(bBase - 4, guardValueBefore, true);
        workspace.view.setInt32(bBase + size * 4, guardValueAfter, true);
        for (let index = 0; index < size; index += 1) {
          workspace.view.setInt32(aBase + index * 4, I32_MAP_EDGE_INPUTS[index % I32_MAP_EDGE_INPUTS.length], true);
          workspace.view.setInt32(bBase + index * 4, -17, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const strideBytes = align((size + 2) * Int32Array.BYTES_PER_ELEMENT, 16);
      const regionBytes = strideBytes * batch;
      const aBase = workspace.start + slot * strideBytes + 4;
      const bBase = workspace.start + regionBytes + slot * strideBytes + 4;
      return instance.exports.map_i32(aBase, size, bBase, size, size, factor, bias);
    },
    expectedCall(size) {
      return Array.from({ length: size }, (_, index) => (
        (Math.imul(I32_MAP_EDGE_INPUTS[index % I32_MAP_EDGE_INPUTS.length], factor) + bias) | 0
      ));
    },
    capture(_instance, workspace, size, batch, returns) {
      const strideBytes = align((size + 2) * Int32Array.BYTES_PER_ELEMENT, 16);
      const regionBytes = strideBytes * batch;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const bBase = workspace.start + regionBytes + slot * strideBytes + 4;
        if (workspace.view.getInt32(bBase - 4, true) !== guardValueBefore ||
            workspace.view.getInt32(bBase + size * 4, true) !== guardValueAfter) {
          throw new Error(`i32_map slot ${slot} wrote outside the destination slice`);
        }
        output.push(Array.from({ length: size }, (_, index) => workspace.view.getInt32(bBase + index * 4, true)));
      }
      return callSummary('map_i32', returns, { b: output });
    },
    expected(size, batch) {
      return callSummary('map_i32', Array(batch).fill(undefined), {
        b: Array.from({ length: batch }, () => this.expectedCall(size)),
      });
    },
  };
}

const U32_COMPARE_INPUTS = [
  0, 1, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff,
  16, 17, 18, 0x12345678, 0xdeadbeef,
];

function u32CompareSelect() {
  const guardBefore = 0x13579bdf;
  const guardAfter = 0xfedcba98;
  const pivotForSlot = (slot) => [0x80000000, 17, 0xffffffff][slot % 3];
  const strideBytes = (size) => align((size + 2) * Uint32Array.BYTES_PER_ELEMENT, 16);
  return {
    name: 'u32_compare_select',
    source: 'examples/wasm/compare_select.ck',
    outputTypes: { b: 'u32' },
    workspaceBytes(size, batch) {
      return 2 * strideBytes(size) * batch;
    },
    prepare(workspace, size, batch) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      for (let slot = 0; slot < batch; slot += 1) {
        const aBase = workspace.start + slot * stride + 4;
        const bBase = workspace.start + regionBytes + slot * stride + 4;
        for (const base of [aBase, bBase]) {
          workspace.view.setUint32(base - 4, guardBefore, true);
          workspace.view.setUint32(base + size * 4, guardAfter, true);
        }
        for (let index = 0; index < size; index += 1) {
          workspace.view.setUint32(aBase + index * 4, U32_COMPARE_INPUTS[index % U32_COMPARE_INPUTS.length], true);
          workspace.view.setUint32(bBase + index * 4, 0x55555555, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      return instance.exports.map_u32_compare_select(
        workspace.start + slot * stride + 4, size,
        workspace.start + regionBytes + slot * stride + 4, size,
        size, pivotForSlot(slot),
      );
    },
    expectedCall(size, slot) {
      const pivot = pivotForSlot(slot);
      return Array.from({ length: size }, (_, index) => {
        const value = U32_COMPARE_INPUTS[index % U32_COMPARE_INPUTS.length];
        return value < pivot ? (value + 1) >>> 0 : (value - 1) >>> 0;
      });
    },
    capture(_instance, workspace, size, batch, returns) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const bBase = workspace.start + regionBytes + slot * stride + 4;
        if (workspace.view.getUint32(bBase - 4, true) !== guardBefore ||
            workspace.view.getUint32(bBase + size * 4, true) !== guardAfter) {
          throw new Error(`u32_compare_select slot ${slot} wrote outside the destination slice`);
        }
        output.push(Array.from({ length: size }, (_, index) => workspace.view.getUint32(bBase + index * 4, true)));
      }
      return callSummary('map_u32_compare_select', returns, { b: output });
    },
    expected(size, batch) {
      return callSummary('map_u32_compare_select', Array(batch).fill(undefined), {
        b: Array.from({ length: batch }, (_, slot) => this.expectedCall(size, slot)),
      });
    },
  };
}

function u32AliasMap() {
  const guardBefore = 0x13579bdf;
  const guardAfter = 0xfedcba98;
  const inputs = [0, 1, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x12345678];
  const strideBytes = (size) => align((size + 2) * Uint32Array.BYTES_PER_ELEMENT, 16);
  return {
    name: 'u32_alias_map',
    source: 'examples/wasm/alias_map.ck',
    outputTypes: { b: 'u32' },
    workspaceBytes(size, batch) {
      return 2 * strideBytes(size) * batch;
    },
    prepare(workspace, size, batch) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      for (let slot = 0; slot < batch; slot += 1) {
        const aBase = workspace.start + slot * stride + 4;
        const bBase = workspace.start + regionBytes + slot * stride + 4;
        for (const base of [aBase, bBase]) {
          workspace.view.setUint32(base - 4, guardBefore, true);
          workspace.view.setUint32(base + size * 4, guardAfter, true);
        }
        for (let index = 0; index < size; index += 1) {
          workspace.view.setUint32(aBase + index * 4, inputs[(index + slot) % inputs.length], true);
          workspace.view.setUint32(bBase + index * 4, 0x55555555, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      return instance.exports.alias_map(
        workspace.start + slot * stride + 4, size,
        workspace.start + regionBytes + slot * stride + 4, size,
        size,
      );
    },
    expectedCall(size, slot) {
      return Array.from({ length: size }, (_, index) => (inputs[(index + slot) % inputs.length] + 1) >>> 0);
    },
    capture(_instance, workspace, size, batch, returns) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const bBase = workspace.start + regionBytes + slot * stride + 4;
        if (workspace.view.getUint32(bBase - 4, true) !== guardBefore ||
            workspace.view.getUint32(bBase + size * 4, true) !== guardAfter) {
          throw new Error(`u32_alias_map slot ${slot} wrote outside the destination slice`);
        }
        output.push(Array.from({ length: size }, (_, index) => workspace.view.getUint32(bBase + index * 4, true)));
      }
      return callSummary('alias_map', returns, { b: output });
    },
    expected(size, batch) {
      return callSummary('alias_map', Array(batch).fill(undefined), {
        b: Array.from({ length: batch }, (_, slot) => this.expectedCall(size, slot)),
      });
    },
  };
}

function u32Reduction(product) {
  const name = product ? 'u32_reduce_product' : 'u32_reduce_sum';
  const exportName = product ? 'product_u32' : 'sum_u32';
  const inputs = [1, 3, 0xffffffff, 0x80000001, 0x7fffffff, 5, 0x12345679];
  const guardBefore = 0x13579bdf;
  const guardAfter = 0xfedcba98;
  const initialForSlot = (slot) => (0x9e3779b9 + slot * 2) >>> 0;
  const strideBytes = (size) => align((size + 2) * Uint32Array.BYTES_PER_ELEMENT, 16);
  return {
    name,
    source: 'examples/wasm/reduction.ck',
    workspaceBytes(size, batch) {
      return strideBytes(size) * batch;
    },
    prepare(workspace, size, batch) {
      for (let slot = 0; slot < batch; slot += 1) {
        const aBase = workspace.start + slot * strideBytes(size) + 4;
        workspace.view.setUint32(aBase - 4, guardBefore, true);
        workspace.view.setUint32(aBase + size * 4, guardAfter, true);
        for (let index = 0; index < size; index += 1) {
          workspace.view.setUint32(aBase + index * 4, inputs[(index + slot) % inputs.length], true);
        }
      }
    },
    invoke(instance, workspace, size, slot) {
      return instance.exports[exportName](
        workspace.start + slot * strideBytes(size) + 4,
        size,
        size,
        initialForSlot(slot),
      ) >>> 0;
    },
    expectedCall(size, slot) {
      let total = initialForSlot(slot);
      for (let index = 0; index < size; index += 1) {
        const value = inputs[(index + slot) % inputs.length];
        total = product ? Math.imul(total, value) >>> 0 : (total + value) >>> 0;
      }
      return total;
    },
    capture(_instance, workspace, size, batch, returns) {
      for (let slot = 0; slot < batch; slot += 1) {
        const aBase = workspace.start + slot * strideBytes(size) + 4;
        if (workspace.view.getUint32(aBase - 4, true) !== guardBefore ||
            workspace.view.getUint32(aBase + size * 4, true) !== guardAfter) {
          throw new Error(`${name} slot ${slot} changed input guards`);
        }
      }
      return callSummary(exportName, returns);
    },
    expected(size, batch) {
      return callSummary(exportName, Array.from({ length: batch }, (_, slot) => this.expectedCall(size, slot)));
    },
  };
}

function u32CursorCopy() {
  const guardBefore = 0x13579bdf;
  const guardAfter = 0xfedcba98;
  const untouched = 0x55555555;
  const inputs = [0, 1, 0x7fffffff, 0x80000000, 0xffffffff, 0x12345678];
  const strideBytes = (size) => align((size + 2) * Uint32Array.BYTES_PER_ELEMENT, 16);
  const startForSlot = (size, slot) => Math.min(slot % 3, size);
  return {
    name: 'u32_cursor_copy',
    source: 'examples/wasm/cursor_copy.ck',
    outputTypes: { dst: 'u32' },
    workspaceBytes(size, batch) {
      return 2 * strideBytes(size) * batch;
    },
    prepare(workspace, size, batch) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      for (let slot = 0; slot < batch; slot += 1) {
        const srcBase = workspace.start + slot * stride + 4;
        const dstBase = workspace.start + regionBytes + slot * stride + 4;
        for (const base of [srcBase, dstBase]) {
          workspace.view.setUint32(base - 4, guardBefore, true);
          workspace.view.setUint32(base + size * 4, guardAfter, true);
        }
        for (let index = 0; index < size; index += 1) {
          workspace.view.setUint32(srcBase + index * 4, inputs[(index + slot) % inputs.length], true);
          workspace.view.setUint32(dstBase + index * 4, untouched, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      return instance.exports.copy_u32(
        workspace.start + regionBytes + slot * stride + 4,
        workspace.start + slot * stride + 4,
        startForSlot(size, slot), size,
      );
    },
    expectedCall(size, slot) {
      const start = startForSlot(size, slot);
      return Array.from({ length: size }, (_, index) =>
        index < start ? untouched : inputs[(index + slot) % inputs.length]);
    },
    capture(_instance, workspace, size, batch, returns) {
      const stride = strideBytes(size);
      const regionBytes = stride * batch;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const dstBase = workspace.start + regionBytes + slot * stride + 4;
        if (workspace.view.getUint32(dstBase - 4, true) !== guardBefore ||
            workspace.view.getUint32(dstBase + size * 4, true) !== guardAfter) {
          throw new Error(`u32_cursor_copy slot ${slot} wrote outside the destination slice`);
        }
        output.push(Array.from({ length: size }, (_, index) => workspace.view.getUint32(dstBase + index * 4, true)));
      }
      return callSummary('copy_u32', returns, { dst: output });
    },
    expected(size, batch) {
      return callSummary('copy_u32', Array(batch).fill(undefined), {
        dst: Array.from({ length: batch }, (_, slot) => this.expectedCall(size, slot)),
      });
    },
  };
}

function u32Fill() {
  const guardBefore = 0x13579bdf;
  const guardAfter = 0xfedcba98;
  const untouched = 0x55555555;
  const fillValue = 0x5a5a5a5a;
  const strideBytes = (size) => align((size + 2) * Uint32Array.BYTES_PER_ELEMENT, 16);
  return {
    name: 'u32_fill',
    source: 'examples/wasm/bulk_fill.ck',
    outputTypes: { dst: 'u32' },
    workloadFor(size, batch) {
      return {
        elements_per_workload: size,
        wasm_calls_per_workload: 1,
        benchmark_workloads_per_sample: batch,
      };
    },
    workspaceBytes: (size, batch) => strideBytes(size) * batch,
    prepare(workspace, size, batch) {
      for (let slot = 0; slot < batch; slot += 1) {
        const base = workspace.start + slot * strideBytes(size) + 4;
        workspace.view.setUint32(base - 4, guardBefore, true);
        workspace.view.setUint32(base + size * 4, guardAfter, true);
        for (let index = 0; index < size; index += 1) {
          workspace.view.setUint32(base + index * 4, untouched, true);
        }
      }
    },
    invoke(instance, workspace, size, slot) {
      const base = workspace.start + slot * strideBytes(size) + 4;
      return instance.exports.fill_u32(base, 0, size);
    },
    capture(_instance, workspace, size, batch, returns) {
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const base = workspace.start + slot * strideBytes(size) + 4;
        if (workspace.view.getUint32(base - 4, true) !== guardBefore ||
            workspace.view.getUint32(base + size * 4, true) !== guardAfter) {
          throw new Error(`u32_fill slot ${slot} wrote outside the destination range`);
        }
        output.push(Array.from({ length: size }, (_, index) =>
          workspace.view.getUint32(base + index * 4, true)));
      }
      return callSummary('fill_u32', returns, { dst: output });
    },
    expected(size, batch) {
      return callSummary('fill_u32', Array(batch).fill(undefined), {
        dst: Array.from({ length: batch }, () => Array(size).fill(fillValue)),
      });
    },
  };
}

function u32FieldOffset() {
  const values = [0, 1, 0x7fffffff, 0x80000000, 0xffffffff, 0x12345678];
  const count = (size) => Math.max(size, 1);
  const strideBytes = (size) => (count(size) + 2) * 16;
  const indexForSlot = (size, slot) => (slot * 17) % count(size);
  const valueAt = (index, slot) => values[(index + slot) % values.length];
  return {
    name: 'u32_field_offset',
    source: 'examples/wasm/field_offset.ck',
    workspaceBytes(size, batch) {
      return strideBytes(size) * batch;
    },
    prepare(workspace, size, batch) {
      for (let slot = 0; slot < batch; slot += 1) {
        const base = workspace.start + slot * strideBytes(size) + 16;
        for (let index = 0; index < count(size); index += 1) {
          workspace.view.setUint32(base + index * 16 + 4, valueAt(index, slot), true);
        }
      }
    },
    invoke(instance, workspace, size, slot) {
      const base = workspace.start + slot * strideBytes(size) + 16;
      return instance.exports.load_aligned(base, indexForSlot(size, slot)) >>> 0;
    },
    expectedCall(size, slot) {
      return valueAt(indexForSlot(size, slot), slot);
    },
    capture(_instance, _workspace, _size, _batch, returns) {
      return callSummary('load_aligned', returns);
    },
    expected(size, batch) {
      return callSummary('load_aligned', Array.from({ length: batch }, (_, slot) => this.expectedCall(size, slot)));
    },
  };
}

function integerToF64(signed) {
  const name = signed ? 'i32_to_f64' : 'u32_to_f64';
  const exportName = signed ? 'map_i32_to_f64' : 'map_u32_to_f64';
  const values = signed
    ? [-0x80000000, -1, 0, 1, 0x7fffffff, -0x40000001, 0x40000001]
    : [0, 1, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x40000001];
  const guardBefore = 123456.5;
  const guardAfter = -789012.5;
  const inputStride = (size) => align((size + 2) * 4, 16);
  const outputStride = (size) => align((size + 2) * 8, 16);
  return {
    name,
    source: 'examples/wasm/cast_map.ck',
    outputTypes: { b: 'f64' },
    workspaceBytes(size, batch) {
      return (inputStride(size) + outputStride(size)) * batch;
    },
    prepare(workspace, size, batch) {
      const inputRegion = inputStride(size) * batch;
      for (let slot = 0; slot < batch; slot += 1) {
        const aBase = workspace.start + slot * inputStride(size) + 4;
        const bBase = workspace.start + inputRegion + slot * outputStride(size) + 8;
        workspace.view.setFloat64(bBase - 8, guardBefore, true);
        workspace.view.setFloat64(bBase + size * 8, guardAfter, true);
        for (let index = 0; index < size; index += 1) {
          const value = values[(index + slot) % values.length];
          if (signed) workspace.view.setInt32(aBase + index * 4, value, true);
          else workspace.view.setUint32(aBase + index * 4, value, true);
          workspace.view.setFloat64(bBase + index * 8, -17.25, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const inputRegion = inputStride(size) * batch;
      return instance.exports[exportName](
        workspace.start + slot * inputStride(size) + 4, size,
        workspace.start + inputRegion + slot * outputStride(size) + 8, size,
        size,
      );
    },
    expectedCall(size, slot) {
      return Array.from({ length: size }, (_, index) => values[(index + slot) % values.length]);
    },
    capture(_instance, workspace, size, batch, returns) {
      const inputRegion = inputStride(size) * batch;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const bBase = workspace.start + inputRegion + slot * outputStride(size) + 8;
        if (workspace.view.getFloat64(bBase - 8, true) !== guardBefore ||
            workspace.view.getFloat64(bBase + size * 8, true) !== guardAfter) {
          throw new Error(`${name} slot ${slot} wrote outside the destination slice`);
        }
        output.push(Array.from({ length: size }, (_, index) => workspace.view.getFloat64(bBase + index * 8, true)));
      }
      return callSummary(exportName, returns, { b: output });
    },
    expected(size, batch) {
      return callSummary(exportName, Array(batch).fill(undefined), {
        b: Array.from({ length: batch }, (_, slot) => this.expectedCall(size, slot)),
      });
    },
  };
}

function f64Sum() {
  return {
    name: 'f64_sum',
    source: 'examples/wasm/f64_sum.ck',
    workspaceBytes: (size, batch) => size * batch * Float64Array.BYTES_PER_ELEMENT,
    prepare(workspace, size, batch) {
      for (let slot = 0; slot < batch; slot += 1) {
        const base = workspace.start + slot * size * 8;
        for (let index = 0; index < size; index += 1) {
          workspace.view.setFloat64(base + index * 8, ((index % 97) - 48) + 0.25, true);
        }
      }
    },
    invoke(instance, workspace, size, slot) {
      return instance.exports.sum_f64(workspace.start + slot * size * 8, size);
    },
    expectedCall(size) {
      let sum = 0;
      for (let index = 0; index < size; index += 1) sum += ((index % 97) - 48) + 0.25;
      return sum;
    },
    capture(_instance, _workspace, _size, _batch, returns) {
      return callSummary('sum_f64', returns);
    },
    expected(size, batch) {
      const sum = this.expectedCall(size);
      return callSummary('sum_f64', Array(batch).fill(sum));
    },
  };
}

function f64Axpy() {
  return {
    name: 'f64_axpy',
    source: 'examples/wasm/f64_axpy.ck',
    outputTypes: { y: 'f64' },
    workspaceBytes: (size, batch) => 2 * size * batch * 8,
    prepare(workspace, size, batch) {
      const regionBytes = size * batch * 8;
      for (let slot = 0; slot < batch; slot += 1) {
        const xBase = workspace.start + slot * size * 8;
        const yBase = workspace.start + regionBytes + slot * size * 8;
        for (let index = 0; index < size; index += 1) {
          workspace.view.setFloat64(xBase + index * 8, (index % 7) - 3, true);
          workspace.view.setFloat64(yBase + index * 8, (index % 5) - 2, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const regionBytes = size * batch * 8;
      return instance.exports.axpy_f64(
        0.5,
        workspace.start + slot * size * 8,
        workspace.start + regionBytes + slot * size * 8,
        size,
      );
    },
    expectedCall(size) {
      let checksum = 0;
      const output = [];
      for (let index = 0; index < size; index += 1) {
        const value = 0.5 * ((index % 7) - 3) + ((index % 5) - 2);
        output.push(value);
        checksum += value;
      }
      return { checksum, output };
    },
    capture(instance, workspace, size, batch, returns) {
      const regionBytes = size * batch * 8;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const values = [];
        const base = workspace.start + regionBytes + slot * size * 8;
        for (let index = 0; index < size; index += 1) {
          values.push(workspace.view.getFloat64(base + index * 8, true));
        }
        output.push(values);
      }
      return callSummary('axpy_f64', returns, { y: output });
    },
    expected(size, batch) {
      const { checksum, output } = this.expectedCall(size);
      return callSummary('axpy_f64', Array(batch).fill(checksum), {
        y: Array.from({ length: batch }, () => output),
      });
    },
  };
}

function pricingSoa() {
  return {
    name: 'pricing_soa',
    source: 'examples/wasm/pricing_soa.ck',
    outputTypes: { out_totals: 'i64' },
    workspaceBytes: (size, batch) => 5 * size * batch * 8,
    prepare(workspace, size, batch) {
      const regionBytes = size * batch * 8;
      for (let slot = 0; slot < batch; slot += 1) {
        for (let index = 0; index < size; index += 1) {
          const offset = slot * size * 8 + index * 8;
          workspace.view.setBigInt64(workspace.start + offset, BigInt(101 + (index % 11)), true);
          workspace.view.setBigInt64(workspace.start + regionBytes + offset, BigInt(1 + (index % 5)), true);
          workspace.view.setBigInt64(workspace.start + 2 * regionBytes + offset, BigInt(index % 9), true);
          workspace.view.setBigInt64(workspace.start + 3 * regionBytes + offset, BigInt(100_000 + (index % 5) * 10_000), true);
          workspace.view.setBigInt64(workspace.start + 4 * regionBytes + offset, -1n, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const regionBytes = size * batch * 8;
      const base = workspace.start + slot * size * 8;
      return instance.exports.pricing_soa(
        base,
        workspace.start + regionBytes + slot * size * 8,
        workspace.start + 2 * regionBytes + slot * size * 8,
        workspace.start + 3 * regionBytes + slot * size * 8,
        workspace.start + 4 * regionBytes + slot * size * 8,
        size,
      );
    },
    expectedCall(size) {
      const totals = [];
      for (let index = 0; index < size; index += 1) {
        const subtotal = BigInt(101 + (index % 11)) * BigInt(1 + (index % 5));
        const afterDiscount = subtotal - BigInt(index % 9);
        const tax = (afterDiscount * BigInt(100_000 + (index % 5) * 10_000)) / 1_000_000n;
        totals.push(afterDiscount + tax);
      }
      return totals;
    },
    capture(instance, workspace, size, batch, returns) {
      const regionBytes = size * batch * 8;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const values = [];
        const base = workspace.start + 4 * regionBytes + slot * size * 8;
        for (let index = 0; index < size; index += 1) {
          values.push(workspace.view.getBigInt64(base + index * 8, true));
        }
        output.push(values);
      }
      return callSummary('pricing_soa', returns, { out_totals: output });
    },
    expected(size, batch) {
      const totals = this.expectedCall(size);
      return callSummary('pricing_soa', Array(batch).fill(0), {
        out_totals: Array.from({ length: batch }, () => totals),
      });
    },
  };
}

function pricingCrossingComparison(scalarCalls) {
  const exportName = scalarCalls ? 'pricing_one' : 'pricing_batch';
  const name = scalarCalls ? 'pricing_one_calls' : 'pricing_batch_call';
  const totalsForSlot = (size, slot) => Array.from({ length: size }, (_, index) => {
    const item = index + slot;
    const subtotal = BigInt(101 + (item % 11)) * BigInt(1 + (item % 5));
    const afterDiscount = subtotal - BigInt(item % 9);
    const tax = (afterDiscount * BigInt(100_000 + (item % 5) * 10_000)) / 1_000_000n;
    return afterDiscount + tax;
  });
  return {
    name,
    source: 'examples/wasm/pricing_batch.ck',
    outputTypes: { out_totals: 'i64' },
    workloadFor(size, batch) {
      return {
        logical_rows_per_workload: size,
        wasm_calls_per_workload: scalarCalls ? size : 1,
        benchmark_workloads_per_sample: batch,
      };
    },
    workspaceBytes: (size, batch) => 5 * size * batch * 8,
    prepare(workspace, size, batch) {
      const regionBytes = size * batch * 8;
      for (let slot = 0; slot < batch; slot += 1) {
        for (let index = 0; index < size; index += 1) {
          const item = index + slot;
          const offset = slot * size * 8 + index * 8;
          workspace.view.setBigInt64(workspace.start + offset, BigInt(101 + (item % 11)), true);
          workspace.view.setBigInt64(workspace.start + regionBytes + offset, BigInt(1 + (item % 5)), true);
          workspace.view.setBigInt64(workspace.start + 2 * regionBytes + offset, BigInt(item % 9), true);
          workspace.view.setBigInt64(workspace.start + 3 * regionBytes + offset, BigInt(100_000 + (item % 5) * 10_000), true);
          workspace.view.setBigInt64(workspace.start + 4 * regionBytes + offset, -1n, true);
        }
      }
    },
    invoke(instance, workspace, size, slot, batch) {
      const regionBytes = size * batch * 8;
      const input = [0, 1, 2, 3].map((region) => workspace.start + region * regionBytes + slot * size * 8);
      const outputBase = workspace.start + 4 * regionBytes + slot * size * 8;
      if (scalarCalls) {
        let last = 0n;
        for (let index = 0; index < size; index += 1) {
          const offset = index * 8;
          last = instance.exports.pricing_one(
            workspace.view.getBigInt64(input[0] + offset, true),
            workspace.view.getBigInt64(input[1] + offset, true),
            workspace.view.getBigInt64(input[2] + offset, true),
            workspace.view.getBigInt64(input[3] + offset, true),
          );
          workspace.view.setBigInt64(outputBase + offset, last, true);
        }
        return last;
      }
      return instance.exports.pricing_batch(...input, outputBase, size);
    },
    capture(_instance, workspace, size, batch, returns) {
      const regionBytes = size * batch * 8;
      const output = [];
      for (let slot = 0; slot < batch; slot += 1) {
        const base = workspace.start + 4 * regionBytes + slot * size * 8;
        output.push(Array.from({ length: size }, (_, index) =>
          workspace.view.getBigInt64(base + index * 8, true)));
      }
      return callSummary(exportName, returns, { out_totals: output });
    },
    expected(size, batch) {
      const outputs = Array.from({ length: batch }, (_, slot) => totalsForSlot(size, slot));
      const returns = scalarCalls
        ? outputs.map((row) => row.at(-1) ?? 0n)
        : Array(batch).fill(0);
      return callSummary(exportName, returns, { out_totals: outputs });
    },
  };
}

function pricingOneCalls() {
  return pricingCrossingComparison(true);
}

function pricingBatchCall() {
  return pricingCrossingComparison(false);
}

const caseDefinitions = [
  scalarSmallCall(),
  controlFlow(),
  branchDiamond(),
  nestedControl(),
  f64Map(),
  i32Map(),
  u32CompareSelect(),
  integerToF64(true),
  integerToF64(false),
  u32AliasMap(),
  u32Reduction(false),
  u32Reduction(true),
  u32CursorCopy(),
  u32Fill(),
  u32FieldOffset(),
  f64Sum(),
  f64Axpy(),
  pricingSoa(),
  pricingOneCalls(),
  pricingBatchCall(),
];

function selectCase(name) {
  return caseDefinitions.find((definition) => definition.name === name);
}

function runCompiler(compilerPath, args) {
  const result = spawnSync(compilerPath, args, { cwd: repoRoot, encoding: 'utf8' });
  if (result.error) throw new Error(`could not run ckc at ${compilerPath}: ${result.error.message}`);
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || '').trim();
    throw new Error(`ckc exited with status ${result.status}${detail ? `: ${detail}` : ''}`);
  }
  return result.stdout.trim();
}

function readU32Leb(bytes, start, limit, description) {
  let value = 0;
  let shift = 0;
  let offset = start;
  for (let index = 0; index < 5; index += 1) {
    if (offset >= limit) throw new Error(`truncated ${description} in Wasm artifact`);
    const byte = bytes[offset++];
    if (index === 4 && (byte & 0xf0) !== 0) throw new Error(`invalid ${description} in Wasm artifact`);
    value |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) return { value: value >>> 0, next: offset };
    shift += 7;
  }
  throw new Error(`invalid ${description} in Wasm artifact`);
}

function decodeUtf8(bytes, description) {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`${description} is not valid UTF-8`);
  }
}

function codeSectionStats(bytes, start, end) {
  const functionCount = readU32Leb(bytes, start, end, 'code function count');
  let offset = functionCount.next;
  let localCount = 0;
  const functionBodyBytes = [];
  for (let functionIndex = 0; functionIndex < functionCount.value; functionIndex += 1) {
    const bodyLength = readU32Leb(bytes, offset, end, 'function body length');
    offset = bodyLength.next;
    functionBodyBytes.push(bodyLength.value);
    const bodyEnd = offset + bodyLength.value;
    if (!Number.isSafeInteger(bodyEnd) || bodyEnd > end) {
      throw new Error('function body extends past the code section');
    }
    const groupCount = readU32Leb(bytes, offset, bodyEnd, 'function local group count');
    offset = groupCount.next;
    if (groupCount.value > bodyEnd - offset) {
      throw new Error('function local declarations extend past the function body');
    }
    for (let groupIndex = 0; groupIndex < groupCount.value; groupIndex += 1) {
      const groupSize = readU32Leb(bytes, offset, bodyEnd, 'function local count');
      offset = groupSize.next;
      if (offset >= bodyEnd) throw new Error('function local type is missing');
      offset += 1; // value type byte
      localCount += groupSize.value;
      if (!Number.isSafeInteger(localCount)) throw new Error('function local count is too large');
    }
    offset = bodyEnd;
  }
  if (offset !== end) throw new Error('Wasm code section has trailing bytes');
  return {
    function_count: functionCount.value,
    function_body_bytes: functionBodyBytes,
    local_count: localCount,
  };
}

function parseWasmTargetMetadata(bytes) {
  if (bytes.byteLength < 8 || !bytes.subarray(0, 8).equals(Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]))) {
    throw new Error('ckc emitted an invalid Wasm header');
  }

  const targetSections = [];
  const sectionPayloadBytes = {};
  let codeSectionBytes = 0;
  let codeStats = { function_count: 0, function_body_bytes: [], local_count: 0 };
  const sectionNames = ['custom', 'type', 'import', 'function', 'table', 'memory', 'global', 'export', 'start', 'element', 'code', 'data'];
  let offset = 8;
  while (offset < bytes.byteLength) {
    const sectionId = bytes[offset++];
    const sectionLength = readU32Leb(bytes, offset, bytes.byteLength, 'section length');
    offset = sectionLength.next;
    const sectionEnd = offset + sectionLength.value;
    if (!Number.isSafeInteger(sectionEnd) || sectionEnd > bytes.byteLength) {
      throw new Error('Wasm section extends past the end of the artifact');
    }
    const sectionName = sectionNames[sectionId] ?? `section_${sectionId}`;
    sectionPayloadBytes[sectionName] = (sectionPayloadBytes[sectionName] ?? 0) + sectionLength.value;

    if (sectionId === 10) {
      codeSectionBytes += sectionLength.value;
      codeStats = codeSectionStats(bytes, offset, sectionEnd);
    }

    if (sectionId === 0) {
      const nameLength = readU32Leb(bytes, offset, sectionEnd, 'custom-section name length');
      const nameEnd = nameLength.next + nameLength.value;
      if (nameEnd > sectionEnd) throw new Error('Wasm custom-section name extends past its section');
      const name = decodeUtf8(bytes.subarray(nameLength.next, nameEnd), 'Wasm custom-section name');
      if (name === 'ck.wasm.target') targetSections.push(bytes.subarray(nameEnd, sectionEnd));
    }
    offset = sectionEnd;
  }

  if (targetSections.length !== 1) {
    throw new Error(`expected exactly one ck.wasm.target custom section; found ${targetSections.length}`);
  }
  const text = decodeUtf8(targetSections[0], 'ck.wasm.target payload');
  let metadata;
  try {
    metadata = JSON.parse(text);
  } catch (error) {
    throw new Error(`ck.wasm.target payload is not valid JSON: ${error.message}`);
  }
  const expectedKeys = ['schema', 'target', 'features', 'profile_sha256'];
  if (metadata === null || typeof metadata !== 'object' || Array.isArray(metadata) ||
      JSON.stringify(Object.keys(metadata)) !== JSON.stringify(expectedKeys) ||
      JSON.stringify(metadata) !== text) {
    throw new Error('ck.wasm.target payload must use the canonical deterministic JSON schema');
  }
  if (metadata.schema !== 2 || metadata.target !== 'wasm32' ||
      !['baseline', 'simd128'].includes(metadata.features) ||
      typeof metadata.profile_sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(metadata.profile_sha256)) {
    throw new Error('ck.wasm.target payload has invalid schema, target, feature, or profile digest values');
  }
  return {
    metadata,
    wasm_stats: {
      module_bytes: bytes.byteLength,
      section_payload_bytes: sectionPayloadBytes,
      code_section_bytes: codeSectionBytes,
      ...codeStats,
    },
  };
}

function parseKirProfileDigest(output) {
  const header = output.split(/\r?\n/, 1)[0];
  const fields = header.trim().split(/\s+/);
  if (fields.shift() !== 'kir-v3') throw new Error('emit-kir did not return a KIR v3 profile header');
  const values = new Map();
  for (const field of fields) {
    const separator = field.indexOf('=');
    if (separator <= 0) continue;
    const key = field.slice(0, separator);
    if (values.has(key)) throw new Error(`emit-kir profile header repeats '${key}'`);
    values.set(key, field.slice(separator + 1));
  }
  if (values.get('consumer') !== 'wasm' || values.get('overflow') !== 'unchecked' ||
      values.get('bounds') !== 'unchecked') {
    throw new Error('emit-kir profile header does not describe the requested Wasm modes');
  }
  const digest = values.get('profile-sha256');
  if (!digest || !/^[a-f0-9]{64}$/.test(digest)) {
    throw new Error('emit-kir profile header has no valid profile-sha256 digest');
  }
  return digest;
}

function compilerIdentity(compilerPath) {
  let resolved;
  try {
    resolved = realpathSync(compilerPath);
    accessSync(resolved, fsConstants.X_OK);
    if (!statSync(resolved).isFile()) throw new Error('path is not a file');
  } catch (error) {
    throw new Error(`--ckc must name an executable file: ${error.message}`);
  }
  const versionResult = spawnSync(resolved, ['--version'], { cwd: repoRoot, encoding: 'utf8' });
  if (versionResult.error || versionResult.status !== 0) {
    throw new Error(`could not identify ckc version at ${resolved}`);
  }
  return {
    path: resolved,
    sha256: hashFile(resolved),
    version: (versionResult.stdout || versionResult.stderr || '').trim(),
  };
}

function runnerIdentity() {
  return {
    node_version: process.version,
    v8_version: process.versions.v8,
    platform: process.platform,
    arch: process.arch,
    os_release: os.release(),
    hostname: os.hostname(),
    cpu_model: os.cpus()[0]?.model ?? 'unknown',
    logical_cpus: os.cpus().length,
    executable: process.execPath,
    runner_path: runnerPath,
    runner_sha256: hashFile(runnerPath),
  };
}

function outputPath(outOption) {
  const resolved = path.resolve(repoRoot, outOption);
  if (resolved.toLowerCase().endsWith('.json')) return resolved;
  return path.join(resolved, outputFilename);
}

function persistArtifacts(variants, reportFile) {
  const reportDirectory = path.dirname(reportFile);
  const artifactDirectory = path.join(reportDirectory, 'artifacts');
  mkdirSync(artifactDirectory, { recursive: true });
  for (const variant of variants) {
    const filename = `${variant.definition.name}-O${variant.optLevel}-${variant.artifactInfo.artifact.sha256}.wasm`;
    const artifactPath = path.join(artifactDirectory, filename);
    if (!existsSync(artifactPath) || hashFile(artifactPath) !== variant.artifactInfo.artifact.sha256) {
      const tempArtifactPath = `${artifactPath}.${process.pid}.${randomUUID()}.tmp`;
      try {
        writeFileSync(tempArtifactPath, variant.artifactInfo.bytes, { flag: 'wx' });
        renameSync(tempArtifactPath, artifactPath);
      } catch (error) {
        rmSync(tempArtifactPath, { force: true });
        throw error;
      }
    }
    variant.artifactInfo.artifact.path = `artifacts/${filename}`;
  }
}

export function writeReportAtomically(reportFile, contents, rename = renameSync) {
  mkdirSync(path.dirname(reportFile), { recursive: true });
  const tempReport = `${reportFile}.${process.pid}.${randomUUID()}.tmp`;
  try {
    writeFileSync(tempReport, contents, { flag: 'wx' });
    rename(tempReport, reportFile);
  } catch (error) {
    rmSync(tempReport, { force: true });
    throw error;
  }
}

function prepareArtifact(compilerPath, definition, optLevel, wasmFeatures, emissionSampleCount, tempDir) {
  const sourcePath = path.join(repoRoot, definition.source);
  const identityStart = nowNs();
  const kirHeader = runCompiler(compilerPath, [
    'emit-kir',
    sourcePath,
    '--consumer', 'wasm',
    '--overflow', 'unchecked',
    '--bounds', 'unchecked',
    '--opt-level', String(optLevel),
    '--wasm-features', wasmFeatures,
  ]);
  const profileDigest = parseKirProfileDigest(kirHeader);
  const profileEvidenceNs = elapsedNs(identityStart);

  const emissionSamplesNs = [];
  const emissionOutputs = [];
  let bytes;
  let parsedArtifact;
  for (let sample = 0; sample < emissionSampleCount; sample += 1) {
    const artifactPath = path.join(tempDir, `${definition.name}-O${optLevel}-emission-${sample}.wasm`);
    const emissionStart = nowNs();
    runCompiler(compilerPath, [
      'emit-wasm',
      sourcePath,
      '--out', artifactPath,
      '--overflow', 'unchecked',
      '--bounds', 'unchecked',
      '--opt-level', String(optLevel),
      '--wasm-features', wasmFeatures,
    ]);
    emissionSamplesNs.push(elapsedNs(emissionStart));
    bytes = readFileSync(artifactPath);
    parsedArtifact = parseWasmTargetMetadata(bytes);
    if (parsedArtifact.metadata.features !== wasmFeatures) {
      throw new Error(`ck.wasm.target feature '${parsedArtifact.metadata.features}' does not match requested '${wasmFeatures}'`);
    }
    if (parsedArtifact.metadata.profile_sha256 !== profileDigest) {
      throw new Error('ck.wasm.target profile digest does not match the independently emitted KIR profile digest');
    }
    emissionOutputs.push({ bytes: bytes.byteLength, sha256: sha256(bytes) });
  }
  const sortedEmissionSamples = [...emissionSamplesNs].sort((left, right) => left - right);
  const emissionNs = sortedEmissionSamples[Math.floor(sortedEmissionSamples.length / 2)];
  return {
    source: {
      path: definition.source,
      sha256: hashFile(sourcePath),
    },
    bytes,
    artifact: {
      bytes: bytes.byteLength,
      sha256: sha256(bytes),
      target_metadata: parsedArtifact.metadata,
      wasm_stats: parsedArtifact.wasm_stats,
      emission_outputs: emissionOutputs,
    },
    emissionNs,
    emissionSamplesNs,
    profileEvidenceNs,
  };
}

function memoryWorkspace(instance, definition, size, batch) {
  const exports = instance.exports;
  const bytesNeeded = definition.workspaceBytes(size, batch);
  if (bytesNeeded === 0) return { memory: null, start: 0, view: null };
  const memory = exports.memory;
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error(`${definition.name}: expected exported WebAssembly memory`);
  }
  const heapBaseExport = exports.__ck_heap_base;
  if (!(heapBaseExport instanceof WebAssembly.Global)) {
    throw new Error(`${definition.name}: expected exported __ck_heap_base WebAssembly.Global`);
  }
  const heapBase = Number(heapBaseExport.value);
  if (!Number.isSafeInteger(heapBase) || heapBase < 0) {
    throw new Error(`${definition.name}: invalid __ck_heap_base export`);
  }
  const start = align(heapBase, 16);
  const end = start + bytesNeeded;
  if (!Number.isSafeInteger(end) || end > 0x7fffffff) {
    throw new Error(`${definition.name}: requested workspace exceeds the Wasm32 address range`);
  }
  if (end > memory.buffer.byteLength) {
    const pageBytes = 64 * 1024;
    const pages = Math.ceil((end - memory.buffer.byteLength) / pageBytes);
    memory.grow(pages);
  }
  return { memory, start, view: new DataView(memory.buffer) };
}

function outputBytes(values, type) {
  const elementBytes = type === 'i32' || type === 'u32' ? 4 : 8;
  const byteLength = values.length * elementBytes;
  const buffer = Buffer.allocUnsafe(byteLength);
  const view = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  for (let index = 0; index < values.length; index += 1) {
    if (type === 'f64') view.setFloat64(index * 8, values[index], true);
    else if (type === 'i32') view.setInt32(index * 4, values[index], true);
    else if (type === 'u32') view.setUint32(index * 4, values[index], true);
    else view.setBigInt64(index * 8, values[index], true);
  }
  return buffer;
}

function summarizeResult(result, outputTypes = {}) {
  const outputs = {};
  for (const [name, slots] of Object.entries(result.outputs)) {
    const flat = slots.flat();
    outputs[name] = {
      slots: slots.length,
      elements_per_slot: slots[0]?.length ?? 0,
      sha256: sha256(outputBytes(flat, outputTypes[name])),
    };
  }
  return {
    function: result.name,
    return_values: result.returns.map(jsonScalar),
    outputs,
  };
}

function assertCorrectness(definition, expected, actual) {
  try {
    if (JSON.stringify(summarizeResult(actual, definition.outputTypes)) !==
        JSON.stringify(summarizeResult(expected, definition.outputTypes))) {
      throw new Error('result summaries differ');
    }
    if (actual.returns.length !== expected.returns.length ||
        actual.returns.some((value, index) => !Object.is(value, expected.returns[index]))) {
      throw new Error('return values differ');
    }
    const actualNames = Object.keys(actual.outputs).sort();
    const expectedNames = Object.keys(expected.outputs).sort();
    if (JSON.stringify(actualNames) !== JSON.stringify(expectedNames)) throw new Error('output names differ');
    for (const name of actualNames) {
      const actualSlots = actual.outputs[name];
      const expectedSlots = expected.outputs[name];
      if (actualSlots.length !== expectedSlots.length) throw new Error(`${name} slot count differs`);
      for (let slot = 0; slot < actualSlots.length; slot += 1) {
        if (actualSlots[slot].length !== expectedSlots[slot].length) throw new Error(`${name} length differs`);
        for (let index = 0; index < actualSlots[slot].length; index += 1) {
          if (!Object.is(actualSlots[slot][index], expectedSlots[slot][index])) {
            throw new Error(`${name}[${slot}][${index}] differs`);
          }
        }
      }
    }
  } catch (error) {
    const expectedSummary = summarizeResult(expected, definition.outputTypes);
    const actualSummary = summarizeResult(actual, definition.outputTypes);
    throw new Error(
      `correctness mismatch for ${definition.name}: ${error.message}; expected ${JSON.stringify(expectedSummary)}, actual ${JSON.stringify(actualSummary)}`,
    );
  }
}

function invocationValues(definition, instance, workspace, size, batch) {
  const returns = [];
  for (let slot = 0; slot < batch; slot += 1) {
    returns.push(definition.invoke(instance, workspace, size, slot, batch));
  }
  return returns;
}

function captureResult(definition, instance, workspace, size, batch, returns) {
  return definition.capture(instance, workspace, size, batch, returns);
}

function makeResultRow(definition, optLevel, artifactInfo, timings, samples, correctness, module, size, batch) {
  return {
    case: definition.name,
    opt_level: optLevel,
    ...(definition.workloadFor ? { workload: definition.workloadFor(size, batch) } : {}),
    source: artifactInfo.source,
    correctness: {
      status: 'passed',
      expected: summarizeResult(correctness.expected, definition.outputTypes),
      actual: summarizeResult(correctness.actual, definition.outputTypes),
    },
    artifact: {
      ...artifactInfo.artifact,
      imports: WebAssembly.Module.imports(module),
      exports: WebAssembly.Module.exports(module),
    },
    timings_ns: timings,
    samples,
  };
}

async function preflight(definition, optLevel, artifactInfo, size, batch) {
  let module;
  const moduleCompileStart = nowNs();
  try {
    module = await WebAssembly.compile(artifactInfo.bytes);
  } catch (error) {
    throw new Error(`${definition.name} O${optLevel}: Node/V8 could not compile emitted Wasm: ${error.message}`);
  }
  const moduleCompileFirstInProcessNs = elapsedNs(moduleCompileStart);
  let instance;
  const instantiateStart = nowNs();
  try {
    instance = new WebAssembly.Instance(module);
  } catch (error) {
    throw new Error(`${definition.name} O${optLevel}: Node/V8 could not instantiate emitted Wasm: ${error.message}`);
  }
  const instantiateFirstInProcessNs = elapsedNs(instantiateStart);
  const workspace = memoryWorkspace(instance, definition, size, batch);
  const expected = definition.expected(size, batch);
  definition.prepare(workspace, size, batch);
  const returns = invocationValues(definition, instance, workspace, size, batch);
  const actual = captureResult(definition, instance, workspace, size, batch, returns);
  assertCorrectness(definition, expected, actual);
  return {
    module,
    instance,
    expected,
    timings_ns: {
      module_compile_first_in_process: moduleCompileFirstInProcessNs,
      instantiate_first_in_process: instantiateFirstInProcessNs,
    },
  };
}

async function measureVariant(definition, optLevel, artifactInfo, preflightResult, size, batch, warmupCount, sampleCount) {
  const { module, instance, expected } = preflightResult;
  const workspace = memoryWorkspace(instance, definition, size, batch);
  const timings = {
    ck_emission: artifactInfo.emissionNs,
    ck_kir_profile_evidence: artifactInfo.profileEvidenceNs,
    ...preflightResult.timings_ns,
  };
  const samples = {
    ck_emission_raw_ns: artifactInfo.emissionSamplesNs,
    warmup: [],
    kernel_ns: [],
    kernel_ns_per_call: [],
    host_preparation_ns: [],
    readback_ns: [],
    round_end_to_end_ns: [],
  };

  const runRound = (measured) => {
    const endToEndStart = nowNs();

    const preparationStart = nowNs();
    definition.prepare(workspace, size, batch);
    const preparationNs = elapsedNs(preparationStart);

    const kernelStart = nowNs();
    const returns = invocationValues(definition, instance, workspace, size, batch);
    const kernelNs = elapsedNs(kernelStart);

    const readbackStart = nowNs();
    const actual = captureResult(definition, instance, workspace, size, batch, returns);
    const readbackNs = elapsedNs(readbackStart);
    const endToEndNs = elapsedNs(endToEndStart);
    assertCorrectness(definition, expected, actual);

    if (measured) {
      samples.kernel_ns.push(kernelNs);
      samples.kernel_ns_per_call.push(kernelNs / batch);
      samples.host_preparation_ns.push(preparationNs);
      samples.readback_ns.push(readbackNs);
      samples.round_end_to_end_ns.push(endToEndNs);
    } else {
      samples.warmup.push(endToEndNs);
    }
  };

  for (let index = 0; index < warmupCount; index += 1) runRound(false);
  for (let index = 0; index < sampleCount; index += 1) runRound(true);

  definition.prepare(workspace, size, batch);
  const finalReturns = invocationValues(definition, instance, workspace, size, batch);
  const finalActual = captureResult(definition, instance, workspace, size, batch, finalReturns);
  assertCorrectness(definition, expected, finalActual);

  return makeResultRow(
    definition,
    optLevel,
    artifactInfo,
    timings,
    samples,
    { expected, actual: finalActual },
    module,
    size,
    batch,
  );
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    process.stdout.write(usage());
    return;
  }

  const compiler = compilerIdentity(options.ckc);
  const runner = runnerIdentity();
  const selectedCases = options.cases.map(selectCase);
  const reportFile = outputPath(options.out);
  const tempDir = mkdtempSync(path.join(os.tmpdir(), 'ck-wasm-bench-'));
  try {
    const variants = [];
    for (const definition of selectedCases) {
      for (const optLevel of optLevels) {
        const artifactInfo = prepareArtifact(
          compiler.path,
          definition,
          optLevel,
          options.wasmFeatures,
          options.emissionSamples,
          tempDir,
        );
        const checked = await preflight(definition, optLevel, artifactInfo, options.size, options.batch);
        variants.push({ definition, optLevel, artifactInfo, checked });
      }
    }

    persistArtifacts(variants, reportFile);

    // No runtime measurements start until every selected O0/O3 artifact passed correctness.
    const results = [];
    for (const variant of variants) {
      results.push(await measureVariant(
        variant.definition,
        variant.optLevel,
        variant.artifactInfo,
        variant.checked,
        options.size,
        options.batch,
        options.warmup,
        options.samples,
      ));
    }

    const report = {
      schema_version: 1,
      generated_at: new Date().toISOString(),
      observation_scope: 'Node.js/V8 observations. module_compile_first_in_process and instantiate_first_in_process measure the first compile and instance for each artifact in this process. round_end_to_end_ns measures prepare, kernel calls, and readback on the warmed instance; it excludes emission, compile, instantiation, and memory growth.',
      identity: {
        compiler,
        runner,
        runner_sha256: runner.runner_sha256,
      },
      configuration: {
        samples: options.samples,
        emission_samples: options.emissionSamples,
        warmup: options.warmup,
        batch: options.batch,
        size: options.size,
        opt_levels: optLevels,
        cases: options.cases,
        compiler_flags: ['--overflow', 'unchecked', '--bounds', 'unchecked'],
        wasm_features: options.wasmFeatures,
      },
      results,
    };
    mkdirSync(path.dirname(reportFile), { recursive: true });
    writeReportAtomically(reportFile, `${JSON.stringify(report, null, 2)}\n`);
    process.stdout.write(`OK: wrote ${reportFile}\n`);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main().catch((error) => {
    process.stderr.write(`wasm benchmark error: ${error.message}\n`);
    process.exitCode = 1;
  });
}
