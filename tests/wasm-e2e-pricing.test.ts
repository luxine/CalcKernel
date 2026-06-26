import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";
import { CKWasmArena, type CKWasmMemory } from "../src/wasm/ck-wasm-arena.js";

interface WasmMemoryLike {
  buffer: ArrayBuffer;
}

interface WasmInstanceLike {
  exports: Record<string, unknown>;
}

interface WasmRuntime {
  instantiate(bytes: Uint8Array): Promise<{ instance: WasmInstanceLike }>;
}

const itemSize = 32;

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "calckernel-wasm-"));
}

function getWasmRuntime(): WasmRuntime | undefined {
  return (globalThis as { WebAssembly?: WasmRuntime }).WebAssembly;
}

function writeItem(view: DataView, offset: number, fields: { price: bigint; qty: bigint; discount: bigint; taxRatePpm: bigint }): void {
  view.setBigInt64(offset + 0, fields.price, true);
  view.setBigInt64(offset + 8, fields.qty, true);
  view.setBigInt64(offset + 16, fields.discount, true);
  view.setBigInt64(offset + 24, fields.taxRatePpm, true);
}

function calcExpected(fields: { price: bigint; qty: bigint; discount: bigint; taxRatePpm: bigint }): bigint {
  const subtotal = fields.price * fields.qty;
  const afterDiscount = subtotal - fields.discount;
  const tax = (afterDiscount * fields.taxRatePpm) / 1_000_000n;
  return afterDiscount + tax;
}

function pricingInputArrays(items: Array<{ price: bigint; qty: bigint; discount: bigint; taxRatePpm: bigint }>): {
  prices: BigInt64Array;
  quantities: BigInt64Array;
  discounts: BigInt64Array;
  taxRatesPpm: BigInt64Array;
  totals: bigint[];
  checksum: bigint;
} {
  const prices = new BigInt64Array(items.length);
  const quantities = new BigInt64Array(items.length);
  const discounts = new BigInt64Array(items.length);
  const taxRatesPpm = new BigInt64Array(items.length);
  const totals: bigint[] = [];
  let checksum = 0n;

  for (let index = 0; index < items.length; index += 1) {
    const item = items[index]!;
    const total = calcExpected(item);
    prices[index] = item.price;
    quantities[index] = item.qty;
    discounts[index] = item.discount;
    taxRatesPpm[index] = item.taxRatePpm;
    totals.push(total);
    checksum += total;
  }

  return { prices, quantities, discounts, taxRatesPpm, totals, checksum };
}

