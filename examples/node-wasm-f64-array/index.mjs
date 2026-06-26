import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const f64SizeBytes = 8;

const exampleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(exampleDir, "..", "..");

function defaultWasmPath() {
  return join(repoRoot, "build", "f64_array.wasm");
}

export function byteOffsetToF64Index(byteOffset) {
  if (!Number.isInteger(byteOffset) || byteOffset < 0) {
    throw new Error("ptr<f64> byte offset must be a non-negative integer");
  }
  if (byteOffset % f64SizeBytes !== 0) {
    throw new Error("ptr<f64> byte offset must be 8-byte aligned");
  }
  return byteOffset / f64SizeBytes;
}

export function f64ByteOffset(index) {
  if (!Number.isInteger(index) || index < 0) {
    throw new Error("Float64Array index must be a non-negative integer");
  }
  return index * f64SizeBytes;
}

export function f64View(memory, requiredBytes = 0) {
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("generated module did not export WebAssembly memory");
  }

  const pageSize = 64 * 1024;
  const requiredPages = Math.ceil(requiredBytes / pageSize);
  const currentPages = Math.ceil(memory.buffer.byteLength / pageSize);

  if (requiredPages > currentPages) {
    memory.grow(requiredPages - currentPages);
  }

  // If memory.grow ran, old typed-array views are detached. Always create the
  // view after any growth and recreate it after future host-side growth.
  return new Float64Array(memory.buffer);
}

export function withinTolerance(actual, expected, absTol = 1e-9, relTol = 1e-9) {
  const diff = Math.abs(actual - expected);
  const scale = Math.max(1, Math.abs(actual), Math.abs(expected));
  return diff <= absTol || diff <= relTol * scale;
}

export async function instantiateWasm(wasmPath = defaultWasmPath()) {
  if (!existsSync(wasmPath)) {
    throw new Error(
      `WASM file not found: ${wasmPath}\n` +
        "Generate it with `ckc emit-wasm examples/node-wasm-f64-array/f64_array.ck --out build/f64_array.wasm -O3`."
    );
  }

  const bytes = readFileSync(wasmPath);
  const { instance } = await WebAssembly.instantiate(bytes);
  return instance;
}

export async function runExample(options = {}) {
  const wasmPath = options.wasmPath ?? defaultWasmPath();
  const instance = await instantiateWasm(wasmPath);
  const { memory, axpy_f64: axpyF64 } = instance.exports;

  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("generated module did not export memory");
  }
  if (typeof axpyF64 !== "function") {
    throw new Error("generated module did not export axpy_f64");
  }

  const len = 4;
  const factor = 1.25;
  const xOffset = 0;
  const yOffset = f64ByteOffset(8);
  const requiredBytes = yOffset + len * f64SizeBytes;
  const values = f64View(memory, requiredBytes);
  const xIndex = byteOffsetToF64Index(xOffset);
  const yIndex = byteOffsetToF64Index(yOffset);

  values.set([1.0, 2.0, 3.0, 4.0], xIndex);
  values.set([0.5, 1.25, 1.25, 2.0], yIndex);

  const checksum = axpyF64(factor, xOffset, yOffset, len);
  const y = Array.from(values.subarray(yIndex, yIndex + len));
  const expected = [1.75, 3.75, 5.0, 7.0];
  const expectedChecksum = expected.reduce((sum, value) => sum + value, 0.0);

  if (!withinTolerance(checksum, expectedChecksum)) {
    throw new Error(`unexpected checksum: expected ${expectedChecksum}, got ${checksum}`);
  }
  for (let index = 0; index < expected.length; index += 1) {
    if (!withinTolerance(y[index], expected[index])) {
      throw new Error(`unexpected y[${index}]: expected ${expected[index]}, got ${y[index]}`);
    }
  }

  return { checksum, expectedChecksum, y, expected, xOffset, yOffset, xIndex, yIndex };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const wasmArgIndex = process.argv.indexOf("--wasm");
  const wasmPath = wasmArgIndex === -1 ? defaultWasmPath() : process.argv[wasmArgIndex + 1];
  if (wasmArgIndex !== -1 && !wasmPath) {
    throw new Error("--wasm requires a path");
  }

  const result = await runExample({ wasmPath });
  console.log(`OK checksum=${result.checksum} y=${result.y.join(",")}`);
}
