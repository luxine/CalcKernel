import { readFileSync } from "node:fs";

const config = parseArgs(process.argv.slice(2));
const bytes = readFileSync(config.wasm);
const { instance } = await WebAssembly.instantiate(bytes);
const memory = instance.exports.memory;
const calcItems = instance.exports.calc_items;

if (!(memory instanceof WebAssembly.Memory)) {
  throw new Error("pricing.wasm does not export WebAssembly memory");
}
if (typeof calcItems !== "function") {
  throw new Error("pricing.wasm does not export calc_items");
}

const layout = requiredBytesFor(config.items);
const view = ensureMemory(memory, layout.totalBytes);
writeItems(view, layout.itemsOffset, config.items);

for (let iteration = 0; iteration < config.iterations; iteration += 1) {
  const status = calcItems(layout.itemsOffset, config.items, layout.outOffset);
  if (status !== 0) {
    throw new Error(`calc_items returned ${status}`);
  }
}

const actual = checksum(view, layout.outOffset, config.items);
const expected = expectedChecksum(config.items);
if (actual !== expected) {
  throw new Error(`checksum mismatch: expected ${expected}, got ${actual}`);
}

console.log(`pricing-wasm-unchecked items=${config.items} iterations=${config.iterations} checksum=${actual}`);

function parseArgs(argv) {
  const config = { items: 100000, iterations: 1000, wasm: "build/perf/generated/pricing.wasm" };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--items") {
      config.items = positiveInteger(argv[index + 1], "--items");
      index += 1;
    } else if (arg === "--iterations") {
      config.iterations = positiveInteger(argv[index + 1], "--iterations");
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

function positiveInteger(value, flag) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${flag} must be a positive integer`);
  }
  return parsed;
}

function alignTo(value, align) {
  return Math.ceil(value / align) * align;
}

function requiredBytesFor(len) {
  const itemSize = 32;
  const outElementSize = 8;
  const itemsOffset = 0;
  const outOffset = alignTo(itemsOffset + len * itemSize, 8);

  return {
    itemsOffset,
    outOffset,
    totalBytes: outOffset + len * outElementSize
  };
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

function writeItems(view, offset, len) {
  const itemSize = 32;
  for (let i = 0; i < len; i += 1) {
    const base = offset + i * itemSize;
    view.setBigInt64(base + 0, BigInt(1000 + (i % 997)), true);
    view.setBigInt64(base + 8, BigInt(1 + (i % 9)), true);
    view.setBigInt64(base + 16, BigInt(i % 113), true);
    view.setBigInt64(base + 24, BigInt(50_000 + (i % 17) * 2500), true);
  }
}

function expectedChecksum(len) {
  let total = 0n;

  for (let i = 0; i < len; i += 1) {
    const price = BigInt(1000 + (i % 997));
    const qty = BigInt(1 + (i % 9));
    const discount = BigInt(i % 113);
    const taxRatePpm = BigInt(50_000 + (i % 17) * 2500);
    const subtotal = price * qty;
    const afterDiscount = subtotal - discount;
    const tax = (afterDiscount * taxRatePpm) / 1_000_000n;
    total += afterDiscount + tax;
  }

  return total;
}

function checksum(view, offset, len) {
  let total = 0n;
  for (let i = 0; i < len; i += 1) {
    total += view.getBigInt64(offset + i * 8, true);
  }
  return total;
}
