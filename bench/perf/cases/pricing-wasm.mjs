import { readFileSync } from "node:fs";

const config = parseArgs(process.argv.slice(2));
const result = await runMode(config);
console.log(result);

function parseArgs(argv) {
  const config = { items: 100000, iterations: 1000, calls: 10000, mode: "compute-only", wasm: "build/perf/generated/pricing.wasm" };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--items") {
      config.items = positiveInteger(argv[index + 1], "--items");
      index += 1;
    } else if (arg === "--iterations") {
      config.iterations = positiveInteger(argv[index + 1], "--iterations");
      index += 1;
    } else if (arg === "--calls") {
      config.calls = positiveInteger(argv[index + 1], "--calls");
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
    case "total":
      return runTotal(config);
    case "compute-only":
      return runComputeOnly(config);
    case "memory-only":
      return runMemoryOnly(config);
    case "call-overhead":
      return runCallOverhead(config);
    case "soa-setup-copy-in":
      return runSoASetupCopyIn(config);
    case "soa-resident-total":
      return runSoAResidentTotal(config);
    case "soa-readback-cost":
      return runSoAReadbackCost(config);
    case "soa-total-with-final-readback":
      return runSoATotalWithFinalReadback(config);
  }
}

function positiveInteger(value, flag) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${flag} must be a positive integer`);
  }
  return parsed;
}

function requireMode(value) {
  if (
    value === "total" ||
    value === "compute-only" ||
    value === "memory-only" ||
    value === "call-overhead" ||
    value === "soa-setup-copy-in" ||
    value === "soa-resident-total" ||
    value === "soa-readback-cost" ||
    value === "soa-total-with-final-readback"
  ) {
    return value;
  }
  throw new Error(
    "--mode must be total, compute-only, memory-only, call-overhead, soa-setup-copy-in, soa-resident-total, soa-readback-cost, or soa-total-with-final-readback"
  );
}

async function instantiate(path) {
  const bytes = readFileSync(path);
  const { instance } = await WebAssembly.instantiate(bytes);
  return instance;
}

async function runTotal(config) {
  const { view, calcItems, layout } = await pricingInstance(config);
  const expected = expectedChecksum(config.items);
  let actual = 0n;

  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    writeItems(view, layout.itemsOffset, config.items);
    const status = calcItems(layout.itemsOffset, config.items, layout.outOffset);
    if (status !== 0) {
      throw new Error(`calc_items returned ${status}`);
    }
    actual = checksum(view, layout.outOffset, config.items);
    if (actual !== expected) {
      throw new Error(`checksum mismatch: expected ${expected}, got ${actual}`);
    }
  }

  return `pricing-wasm-unchecked-total items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runComputeOnly(config) {
  const { view, calcItems, layout } = await pricingInstance(config);
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

  return `pricing-wasm-unchecked-compute-only items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

function runMemoryOnly(config) {
  const layout = requiredBytesFor(config.items);
  const memory = new WebAssembly.Memory({ initial: 1 });
  const view = ensureMemory(memory, layout.totalBytes);
  const expected = expectedChecksum(config.items);
  let actual = 0n;

  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    writeItems(view, layout.itemsOffset, config.items);
    writeExpectedOut(view, layout.outOffset, config.items);
    actual = checksum(view, layout.outOffset, config.items);
    if (actual !== expected) {
      throw new Error(`checksum mismatch: expected ${expected}, got ${actual}`);
    }
  }

  return `pricing-wasm-unchecked-memory-only items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runCallOverhead(config) {
  const instance = await instantiate(config.wasm);
  const addI32 = instance.exports.add_i32;
  if (typeof addI32 !== "function") {
    throw new Error("call-overhead wasm does not export add_i32");
  }

  let actual = 0;
  let expected = 0;
  for (let call = 0; call < config.calls; call += 1) {
    const left = call & 1023;
    const right = 1;
    actual += addI32(left, right);
    expected += left + right;
  }

  if (actual !== expected) {
    throw new Error(`checksum mismatch: expected ${expected}, got ${actual}`);
  }

  return `pricing-wasm-unchecked-call-overhead calls=${config.calls} checksum=${actual}`;
}

async function runSoASetupCopyIn(config) {
  const inputs = makeSoAInput(config.items);
  const runtime = await pricingSoAInstance(config);
  copySoAInputs(runtime, inputs);

  const actual = soaSetupGuard(runtime, config.items);
  const expected = soaInputGuard(inputs, config.items);
  if (actual !== expected) {
    throw new Error(`setup guard mismatch: expected ${expected}, got ${actual}`);
  }

  return `pricing-wasm-soa-setup-copy-in-O3 items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runSoAResidentTotal(config) {
  const inputs = makeSoAInput(config.items);
  const runtime = await pricingSoAInstance(config);
  copySoAInputs(runtime, inputs);

  let actual = 0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += callPricingSoA(runtime, config.items);
  }

  if (actual !== 0) {
    throw new Error(`status mismatch: expected 0, got ${actual}`);
  }

  return `pricing-wasm-soa-resident-total-O3 items=${config.items} iterations=${config.iterations} status=${actual}`;
}

async function runSoAReadbackCost(config) {
  const inputs = makeSoAInput(config.items);
  const runtime = await pricingSoAInstance(config);
  copySoAInputs(runtime, inputs);

  const status = callPricingSoA(runtime, config.items);
  const expectedKernelChecksum = expectedChecksum(config.items);
  if (status !== 0) {
    throw new Error(`pricing_soa returned ${status}`);
  }

  let actual = 0n;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += checksumBigInt64Array(runtime.outTotalsView, config.items);
  }

  const expected = expectedKernelChecksum * BigInt(config.iterations);
  if (actual !== expected) {
    throw new Error(`readback checksum mismatch: expected ${expected}, got ${actual}`);
  }

  return `pricing-wasm-soa-readback-cost-O3 items=${config.items} iterations=${config.iterations} checksum=${actual}`;
}

async function runSoATotalWithFinalReadback(config) {
  const inputs = makeSoAInput(config.items);
  const runtime = await pricingSoAInstance(config);
  copySoAInputs(runtime, inputs);

  let actual = 0;
  for (let iteration = 0; iteration < config.iterations; iteration += 1) {
    actual += callPricingSoA(runtime, config.items);
  }

  const expectedSingle = expectedChecksum(config.items);
  const finalOutputChecksum = checksumBigInt64Array(runtime.outTotalsView, config.items);
  if (actual !== 0) {
    throw new Error(`status mismatch: expected 0, got ${actual}`);
  }
  if (finalOutputChecksum !== expectedSingle) {
    throw new Error(`final readback checksum mismatch: expected ${expectedSingle}, got ${finalOutputChecksum}`);
  }

  return `pricing-wasm-soa-total-with-final-readback-O3 items=${config.items} iterations=${config.iterations} status=${actual} checksum=${finalOutputChecksum}`;
}

async function pricingInstance(config) {
  const instance = await instantiate(config.wasm);
  const memory = instance.exports.memory;
  const calcItems = instance.exports.calc_items;

  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("pricing.wasm does not export WebAssembly memory");
  }
  if (typeof calcItems !== "function") {
    throw new Error("pricing.wasm does not export calc_items");
  }

  const layout = requiredBytesFor(config.items);
  return { view: ensureMemory(memory, layout.totalBytes), calcItems, layout };
}

async function pricingSoAInstance(config) {
  const instance = await instantiate(config.wasm);
  const memory = instance.exports.memory;
  const pricingSoA = instance.exports.pricing_soa;

  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("pricing SoA wasm does not export WebAssembly memory");
  }
  if (typeof pricingSoA !== "function") {
    throw new Error("pricing SoA wasm does not export pricing_soa");
  }

  const { CKWasmArena } = await import("../../../dist/src/wasm/ck-wasm-arena.js");
  const arena = new CKWasmArena(memory, { heapBase: 0 });
  const pricesOffset = arena.allocI64(config.items);
  const quantitiesOffset = arena.allocI64(config.items);
  const discountsOffset = arena.allocI64(config.items);
  const taxRatesPpmOffset = arena.allocI64(config.items);
  const outTotalsOffset = arena.allocI64(config.items);
  const totalBytes = outTotalsOffset + config.items * BigInt64Array.BYTES_PER_ELEMENT;
  arena.ensureBytes(totalBytes);

  return {
    pricingSoA,
    layout: {
      pricesOffset,
      quantitiesOffset,
      discountsOffset,
      taxRatesPpmOffset,
      outTotalsOffset
    },
    pricesView: arena.viewI64(pricesOffset, config.items),
    quantitiesView: arena.viewI64(quantitiesOffset, config.items),
    discountsView: arena.viewI64(discountsOffset, config.items),
    taxRatesPpmView: arena.viewI64(taxRatesPpmOffset, config.items),
    outTotalsView: arena.viewI64(outTotalsOffset, config.items)
  };
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

function makeSoAInput(len) {
  const prices = new BigInt64Array(len);
  const quantities = new BigInt64Array(len);
  const discounts = new BigInt64Array(len);
  const taxRatesPpm = new BigInt64Array(len);

  for (let i = 0; i < len; i += 1) {
    prices[i] = BigInt(1000 + (i % 997));
    quantities[i] = BigInt(1 + (i % 9));
    discounts[i] = BigInt(i % 113);
    taxRatesPpm[i] = BigInt(50_000 + (i % 17) * 2500);
  }

  return { prices, quantities, discounts, taxRatesPpm };
}

function copySoAInputs(runtime, inputs) {
  runtime.pricesView.set(inputs.prices);
  runtime.quantitiesView.set(inputs.quantities);
  runtime.discountsView.set(inputs.discounts);
  runtime.taxRatesPpmView.set(inputs.taxRatesPpm);
}

function callPricingSoA(runtime, len) {
  return runtime.pricingSoA(
    runtime.layout.pricesOffset,
    runtime.layout.quantitiesOffset,
    runtime.layout.discountsOffset,
    runtime.layout.taxRatesPpmOffset,
    runtime.layout.outTotalsOffset,
    len
  );
}

function soaInputGuard(inputs, len) {
  const last = len - 1;
  return (
    inputs.prices[0] +
    inputs.quantities[0] +
    inputs.discounts[0] +
    inputs.taxRatesPpm[0] +
    inputs.prices[last] +
    inputs.quantities[last] +
    inputs.discounts[last] +
    inputs.taxRatesPpm[last]
  );
}

function soaSetupGuard(runtime, len) {
  const last = len - 1;
  return (
    runtime.pricesView[0] +
    runtime.quantitiesView[0] +
    runtime.discountsView[0] +
    runtime.taxRatesPpmView[0] +
    runtime.pricesView[last] +
    runtime.quantitiesView[last] +
    runtime.discountsView[last] +
    runtime.taxRatesPpmView[last]
  );
}

function writeExpectedOut(view, offset, len) {
  for (let i = 0; i < len; i += 1) {
    const price = BigInt(1000 + (i % 997));
    const qty = BigInt(1 + (i % 9));
    const discount = BigInt(i % 113);
    const taxRatePpm = BigInt(50_000 + (i % 17) * 2500);
    const subtotal = price * qty;
    const afterDiscount = subtotal - discount;
    const tax = (afterDiscount * taxRatePpm) / 1_000_000n;
    view.setBigInt64(offset + i * 8, afterDiscount + tax, true);
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

function checksumBigInt64Array(values, len) {
  let total = 0n;
  for (let i = 0; i < len; i += 1) {
    total += values[i];
  }
  return total;
}
