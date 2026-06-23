import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const exampleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(exampleDir, "..", "..");
const wasmPath = join(repoRoot, "build", "pricing.wasm");
const itemSize = 32;

function writeItem(view, base, item) {
  view.setBigInt64(base + 0, item.price, true);
  view.setBigInt64(base + 8, item.qty, true);
  view.setBigInt64(base + 16, item.discount, true);
  view.setBigInt64(base + 24, item.taxRatePpm, true);
}

function readI64(view, offset) {
  return view.getBigInt64(offset, true);
}

function expectedTotal(item) {
  const subtotal = item.price * item.qty;
  const afterDiscount = subtotal - item.discount;
  const tax = (afterDiscount * item.taxRatePpm) / 1_000_000n;
  return afterDiscount + tax;
}

if (!existsSync(wasmPath)) {
  throw new Error(
    `WASM file not found: ${wasmPath}\n` +
      "Generate it first with `ikc emit-wasm ../../examples/pricing.ik --out ../../build/pricing.wasm` " +
      "from examples/node-wasm-call, or `pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm` " +
      "from the repository root."
  );
}

const bytes = readFileSync(wasmPath);
const { instance } = await WebAssembly.instantiate(bytes);
const { memory, calc_items: calcItems } = instance.exports;

if (!(memory instanceof WebAssembly.Memory)) {
  throw new Error("generated module did not export WebAssembly memory");
}

if (typeof calcItems !== "function") {
  throw new Error("generated module did not export calc_items");
}

const view = new DataView(memory.buffer);
const itemsOffset = 0;
const outOffset = 4096;
const items = [
  { price: 10000n, qty: 2n, discount: 1000n, taxRatePpm: 82500n },
  { price: 2500n, qty: 4n, discount: 0n, taxRatePpm: 100000n },
  { price: 1200n, qty: 5n, discount: 500n, taxRatePpm: 100000n }
];

items.forEach((item, index) => {
  writeItem(view, itemsOffset + index * itemSize, item);
});

const status = calcItems(itemsOffset, items.length, outOffset);
if (status !== 0) {
  throw new Error(`calc_items returned ${status}`);
}

const actual = items.map((_, index) => readI64(view, outOffset + index * 8));
const expected = items.map(expectedTotal);

if (actual.length !== expected.length || actual.some((value, index) => value !== expected[index])) {
  throw new Error(`unexpected output: expected ${expected.join(", ")}, got ${actual.join(", ")}`);
}

console.log("OK");
