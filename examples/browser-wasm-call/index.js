const itemSize = 32;
const output = document.querySelector("#output");
const runButton = document.querySelector("#run");

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

async function instantiatePricingWasm() {
  if (WebAssembly.instantiateStreaming) {
    try {
      const result = await WebAssembly.instantiateStreaming(fetch("./pricing.wasm"));
      return result.instance;
    } catch (error) {
      console.warn("instantiateStreaming failed; falling back to ArrayBuffer instantiate", error);
    }
  }

  const response = await fetch("./pricing.wasm");
  if (!response.ok) {
    throw new Error(`failed to fetch pricing.wasm: HTTP ${response.status}`);
  }
  const bytes = await response.arrayBuffer();
  const result = await WebAssembly.instantiate(bytes);
  return result.instance;
}

async function runPricing() {
  output.textContent = "Loading pricing.wasm...";

  const instance = await instantiatePricingWasm();
  const { memory, calc_items: calcItems } = instance.exports;

  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("generated module did not export memory");
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
  const actual = items.map((_, index) => readI64(view, outOffset + index * 8));
  const expected = items.map(expectedTotal);
  const ok = status === 0 && actual.every((value, index) => value === expected[index]);

  output.textContent = [
    `status: ${status}`,
    `out: ${actual.map(String).join(", ")}`,
    `expected: ${expected.map(String).join(", ")}`,
    ok ? "OK" : "FAILED"
  ].join("\n");
}

runButton.addEventListener("click", () => {
  runPricing().catch((error) => {
    output.textContent = error instanceof Error ? error.stack ?? error.message : String(error);
  });
});
