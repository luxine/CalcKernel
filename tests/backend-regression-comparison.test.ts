import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];
const itemSize = 32;

interface WasmMemoryLike {
  buffer: ArrayBuffer;
}

interface WasmInstanceLike {
  exports: Record<string, unknown>;
}

interface WasmRuntime {
  Module: new (bytes: Uint8Array) => unknown;
  Instance: new (module: unknown) => WasmInstanceLike;
  instantiate(bytes: Uint8Array): Promise<{ instance: WasmInstanceLike }>;
}

interface RegressionFixture {
  name: string;
  sourceFile: string;
  expected: string;
  cHarness: (headerName: string) => string;
  llvmHarness: string;
  runWasm: (instance: WasmInstanceLike) => string;
}

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-backend-regression-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

function getWasmRuntime(): WasmRuntime | undefined {
  return (globalThis as { WebAssembly?: WasmRuntime }).WebAssembly;
}

function supportsWasmI64BigInt(): boolean {
  const wasm = getWasmRuntime();
  if (!wasm || typeof BigInt !== "function") {
    return false;
  }

  try {
    const bytes = Uint8Array.from([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x06, 0x01, 0x60, 0x01, 0x7e, 0x01, 0x7e,
      0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x01, 0x02, 0x69, 0x64, 0x00, 0x00, 0x0a, 0x06, 0x01, 0x04,
      0x00, 0x20, 0x00, 0x0b
    ]);
    const module = new wasm.Module(bytes);
    const instance = new wasm.Instance(module);
    const id = instance.exports.id as (value: bigint) => bigint;
    return id(1n) === 1n;
  } catch {
    return false;
  }
}

function writeFixtureSource(cwd: string, sourceFile: string): string {
  const inputName = basename(sourceFile);
  writeFileSync(join(cwd, inputName), readFileSync(sourceFile, "utf8"));
  return inputName;
}

function runNativeCBackend(fixture: RegressionFixture): string {
  const cwd = tempDir();
  const inputName = writeFixtureSource(cwd, fixture.sourceFile);
  const cFile = join(cwd, `build/${fixture.name}.c`);
  const headerFile = join(cwd, `build/${fixture.name}.h`);
  const headerName = basename(headerFile);
  const harnessFile = join(cwd, `build/${fixture.name}_c_harness.c`);
  const executable = join(cwd, `build/${fixture.name}_c_test`);

  const emitExitCode = runCli(["emit-c", inputName, "--out", cFile, "--header", headerFile], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });
  expect(emitExitCode).toBe(0);

  writeFileSync(harnessFile, fixture.cHarness(headerName));

  const compile = spawnSync("clang", [...strictClangFlags, cFile, harnessFile, "-o", executable], { encoding: "utf8" });
  expect(compile.status, compile.stderr || compile.stdout).toBe(0);

  const run = spawnSync(executable, [], { encoding: "utf8" });
  expect(run.status, run.stderr || run.stdout).toBe(0);
  return run.stdout.trim();
}

function runNativeLlvmBackend(fixture: RegressionFixture): string {
  const cwd = tempDir();
  const inputName = writeFixtureSource(cwd, fixture.sourceFile);
  const llFile = join(cwd, `build/${fixture.name}.ll`);
  const harnessFile = join(cwd, `build/${fixture.name}_llvm_harness.c`);
  const executable = join(cwd, `build/${fixture.name}_llvm_test`);

  const emitExitCode = runCli(["emit-llvm", inputName, "--out", llFile], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });
  expect(emitExitCode).toBe(0);

  writeFileSync(harnessFile, fixture.llvmHarness);

  const compile = spawnSync("clang", [...strictClangFlags, llFile, harnessFile, "-o", executable], { encoding: "utf8" });
  expect(compile.status, compile.stderr || compile.stdout).toBe(0);

  const run = spawnSync(executable, [], { encoding: "utf8" });
  expect(run.status, run.stderr || run.stdout).toBe(0);
  return run.stdout.trim();
}