describe("Node.js pricing WASM e2e", () => {
  const wasmAvailable = getWasmRuntime() !== undefined && typeof BigInt === "function";

  it.skipIf(!wasmAvailable)(
    wasmAvailable
      ? "runs examples/pricing.ck through generated WASM"
      : "runs examples/pricing.ck through generated WASM (skipped because Node.js WebAssembly BigInt support is unavailable)",
    async () => {
      const wasm = getWasmRuntime();
      expect(wasm).toBeDefined();
      const cwd = tempDir();
      writeFileSync(join(cwd, "pricing.ck"), readFileSync("examples/pricing.ck", "utf8"));
      const watFile = join(cwd, "build/pricing.wat");
      const wasmFile = join(cwd, "build/pricing.wasm");

      let stdout = "";
      let stderr = "";
      const emitWatExitCode = runCli(["emit-wat", "pricing.ck", "--out", "build/pricing.wat"], {
        cwd,
        stdout: (text) => {
          stdout += text;
        },
        stderr: (text) => {
          stderr += text;
        }
      });

      expect(emitWatExitCode).toBe(0);
      expect(stderr).toBe("");
      expect(stdout).toBe("OK: emitted WAT build/pricing.wat\n");
      expect(existsSync(watFile)).toBe(true);

      stdout = "";
      stderr = "";
      const emitWasmExitCode = runCli(["emit-wasm", "pricing.ck", "--out", "build/pricing.wasm"], {
        cwd,
        stdout: (text) => {
          stdout += text;
        },
        stderr: (text) => {
          stderr += text;
        }
      });

      expect(emitWasmExitCode).toBe(0);
      expect(stderr).toBe("");
      expect(stdout).toBe("OK: emitted WASM build/pricing.wasm\n");

      const bytes = readFileSync(wasmFile);
      const { instance } = await wasm!.instantiate(bytes);
      const memory = instance.exports.memory as WasmMemoryLike;
      const view = new DataView(memory.buffer);
      const calcItems = instance.exports.calc_items as (items: number, len: number, out: number) => number;
      const itemsOffset = 0;
      const outOffset = 4096;
      const item0 = { price: 1000n, qty: 3n, discount: 250n, taxRatePpm: 100000n };
      const item1 = { price: 500n, qty: 10n, discount: 1000n, taxRatePpm: 250000n };

      writeItem(view, itemsOffset, item0);
      writeItem(view, itemsOffset + itemSize, item1);

      expect(calcItems(itemsOffset, 2, outOffset)).toBe(0);
      expect(view.getBigInt64(outOffset + 0, true)).toBe(calcExpected(item0));
      expect(view.getBigInt64(outOffset + 8, true)).toBe(calcExpected(item1));
    }
  );

  it.skipIf(!wasmAvailable)(
    wasmAvailable
      ? "runs pricing SoA fixture through CKWasmArena resident memory views"
      : "runs pricing SoA fixture through CKWasmArena resident memory views (skipped because Node.js WebAssembly BigInt support is unavailable)",
    async () => {
      const wasm = getWasmRuntime();
      expect(wasm).toBeDefined();
      const cwd = tempDir();
      writeFileSync(join(cwd, "pricing_soa.ck"), readFileSync("bench/perf/fixtures/pricing_soa.ck", "utf8"));
      const wasmFile = join(cwd, "build/pricing_soa.wasm");

      let stdout = "";
      let stderr = "";
      const emitWasmExitCode = runCli(["emit-wasm", "pricing_soa.ck", "--out", "build/pricing_soa.wasm", "-O3"], {
        cwd,
        stdout: (text) => {
          stdout += text;
        },
        stderr: (text) => {
          stderr += text;
        }
      });

      expect(emitWasmExitCode).toBe(0);
      expect(stderr).toBe("");
      expect(stdout).toBe("OK: emitted WASM build/pricing_soa.wasm\n");

      const bytes = readFileSync(wasmFile);
      const { instance } = await wasm!.instantiate(bytes);
      const memory = instance.exports.memory as CKWasmMemory;
      const pricingSoa = instance.exports.pricing_soa as (
        prices: number,
        quantities: number,
        discounts: number,
        taxRatesPpm: number,
        outTotals: number,
        n: number
      ) => number;
      const arena = new CKWasmArena(memory, { heapBase: 0 });
      const items = [
        { price: 0n, qty: 0n, discount: 0n, taxRatePpm: 0n },
        { price: 1000n, qty: 3n, discount: 250n, taxRatePpm: 100000n },
        { price: 500n, qty: 10n, discount: 1000n, taxRatePpm: 250000n },
        { price: 9n, qty: 2n, discount: 100n, taxRatePpm: 100000n },
        { price: 2147483647n, qty: 3n, discount: 7n, taxRatePpm: 0n }
      ];
      const input = pricingInputArrays(items);

      const pricesPtr = arena.allocI64(items.length);
      const quantitiesPtr = arena.allocI64(items.length);
      const discountsPtr = arena.allocI64(items.length);
      const taxRatesPpmPtr = arena.allocI64(items.length);
      const outTotalsPtr = arena.allocI64(items.length);
      const pricesView = arena.viewI64(pricesPtr, items.length);
      const quantitiesView = arena.viewI64(quantitiesPtr, items.length);
      const discountsView = arena.viewI64(discountsPtr, items.length);
      const taxRatesPpmView = arena.viewI64(taxRatesPpmPtr, items.length);
      const outTotalsView = arena.viewI64(outTotalsPtr, items.length);

      pricesView.set(input.prices);
      quantitiesView.set(input.quantities);
      discountsView.set(input.discounts);
      taxRatesPpmView.set(input.taxRatesPpm);

      const status = pricingSoa(pricesPtr, quantitiesPtr, discountsPtr, taxRatesPpmPtr, outTotalsPtr, items.length);
      expect(status).toBe(0);
      expect(Array.from(outTotalsView)).toEqual(input.totals);
      expect(Array.from(outTotalsView).reduce((total, value) => total + value, 0n)).toBe(input.checksum);
      expect(outTotalsView.buffer).toBe(memory.buffer);
    }
  );
});
