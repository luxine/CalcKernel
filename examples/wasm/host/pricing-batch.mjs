#!/usr/bin/env node

import { readFile } from 'node:fs/promises';
import { createCKWasmArena } from './ck-wasm-arena.mjs';

const wasmPath = process.argv[2];
if (!wasmPath) {
  console.error('Usage: node examples/wasm/host/pricing-batch.mjs <pricing_batch.wasm>');
  process.exitCode = 2;
} else {
  await runPricingBatch(wasmPath);
}

async function runPricingBatch(wasmPath) {
  const bytes = await readFile(wasmPath);
  const { instance } = await WebAssembly.instantiate(bytes);
  const arena = createCKWasmArena(instance);
  const count = 3;

  // Allocate each column once. These addresses and the arena remain in use
  // across every batch call below.
  const prices = arena.allocI64(count);
  const quantities = arena.allocI64(count);
  const discounts = arena.allocI64(count);
  const taxRates = arena.allocI64(count);
  const totals = arena.allocI64(count);
  const oldPricesView = arena.viewI64(prices, count);

  // Force a growth so this example demonstrates the host's view refresh rule.
  // Existing views may be detached; rebuild every view from the current buffer.
  const memoryBytesBeforeGrowth = arena.memory.buffer.byteLength;
  arena.reserve(memoryBytesBeforeGrowth + 1);
  arena.refreshViewsIfNeeded();
  const memoryBytesAfterGrowth = arena.memory.buffer.byteLength;
  if (memoryBytesAfterGrowth <= memoryBytesBeforeGrowth) {
    throw new Error('expected memory.reserve to grow WebAssembly memory');
  }

  const pricesView = arena.viewI64(prices, count);
  const quantitiesView = arena.viewI64(quantities, count);
  const discountsView = arena.viewI64(discounts, count);
  const taxRatesView = arena.viewI64(taxRates, count);
  const totalsView = arena.viewI64(totals, count);
  if (oldPricesView.byteLength !== 0) {
    throw new Error('expected the pre-growth typed array view to be detached');
  }

  const rounds = [];
  for (let round = 0; round < 3; round += 1) {
    for (let index = 0; index < count; index += 1) {
      pricesView[index] = BigInt(100 + index * 100 + round * 10);
      quantitiesView[index] = BigInt([2, 1, 4][index]);
      discountsView[index] = BigInt([5, 0, 20][index]);
      taxRatesView[index] = BigInt([100_000, 200_000, 50_000][index]);
    }

    const status = instance.exports.pricing_batch(
      prices, quantities, discounts, taxRates, totals, count,
    );
    if (status !== 0) throw new Error(`pricing_batch returned status ${status}`);

    rounds.push({
      round,
      totals: Array.from(totalsView, (value) => value.toString()),
    });
  }

  console.log(JSON.stringify({ memoryBytesBeforeGrowth, memoryBytesAfterGrowth, rounds }, null, 2));
}
