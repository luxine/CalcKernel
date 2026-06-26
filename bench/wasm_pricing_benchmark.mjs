import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const sizes = [100, 1_000, 10_000, 100_000];
const pageSize = 64 * 1024;
const itemSize = 32;
const outElementSize = 8;
const wasmPath = fileURLToPath(new URL("../build/pricing.wasm", import.meta.url));

function alignTo(value, align) {
  return Math.ceil(value / align) * align;
}

function formatMs(ns) {
  return (Number(ns) / 1_000_000).toFixed(3);
}

function requiredBytesFor(len) {
  const itemsOffset = 0;
  const itemBytes = len * itemSize;
  const outOffset = alignTo(itemsOffset + itemBytes, 8);
  const outBytes = len * outElementSize;

  return {
    itemsOffset,
    outOffset,
    totalBytes: outOffset + outBytes
  };
}

function ensureMemory(memory, requiredBytes) {
  const currentPages = Math.ceil(memory.buffer.byteLength / pageSize);
  const requiredPages = Math.ceil(requiredBytes / pageSize);

  if (requiredPages > currentPages) {
    memory.grow(requiredPages - currentPages);
  }

  return new DataView(memory.buffer);
}

function writeItems(view, offset, len) {
  for (let i = 0; i < len; i += 1) {
    const base = offset + i * itemSize;
    view.setBigInt64(base + 0, BigInt(1000 + (i % 997)), true);
    view.setBigInt64(base + 8, BigInt(1 + (i % 9)), true);
    view.setBigInt64(base + 16, BigInt(i % 113), true);
    view.setBigInt64(base + 24, BigInt(50_000 + (i % 17) * 2500), true);
  }
}

function clearOutput(view, offset, len) {
  for (let i = 0; i < len; i += 1) {
    view.setBigInt64(offset + i * outElementSize, 0n, true);
  }
}

function checksum(view, offset, len) {
  let total = 0n;

  for (let i = 0; i < len; i += 1) {
    total += view.getBigInt64(offset + i * outElementSize, true);
  }

  return total;
}

const bytes = await readFile(wasmPath).catch((error) => {
  if (error && error.code === "ENOENT") {
    throw new Error(`Missing ${wasmPath}. Generate it with: pnpm ckc emit-wasm examples/pricing.ck --out build/pricing.wasm`);
  }

  throw error;
});

const { instance } = await WebAssembly.instantiate(bytes);
const memory = instance.exports.memory;
const calcItems = instance.exports.calc_items;

if (!(memory instanceof WebAssembly.Memory)) {
  throw new Error("pricing.wasm does not export WebAssembly memory");
}

if (typeof calcItems !== "function") {
  throw new Error("pricing.wasm does not export calc_items");
}

for (const len of sizes) {
  const { itemsOffset, outOffset, totalBytes } = requiredBytesFor(len);
  const view = ensureMemory(memory, totalBytes);

  writeItems(view, itemsOffset, len);

  if (calcItems(itemsOffset, len, outOffset) !== 0) {
    throw new Error(`warmup calc_items failed for ${len} items`);
  }

  clearOutput(view, outOffset, len);

  const start = process.hrtime.bigint();
  const status = calcItems(itemsOffset, len, outOffset);
  const elapsed = process.hrtime.bigint() - start;

  if (status !== 0) {
    throw new Error(`calc_items returned ${status} for ${len} items`);
  }

  console.log(`${len} items: ${formatMs(elapsed)} ms (checksum=${checksum(view, outOffset, len)})`);
}