async function runWasmBackend(fixture: RegressionFixture): Promise<string> {
  const wasm = getWasmRuntime();
  expect(wasm).toBeDefined();
  const cwd = tempDir();
  const inputName = writeFixtureSource(cwd, fixture.sourceFile);
  const wasmFile = join(cwd, `build/${fixture.name}.wasm`);

  const emitExitCode = runCli(["emit-wasm", inputName, "--out", wasmFile], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });
  expect(emitExitCode).toBe(0);

  const bytes = readFileSync(wasmFile);
  const { instance } = await wasm!.instantiate(bytes);
  return fixture.runWasm(instance);
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

function classifyF64(value: number): string {
  if (Number.isNaN(value)) {
    return "nan";
  }
  if (value === Infinity) {
    return "+inf";
  }
  if (value === -Infinity) {
    return "-inf";
  }
  if (Object.is(value, -0)) {
    return "-0";
  }
  if (Object.is(value, 0)) {
    return "+0";
  }
  return "finite";
}

function closeF64(actual: number, expected: number): boolean {
  const diff = Math.abs(actual - expected);
  const scale = Math.max(1, Math.abs(actual), Math.abs(expected));
  return diff <= 1e-12 * scale || diff <= 1e-12;
}

type MatrixResult = [name: string, passed: boolean];

function formatF64MatrixResults(results: readonly MatrixResult[]): string {
  return `f64_matrix:${results.map(([name, passed]) => `${name}=${passed ? "ok" : "fail"}`).join(";")}`;
}

function cHarness(headerName: string, body: string): string {
  return `#include "${headerName}"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

${body}
`;
}

function llvmHarness(prototypes: string, body: string): string {
  return `#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

${prototypes}

${body}
`;
}

const scalarBody = `int main(void) {
  printf("scalar:add_i64=%lld;mul_i32=%d;less_i64=%d;div_u64=%llu\\n",
    (long long)add_i64(1, 2),
    mul_i32(3, 4),
    less_i64(1, 2) ? 1 : 0,
    (unsigned long long)div_u64(10, 2));
  return 0;
}`;

const controlFlowBody = `int main(void) {
  printf("control:max_a=%d;max_b=%d;sum=%lld\\n",
    max_i32(10, 3),
    max_i32(1, 3),
    (long long)sum_to_n(5));
  return 0;
}`;

const callsBody = `int main(void) {
  printf("calls:calc=%lld\\n", (long long)calc(1, 2));
  return 0;
}`;

const shortCircuitBody = `int main(void) {
  printf("short:and0=%d;and2=%d;or0=%d;or2=%d\\n",
    and_short_circuit(0, 10) ? 1 : 0,
    and_short_circuit(2, 10) ? 1 : 0,
    or_short_circuit(0, 10) ? 1 : 0,
    or_short_circuit(2, 10) ? 1 : 0);
  return 0;
}`;

const memoryTypes = `typedef struct Item {
  int64_t price;
  int64_t qty;
  int64_t discount;
  int64_t tax_rate_ppm;
} Item;`;

const memoryBody = `int main(void) {
  Item items[2] = {
    { .price = 1234, .qty = 2, .discount = 3, .tax_rate_ppm = 4 },
    { .price = 222, .qty = 0, .discount = 0, .tax_rate_ppm = 0 }
  };
  int64_t out[1] = {0};
  int32_t status = write_i64(out, 123);
  printf("memory:first=%lld;second=%lld;status=%d;out=%lld\\n",
    (long long)first_price(items),
    (long long)get_price(items, 1),
    status,
    (long long)out[0]);
  return 0;
}`;

const pricingBody = `int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};
  int32_t status = calc_items(items, 2, out);
  printf("pricing:status=%d;out0=%lld;out1=%lld\\n",
    status,
    (long long)out[0],
    (long long)out[1]);
  return 0;
}`;

const f64MatrixCaseNames = [
  "finite_add",
  "finite_sub",
  "finite_mul",
  "finite_div",
  "tolerance_calc",
  "finite_less",
  "finite_less_equal",
  "finite_equal",
  "pos_inf",
  "neg_inf",
  "nan",
  "neg_zero",
  "zero_eq_neg_zero",
  "nan_eq_nan",
  "nan_ne_nan",
  "nan_lt_one",
  "nan_le_one",
  "nan_gt_one",
  "nan_ge_one",
  "inf_plus",
  "inf_minus_inf",
  "overflow",
  "underflow",
  "inf_gt_finite",
  "neg_inf_lt_finite",
  "ptr_load",
  "ptr_store",
  "struct_read",
  "struct_write",
  "nested_struct_read",
  "nested_struct_write"
] as const;

