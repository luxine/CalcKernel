const sizes = [100, 1_000, 10_000, 100_000];
const taxScale = 1_000_000n;

function makeInput(len) {
  const price = new BigInt64Array(len);
  const qty = new BigInt64Array(len);
  const discount = new BigInt64Array(len);
  const taxRatePpm = new BigInt64Array(len);
  const out = new BigInt64Array(len);

  for (let i = 0; i < len; i += 1) {
    price[i] = BigInt(1000 + (i % 997));
    qty[i] = BigInt(1 + (i % 9));
    discount[i] = BigInt(i % 113);
    taxRatePpm[i] = BigInt(50_000 + (i % 17) * 2500);
  }

  return { price, qty, discount, taxRatePpm, out };
}

function calcItems(input, len) {
  const { price, qty, discount, taxRatePpm, out } = input;

  for (let i = 0; i < len; i += 1) {
    const subtotal = price[i] * qty[i];
    const afterDiscount = subtotal - discount[i];
    const tax = (afterDiscount * taxRatePpm[i]) / taxScale;
    out[i] = afterDiscount + tax;
  }

  return 0;
}

function checksum(values) {
  let total = 0n;
  for (let i = 0; i < values.length; i += 1) {
    total += values[i];
  }
  return total;
}

function formatMs(ns) {
  return (Number(ns) / 1_000_000).toFixed(3);
}

for (const len of sizes) {
  const input = makeInput(len);

  calcItems(input, len);
  input.out.fill(0n);

  const start = process.hrtime.bigint();
  const status = calcItems(input, len);
  const elapsed = process.hrtime.bigint() - start;

  if (status !== 0) {
    throw new Error(`calcItems returned ${status}`);
  }

  console.log(`${len} items: ${formatMs(elapsed)} ms (checksum=${checksum(input.out)})`);
}
