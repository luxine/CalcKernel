const config = parseArgs(process.argv.slice(2));
const input = makeInput(config.items);

for (let iteration = 0; iteration < config.iterations; iteration += 1) {
  calcItems(input, config.items);
}

const actual = checksum(input.out);
const expected = expectedChecksum(input);
if (actual !== expected) {
  throw new Error(`checksum mismatch: expected ${expected}, got ${actual}`);
}

console.log(`pricing-js-typedarray-number items=${config.items} iterations=${config.iterations} checksum=${actual}`);

function parseArgs(argv) {
  const config = { items: 100000, iterations: 1000 };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--items") {
      config.items = positiveInteger(argv[index + 1], "--items");
      index += 1;
    } else if (arg === "--iterations") {
      config.iterations = positiveInteger(argv[index + 1], "--iterations");
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

function makeInput(len) {
  const price = new Float64Array(len);
  const qty = new Float64Array(len);
  const discount = new Float64Array(len);
  const taxRatePpm = new Float64Array(len);
  const out = new Float64Array(len);

  for (let i = 0; i < len; i += 1) {
    price[i] = 1000 + (i % 997);
    qty[i] = 1 + (i % 9);
    discount[i] = i % 113;
    taxRatePpm[i] = 50_000 + (i % 17) * 2500;
  }

  return { price, qty, discount, taxRatePpm, out };
}

function calcItems(input, len) {
  const { price, qty, discount, taxRatePpm, out } = input;

  for (let i = 0; i < len; i += 1) {
    const subtotal = price[i] * qty[i];
    const afterDiscount = subtotal - discount[i];
    const tax = Math.trunc((afterDiscount * taxRatePpm[i]) / 1_000_000);
    out[i] = afterDiscount + tax;
  }

  return 0;
}

function expectedChecksum(input) {
  let total = 0;

  for (let i = 0; i < input.price.length; i += 1) {
    const subtotal = input.price[i] * input.qty[i];
    const afterDiscount = subtotal - input.discount[i];
    const tax = Math.trunc((afterDiscount * input.taxRatePpm[i]) / 1_000_000);
    total += afterDiscount + tax;
  }

  return total;
}

function checksum(values) {
  let total = 0;
  for (let i = 0; i < values.length; i += 1) {
    total += values[i];
  }
  return total;
}