const f64MatrixExpected = `f64_matrix:${f64MatrixCaseNames.map((name) => `${name}=ok`).join(";")}`;

const f64MatrixTypes = `typedef struct Quote {
  double price;
  double tax;
} Quote;

typedef struct NestedQuote {
  Quote quote;
  double fee;
} NestedQuote;`;

const f64MatrixBody = `#include <math.h>
#include <string.h>

static const char* class_f64(double value) {
  if (isnan(value)) {
    return "nan";
  }
  if (isinf(value)) {
    return signbit(value) ? "-inf" : "+inf";
  }
  if (value == 0.0 && signbit(value)) {
    return "-0";
  }
  if (value == 0.0) {
    return "+0";
  }
  return "finite";
}

static const char* ok(int pass) {
  return pass ? "ok" : "fail";
}

static int class_is(double value, const char* expected) {
  return strcmp(class_f64(value), expected) == 0;
}

static int close_f64(double actual, double expected) {
  double diff = fabs(actual - expected);
  double scale = fabs(actual);
  double expected_abs = fabs(expected);
  if (expected_abs > scale) {
    scale = expected_abs;
  }
  if (scale < 1.0) {
    scale = 1.0;
  }
  return diff <= 0.000000000001 * scale || diff <= 0.000000000001;
}

int main(void) {
  double values[3] = {1.0, 2.5, 4.0};
  Quote quotes[2] = {
    { .price = 10.25, .tax = 0.75 },
    { .price = 20.5, .tax = 1.25 }
  };
  NestedQuote nested[2] = {
    { .quote = { .price = 1.25, .tax = 0.75 }, .fee = 2.0 },
    { .quote = { .price = 10.0, .tax = 2.0 }, .fee = 3.0 }
  };
  double ptr_store_value = ptr_write(values, 1, 8.75);
  double struct_write_value = struct_write(quotes, 1, 0.5);
  double nested_write_value = nested_struct_write(nested, 1, 1.5);

  printf("f64_matrix:");
  printf("finite_add=%s;", ok(close_f64(finite_add(), 4.0)));
  printf("finite_sub=%s;", ok(close_f64(finite_sub(), 3.5)));
  printf("finite_mul=%s;", ok(close_f64(finite_mul(), 3.75)));
  printf("finite_div=%s;", ok(close_f64(finite_div(), 3.5)));
  printf("tolerance_calc=%s;", ok(close_f64(tolerance_calc(), 10.0)));
  printf("finite_less=%s;", ok(finite_less() == true));
  printf("finite_less_equal=%s;", ok(finite_less_equal() == true));
  printf("finite_equal=%s;", ok(finite_equal() == true));
  printf("pos_inf=%s;", ok(class_is(positive_infinity(), "+inf")));
  printf("neg_inf=%s;", ok(class_is(negative_infinity(), "-inf")));
  printf("nan=%s;", ok(class_is(not_a_number(), "nan")));
  printf("neg_zero=%s;", ok(class_is(negative_zero(), "-0")));
  printf("zero_eq_neg_zero=%s;", ok(zero_equals_negative_zero() == true));
  printf("nan_eq_nan=%s;", ok(nan_equals_nan() == false));
  printf("nan_ne_nan=%s;", ok(nan_not_equals_nan() == true));
  printf("nan_lt_one=%s;", ok(nan_less_than_one() == false));
  printf("nan_le_one=%s;", ok(nan_less_equal_one() == false));
  printf("nan_gt_one=%s;", ok(nan_greater_than_one() == false));
  printf("nan_ge_one=%s;", ok(nan_greater_equal_one() == false));
  printf("inf_plus=%s;", ok(class_is(infinity_plus_finite(), "+inf")));
  printf("inf_minus_inf=%s;", ok(class_is(infinity_minus_infinity(), "nan")));
  printf("overflow=%s;", ok(class_is(overflow_to_infinity(), "+inf")));
  printf("underflow=%s;", ok(class_is(underflow_smoke(), "+0")));
  printf("inf_gt_finite=%s;", ok(infinity_greater_than_finite() == true));
  printf("neg_inf_lt_finite=%s;", ok(negative_infinity_less_than_finite() == true));
  printf("ptr_load=%s;", ok(close_f64(ptr_read(values, 2), 4.0)));
  printf("ptr_store=%s;", ok(close_f64(ptr_store_value, 8.75) && close_f64(values[1], 8.75)));
  printf("struct_read=%s;", ok(close_f64(struct_read(quotes, 0), 11.0)));
  printf("struct_write=%s;", ok(close_f64(struct_write_value, 21.0) && close_f64(quotes[1].tax, 0.5)));
  printf("nested_struct_read=%s;", ok(close_f64(nested_struct_read(nested, 0), 4.0)));
  printf("nested_struct_write=%s\\n", ok(close_f64(nested_write_value, 14.5) && close_f64(nested[1].quote.tax, 1.5)));
  return 0;
}`;

