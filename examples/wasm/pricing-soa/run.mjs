import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const exampleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(exampleDir, "../../..");
const sourcePath = join(exampleDir, "pricing_soa.ck");
const wasmPath = join(repoRoot, "build/examples/wasm/pricing-soa/pricing_soa.wasm");

function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`);
  }
  return result;
}

async function loadCalcKernel() {
  const distIndex = join(repoRoot, "dist/src/index.js");
  if (!existsSync(distIndex)) {
    throw new Error("Build CK / CalcKernel first with `pnpm build`.");
  }
  return import(pathToFileURL(distIndex).href);
}

function emitWasm() {
  mkdirSync(dirname(wasmPath), { recursive: true });
  run(process.execPath, [join(repoRoot, "dist/src/cli.js"), "emit-wasm", sourcePath, "--out", wasmPath, "-O3"], repoRoot);
}

function expectedTotal(price, quantity, discount, taxRatePpm) {
  const subtotal = price * quantity;
  const afterDiscount = subtotal - discount;
  const tax = (afterDiscount * taxRatePpm) / 1_000_000n;
  return afterDiscount + tax;
}

function pricingInput() {
  const rows = [
    { price: 10000n, quantity: 2n, discount: 1000n, taxRatePpm: 82500n },
    { price: 2500n, quantity: 4n, discount: 0n, taxRatePpm: 100000n },
    { price: 1200n, quantity: 5n, discount: 500n, taxRatePpm: 100000n },
    { price: 999n, quantity: 3n, discount: 100n, taxRatePpm: 62500n }
  ];
  const prices = new BigInt64Array(rows.length);
  const quantities = new BigInt64Array(rows.length);
  const discounts = new BigInt64Array(rows.length);
  const taxRatesPpm = new BigInt64Array(rows.length);
  const expected = [];

  for (let index = 0; index < rows.length; index += 1) {
    const row = rows[index];
    prices[index] = row.price;
    quantities[index] = row.quantity;
    discounts[index] = row.discount;
    taxRatesPpm[index] = row.taxRatePpm;
    expected.push(expectedTotal(row.price, row.quantity, row.discount, row.taxRatePpm));
  }

  return { prices, quantities, discounts, taxRatesPpm, expected };
}

export async function runExample() {
  emitWasm();
  const { createCKWasmArena } = await loadCalcKernel();
  const bytes = readFileSync(wasmPath);
  const { instance } = await WebAssembly.instantiate(bytes);
  const pricingSoA = instance.exports.pricing_soa;
  if (typeof pricingSoA !== "function") {
    throw new Error("generated WASM did not export pricing_soa");
  }

  const arena = createCKWasmArena(instance);
  const input = pricingInput();
  const len = input.expected.length;

  const pricesPtr = arena.allocI64(len);
  const quantitiesPtr = arena.allocI64(len);
  const discountsPtr = arena.allocI64(len);
  const taxRatesPpmPtr = arena.allocI64(len);
  const outTotalsPtr = arena.allocI64(len);

  const pricesView = arena.viewI64(pricesPtr, len);
  const quantitiesView = arena.viewI64(quantitiesPtr, len);
  const discountsView = arena.viewI64(discountsPtr, len);
  const taxRatesPpmView = arena.viewI64(taxRatesPpmPtr, len);
  const outTotalsView = arena.viewI64(outTotalsPtr, len);

  pricesView.set(input.prices);
  quantitiesView.set(input.quantities);
  discountsView.set(input.discounts);
  taxRatesPpmView.set(input.taxRatesPpm);

  const status = pricingSoA(pricesPtr, quantitiesPtr, discountsPtr, taxRatesPpmPtr, outTotalsPtr, len);
  const actual = Array.from(outTotalsView);
  if (status !== 0) {
    throw new Error(`pricing_soa returned ${status}`);
  }
  for (let index = 0; index < input.expected.length; index += 1) {
    if (actual[index] !== input.expected[index]) {
      throw new Error(`unexpected total[${index}]: expected ${input.expected[index]}, got ${actual[index]}`);
    }
  }

  return {
    status,
    actual,
    expected: input.expected,
    dataViewHotPath: false,
    outputOwnership: "wasm-memory-view"
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = await runExample();
  console.log(`OK pricing-soa totals=${result.actual.map(String).join(",")}`);
}
