import { assertWithinTolerance } from "../lib/f64-correctness.mjs";
import { createF64Inputs, expectedF64ComputeSink, f64Factor, positiveInteger, requireF64Kernel, runF64Kernel } from "../lib/f64-workloads.mjs";

const config = parseArgs(process.argv.slice(2));
const { x, y } = createF64Inputs(config.items, "typedarray");
const actual = runF64Kernel(config.kernel, x, y, config.iterations, f64Factor);
const expected = expectedF64ComputeSink(config.kernel, config.items, config.iterations, f64Factor);

assertWithinTolerance(actual, expected, `f64-${config.kernel}-js-float64array`);
console.log(`f64-${config.kernel}-js-float64array items=${config.items} iterations=${config.iterations} checksum=${actual}`);

function parseArgs(argv) {
  const config = { items: 100000, iterations: 1000, kernel: "axpy" };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--items") {
      config.items = positiveInteger(argv[index + 1], "--items");
      index += 1;
    } else if (arg === "--iterations") {
      config.iterations = positiveInteger(argv[index + 1], "--iterations");
      index += 1;
    } else if (arg === "--kernel") {
      config.kernel = requireF64Kernel(argv[index + 1]);
      index += 1;
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }

  return config;
}