const fixtures: RegressionFixture[] = [
  {
    name: "scalar",
    sourceFile: "examples/llvm_scalar.ik",
    expected: "scalar:add_i64=3;mul_i32=12;less_i64=1;div_u64=5",
    cHarness: (headerName) => cHarness(headerName, scalarBody),
    llvmHarness: llvmHarness(
      `int64_t add_i64(int64_t a, int64_t b);
int32_t mul_i32(int32_t a, int32_t b);
bool less_i64(int64_t a, int64_t b);
uint64_t div_u64(uint64_t a, uint64_t b);`,
      scalarBody
    ),
    runWasm: (instance) => {
      const addI64 = instance.exports.add_i64 as (a: bigint, b: bigint) => bigint;
      const mulI32 = instance.exports.mul_i32 as (a: number, b: number) => number;
      const lessI64 = instance.exports.less_i64 as (a: bigint, b: bigint) => number;
      const divU64 = instance.exports.div_u64 as (a: bigint, b: bigint) => bigint;
      return `scalar:add_i64=${addI64(1n, 2n)};mul_i32=${mulI32(3, 4)};less_i64=${lessI64(1n, 2n)};div_u64=${divU64(10n, 2n)}`;
    }
  },
  {
    name: "control",
    sourceFile: "examples/llvm_control_flow.ik",
    expected: "control:max_a=10;max_b=3;sum=10",
    cHarness: (headerName) => cHarness(headerName, controlFlowBody),
    llvmHarness: llvmHarness(
      `int32_t max_i32(int32_t a, int32_t b);
int64_t sum_to_n(int64_t n);`,
      controlFlowBody
    ),
    runWasm: (instance) => {
      const maxI32 = instance.exports.max_i32 as (a: number, b: number) => number;
      const sumToN = instance.exports.sum_to_n as (n: bigint) => bigint;
      return `control:max_a=${maxI32(10, 3)};max_b=${maxI32(1, 3)};sum=${sumToN(5n)}`;
    }
  },
  {
    name: "calls",
    sourceFile: "examples/llvm_calls.ik",
    expected: "calls:calc=6",
    cHarness: (headerName) => cHarness(headerName, callsBody),
    llvmHarness: llvmHarness("int64_t calc(int64_t a, int64_t b);", callsBody),
    runWasm: (instance) => {
      const calc = instance.exports.calc as (a: bigint, b: bigint) => bigint;
      return `calls:calc=${calc(1n, 2n)}`;
    }
  },
  {
    name: "short",
    sourceFile: "examples/llvm_short_circuit.ik",
    expected: "short:and0=0;and2=1;or0=1;or2=1",
    cHarness: (headerName) => cHarness(headerName, shortCircuitBody),
    llvmHarness: llvmHarness(
      `bool and_short_circuit(int64_t a, int64_t b);
bool or_short_circuit(int64_t a, int64_t b);`,
      shortCircuitBody
    ),
    runWasm: (instance) => {
      const andShortCircuit = instance.exports.and_short_circuit as (a: bigint, b: bigint) => number;
      const orShortCircuit = instance.exports.or_short_circuit as (a: bigint, b: bigint) => number;
      return `short:and0=${andShortCircuit(0n, 10n)};and2=${andShortCircuit(2n, 10n)};or0=${orShortCircuit(0n, 10n)};or2=${orShortCircuit(2n, 10n)}`;
    }
  },
  {
    name: "memory",
    sourceFile: "examples/llvm_memory.ik",
    expected: "memory:first=1234;second=222;status=0;out=123",
    cHarness: (headerName) => cHarness(headerName, memoryBody),
    llvmHarness: llvmHarness(
      `${memoryTypes}

int64_t first_price(Item* items);
int64_t get_price(Item* items, int32_t i);
int32_t write_i64(int64_t* out, int64_t value);`,
      memoryBody
    ),
    runWasm: (instance) => {
      const memory = instance.exports.memory as WasmMemoryLike;
      const view = new DataView(memory.buffer);
      const firstPrice = instance.exports.first_price as (items: number) => bigint;
      const getPrice = instance.exports.get_price as (items: number, i: number) => bigint;
      const writeI64 = instance.exports.write_i64 as (out: number, value: bigint) => number;

      writeItem(view, 0, { price: 1234n, qty: 2n, discount: 3n, taxRatePpm: 4n });
      writeItem(view, itemSize, { price: 222n, qty: 0n, discount: 0n, taxRatePpm: 0n });
      const outOffset = 512;
      const status = writeI64(outOffset, 123n);
      return `memory:first=${firstPrice(0)};second=${getPrice(0, 1)};status=${status};out=${view.getBigInt64(outOffset, true)}`;
    }
  },
  {
    name: "f64_matrix",
    sourceFile: "tests/fixtures/f64_edges.ik",
    expected: f64MatrixExpected,
    cHarness: (headerName) => cHarness(headerName, f64MatrixBody),
    llvmHarness: llvmHarness(
      `${f64MatrixTypes}

double finite_add(void);
double finite_sub(void);
double finite_mul(void);
double finite_div(void);
double tolerance_calc(void);
bool finite_less(void);
bool finite_less_equal(void);
bool finite_equal(void);
double negative_infinity(void);
double positive_infinity(void);
double not_a_number(void);
double negative_zero(void);
bool zero_equals_negative_zero(void);
bool nan_equals_nan(void);
bool nan_not_equals_nan(void);
bool nan_less_than_one(void);
bool nan_less_equal_one(void);
bool nan_greater_than_one(void);
bool nan_greater_equal_one(void);
double infinity_plus_finite(void);
double infinity_minus_infinity(void);
double overflow_to_infinity(void);
double underflow_smoke(void);
bool infinity_greater_than_finite(void);
bool negative_infinity_less_than_finite(void);
double ptr_read(double* values, int32_t index);
double ptr_write(double* values, int32_t index, double value);
double struct_read(Quote* quotes, int32_t index);
double struct_write(Quote* quotes, int32_t index, double value);
double nested_struct_read(NestedQuote* nested, int32_t index);
double nested_struct_write(NestedQuote* nested, int32_t index, double value);`,
      f64MatrixBody
    ),
    runWasm: (instance) => {
      const memory = instance.exports.memory as WasmMemoryLike;
      const view = new DataView(memory.buffer);
      const finiteAdd = instance.exports.finite_add as () => number;
      const finiteSub = instance.exports.finite_sub as () => number;
      const finiteMul = instance.exports.finite_mul as () => number;
      const finiteDiv = instance.exports.finite_div as () => number;
      const toleranceCalc = instance.exports.tolerance_calc as () => number;
      const finiteLess = instance.exports.finite_less as () => number;
      const finiteLessEqual = instance.exports.finite_less_equal as () => number;
      const finiteEqual = instance.exports.finite_equal as () => number;
      const positiveInfinity = instance.exports.positive_infinity as () => number;
      const negativeInfinity = instance.exports.negative_infinity as () => number;
      const notANumber = instance.exports.not_a_number as () => number;
      const negativeZero = instance.exports.negative_zero as () => number;
      const zeroEqualsNegativeZero = instance.exports.zero_equals_negative_zero as () => number;
      const nanEqualsNan = instance.exports.nan_equals_nan as () => number;
      const nanNotEqualsNan = instance.exports.nan_not_equals_nan as () => number;
      const nanLessThanOne = instance.exports.nan_less_than_one as () => number;
      const nanLessEqualOne = instance.exports.nan_less_equal_one as () => number;
      const nanGreaterThanOne = instance.exports.nan_greater_than_one as () => number;
      const nanGreaterEqualOne = instance.exports.nan_greater_equal_one as () => number;
      const infinityPlusFinite = instance.exports.infinity_plus_finite as () => number;
      const infinityMinusInfinity = instance.exports.infinity_minus_infinity as () => number;
      const overflowToInfinity = instance.exports.overflow_to_infinity as () => number;
      const underflowSmoke = instance.exports.underflow_smoke as () => number;
      const infinityGreaterThanFinite = instance.exports.infinity_greater_than_finite as () => number;
      const negativeInfinityLessThanFinite = instance.exports.negative_infinity_less_than_finite as () => number;
      const ptrRead = instance.exports.ptr_read as (values: number, index: number) => number;
      const ptrWrite = instance.exports.ptr_write as (values: number, index: number, value: number) => number;
      const structRead = instance.exports.struct_read as (quotes: number, index: number) => number;
      const structWrite = instance.exports.struct_write as (quotes: number, index: number, value: number) => number;
      const nestedStructRead = instance.exports.nested_struct_read as (nested: number, index: number) => number;
      const nestedStructWrite = instance.exports.nested_struct_write as (nested: number, index: number, value: number) => number;

      const valuesOffset = 128;
      view.setFloat64(valuesOffset + 0, 1.0, true);
      view.setFloat64(valuesOffset + 8, 2.5, true);
      view.setFloat64(valuesOffset + 16, 4.0, true);
      const ptrStoreValue = ptrWrite(valuesOffset, 1, 8.75);

      const quotesOffset = 512;
      view.setFloat64(quotesOffset + 0, 10.25, true);
      view.setFloat64(quotesOffset + 8, 0.75, true);
      view.setFloat64(quotesOffset + 16, 20.5, true);
      view.setFloat64(quotesOffset + 24, 1.25, true);
      const structWriteValue = structWrite(quotesOffset, 1, 0.5);

      const nestedOffset = 1024;
      view.setFloat64(nestedOffset + 0, 1.25, true);
      view.setFloat64(nestedOffset + 8, 0.75, true);
      view.setFloat64(nestedOffset + 16, 2.0, true);
      view.setFloat64(nestedOffset + 24, 10.0, true);
      view.setFloat64(nestedOffset + 32, 2.0, true);
      view.setFloat64(nestedOffset + 40, 3.0, true);
      const nestedWriteValue = nestedStructWrite(nestedOffset, 1, 1.5);

      return formatF64MatrixResults([
        ["finite_add", closeF64(finiteAdd(), 4.0)],
        ["finite_sub", closeF64(finiteSub(), 3.5)],
        ["finite_mul", closeF64(finiteMul(), 3.75)],
        ["finite_div", closeF64(finiteDiv(), 3.5)],
        ["tolerance_calc", closeF64(toleranceCalc(), 10.0)],
        ["finite_less", finiteLess() === 1],
        ["finite_less_equal", finiteLessEqual() === 1],
        ["finite_equal", finiteEqual() === 1],
        ["pos_inf", classifyF64(positiveInfinity()) === "+inf"],
        ["neg_inf", classifyF64(negativeInfinity()) === "-inf"],
        ["nan", classifyF64(notANumber()) === "nan"],
        ["neg_zero", classifyF64(negativeZero()) === "-0"],
        ["zero_eq_neg_zero", zeroEqualsNegativeZero() === 1],
        ["nan_eq_nan", nanEqualsNan() === 0],
        ["nan_ne_nan", nanNotEqualsNan() === 1],
        ["nan_lt_one", nanLessThanOne() === 0],
        ["nan_le_one", nanLessEqualOne() === 0],
        ["nan_gt_one", nanGreaterThanOne() === 0],
        ["nan_ge_one", nanGreaterEqualOne() === 0],
        ["inf_plus", classifyF64(infinityPlusFinite()) === "+inf"],
        ["inf_minus_inf", classifyF64(infinityMinusInfinity()) === "nan"],
        ["overflow", classifyF64(overflowToInfinity()) === "+inf"],
        ["underflow", classifyF64(underflowSmoke()) === "+0"],
        ["inf_gt_finite", infinityGreaterThanFinite() === 1],
        ["neg_inf_lt_finite", negativeInfinityLessThanFinite() === 1],
        ["ptr_load", closeF64(ptrRead(valuesOffset, 2), 4.0)],
        ["ptr_store", closeF64(ptrStoreValue, 8.75) && closeF64(view.getFloat64(valuesOffset + 8, true), 8.75)],
        ["struct_read", closeF64(structRead(quotesOffset, 0), 11.0)],
        ["struct_write", closeF64(structWriteValue, 21.0) && closeF64(view.getFloat64(quotesOffset + 24, true), 0.5)],
        ["nested_struct_read", closeF64(nestedStructRead(nestedOffset, 0), 4.0)],
        ["nested_struct_write", closeF64(nestedWriteValue, 14.5) && closeF64(view.getFloat64(nestedOffset + 32, true), 1.5)]
      ]);
    }
  },
  {
    name: "pricing",
    sourceFile: "examples/pricing.ik",
    expected: "pricing:status=0;out0=20567;out1=11000",
    cHarness: (headerName) => cHarness(headerName, pricingBody),
    llvmHarness: llvmHarness(
      `${memoryTypes}

int32_t calc_items(Item* items, int32_t len, int64_t* out);`,
      pricingBody
    ),
    runWasm: (instance) => {
      const memory = instance.exports.memory as WasmMemoryLike;
      const view = new DataView(memory.buffer);
      const calcItems = instance.exports.calc_items as (items: number, len: number, out: number) => number;
      const itemsOffset = 0;
      const outOffset = 4096;
      const item0 = { price: 10000n, qty: 2n, discount: 1000n, taxRatePpm: 82500n };
      const item1 = { price: 2500n, qty: 4n, discount: 0n, taxRatePpm: 100000n };

      writeItem(view, itemsOffset, item0);
      writeItem(view, itemsOffset + itemSize, item1);
      const status = calcItems(itemsOffset, 2, outOffset);
      expect(view.getBigInt64(outOffset + 0, true)).toBe(calcExpected(item0));
      expect(view.getBigInt64(outOffset + 8, true)).toBe(calcExpected(item1));
      return `pricing:status=${status};out0=${view.getBigInt64(outOffset + 0, true)};out1=${view.getBigInt64(outOffset + 8, true)}`;
    }
  }
];

describe("LLVM/C/WASM backend regression comparison", () => {
  const nativeAvailable = hasClang();
  const wasmAvailable = supportsWasmI64BigInt();

  for (const fixture of fixtures) {
    it(`${fixture.name} behavior is consistent across supported backends`, async () => {
      const outputs: Partial<Record<"c" | "llvm" | "wasm", string>> = {};

      if (nativeAvailable) {
        outputs.c = runNativeCBackend(fixture);
        outputs.llvm = runNativeLlvmBackend(fixture);
      } else {
        console.warn("skipped C/LLVM native run because clang was not found");
      }

      if (wasmAvailable) {
        outputs.wasm = await runWasmBackend(fixture);
      } else {
        console.warn("skipped WASM run because Node.js WebAssembly i64 BigInt interop is unavailable");
      }

      expect(Object.keys(outputs).length).toBeGreaterThan(0);
      for (const output of Object.values(outputs)) {
        expect(output).toBe(fixture.expected);
      }

      if (outputs.c && outputs.llvm) {
        expect(outputs.llvm).toBe(outputs.c);
      }
      if (outputs.c && outputs.wasm) {
        expect(outputs.wasm).toBe(outputs.c);
      }
    });
  }
});
