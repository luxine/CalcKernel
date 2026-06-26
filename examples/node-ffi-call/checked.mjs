import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import koffi from "koffi";

const CK_OK = 0;
const CK_ERR_OVERFLOW = 1;
const CK_ERR_DIV_BY_ZERO = 2;
const CK_ERR_NULL_POINTER = 3;

const exampleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(exampleDir, "..", "..");

function libraryPath() {
  switch (process.platform) {
    case "darwin":
      return join(repoRoot, "build", "libpricing_checked.dylib");
    case "linux":
      return join(repoRoot, "build", "libpricing_checked.so");
    case "win32":
      return join(repoRoot, "build", "pricing_checked.dll");
    default:
      throw new Error(`unsupported platform: ${process.platform}`);
  }
}

const dylib = libraryPath();
if (!existsSync(dylib)) {
  throw new Error(
    `dynamic library not found: ${dylib}\n` +
      "Build it first with `pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked` " +
      "on macOS/Linux, or `pnpm ckc build examples/pricing.ck --out build/pricing_checked.dll --overflow checked` on Windows."
  );
}

const lib = koffi.load(dylib);

const Item = koffi.struct("Item", {
  price: "int64_t",
  qty: "int64_t",
  discount: "int64_t",
  tax_rate_ppm: "int64_t"
});

const calcItems = lib.func("int32_t calc_items(Item *items, int32_t len, _Out_ int64_t *out, _Out_ int32_t *ck_return)");

function runSuccessCase() {
  const items = [
    { price: 10000n, qty: 2n, discount: 1000n, tax_rate_ppm: 82500n },
    { price: 2500n, qty: 4n, discount: 0n, tax_rate_ppm: 100000n },
    { price: 1200n, qty: 5n, discount: 500n, tax_rate_ppm: 100000n }
  ];
  const out = new BigInt64Array(items.length);
  const ckReturn = new Int32Array(1);
  ckReturn[0] = -1;

  const status = calcItems(items, items.length, out, ckReturn);
  if (status !== CK_OK) {
    throw new Error(`calc_items returned CK_Status ${status}`);
  }
  if (ckReturn[0] !== 0) {
    throw new Error(`unexpected ck_return: expected 0, got ${ckReturn[0]}`);
  }

  const expected = [20567n, 11000n, 6050n];
  const actual = Array.from(out);
  if (actual.length !== expected.length || actual.some((value, index) => value !== expected[index])) {
    throw new Error(`unexpected output: expected ${expected.join(", ")}, got ${actual.join(", ")}`);
  }
}

function runOverflowCase() {
  const items = [{ price: 9223372036854775807n, qty: 2n, discount: 0n, tax_rate_ppm: 0n }];
  const out = new BigInt64Array(items.length);
  const ckReturn = new Int32Array(1);
  ckReturn[0] = -1;

  const status = calcItems(items, items.length, out, ckReturn);
  if (status !== CK_ERR_OVERFLOW) {
    throw new Error(`expected CK_ERR_OVERFLOW, got CK_Status ${status}`);
  }
}

runSuccessCase();
runOverflowCase();

console.log("OK");
