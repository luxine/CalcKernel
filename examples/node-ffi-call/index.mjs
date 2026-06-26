import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import koffi from "koffi";

const exampleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(exampleDir, "..", "..");

function libraryPath() {
  switch (process.platform) {
    case "darwin":
      return join(repoRoot, "build", "libpricing.dylib");
    case "linux":
      return join(repoRoot, "build", "libpricing.so");
    case "win32":
      return join(repoRoot, "build", "pricing.dll");
    default:
      throw new Error(`unsupported platform: ${process.platform}`);
  }
}

const dylib = libraryPath();
if (!existsSync(dylib)) {
  throw new Error(
    `dynamic library not found: ${dylib}\n` +
      "Build it first with `pnpm ckc build examples/pricing.ck --out build/libpricing` " +
      "on macOS/Linux, or `pnpm ckc build examples/pricing.ck --out build/pricing.dll` on Windows."
  );
}

const lib = koffi.load(dylib);

const Item = koffi.struct("Item", {
  price: "int64_t",
  qty: "int64_t",
  discount: "int64_t",
  tax_rate_ppm: "int64_t"
});

const calcItems = lib.func("int32_t calc_items(Item *items, int32_t len, _Out_ int64_t *out)");

const items = [
  { price: 10000n, qty: 2n, discount: 1000n, tax_rate_ppm: 82500n },
  { price: 2500n, qty: 4n, discount: 0n, tax_rate_ppm: 100000n },
  { price: 1200n, qty: 5n, discount: 500n, tax_rate_ppm: 100000n }
];
const out = new BigInt64Array(items.length);

const status = calcItems(items, items.length, out);
if (status !== 0) {
  throw new Error(`calc_items returned status ${status}`);
}

const expected = [20567n, 11000n, 6050n];
const actual = Array.from(out);
if (actual.length !== expected.length || actual.some((value, index) => value !== expected[index])) {
  throw new Error(`unexpected output: expected ${expected.join(", ")}, got ${actual.join(", ")}`);
}

console.log("OK");
