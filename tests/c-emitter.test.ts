import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { emitCFiles, emitDefaultCSource } from "../src/backend/c/c-build.js";
import { emitCHeader } from "../src/backend/c/c-header-emitter.js";
import { emitCSource } from "../src/backend/c/c-emitter.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function checkedSource(fileName: string, text: string) {
  const result = check(new SourceFile(fileName, text));
  expect(result.diagnostics).toEqual([]);
  return result;
}

function normalizeNewlines(text: string): string {
  return text.replaceAll("\r\n", "\n");
}

type SnapshotExample = "pricing" | "scalar";
type CheckedSnapshotExample = SnapshotExample | "scalar_checked" | "scalar_control_checked" | "scalar_logical_checked" | "scalar_calls_checked";

function expectGoldenSnapshot(exampleName: SnapshotExample): void {
  const sourceText = normalizeNewlines(readFileSync(`examples/${exampleName}.ik`, "utf8"));
  const checked = checkedSource(`${exampleName}.ik`, sourceText);
  const header = emitCHeader(checked);
  const source = emitDefaultCSource(checked, { headerFileName: `${exampleName}.h` });

  expect(header).toBe(normalizeNewlines(readFileSync(`tests/snapshots/${exampleName}.h.snap`, "utf8")));
  expect(source).toBe(normalizeNewlines(readFileSync(`tests/snapshots/${exampleName}.c.snap`, "utf8")));
}

function expectCheckedHeaderSnapshot(exampleName: CheckedSnapshotExample): void {
  const sourceText = normalizeNewlines(readFileSync(`examples/${exampleName}.ik`, "utf8"));
  const checked = checkedSource(`${exampleName}.ik`, sourceText);
  const header = emitCHeader(checked, { overflowMode: "checked" });

  expect(header).toBe(normalizeNewlines(readFileSync(`tests/snapshots/${exampleName}.checked.h.snap`, "utf8")));
}

function expectCheckedSourceSnapshot(exampleName: CheckedSnapshotExample): void {
  const sourceText = normalizeNewlines(readFileSync(`examples/${exampleName}.ik`, "utf8"));
  const checked = checkedSource(`${exampleName}.ik`, sourceText);
  const source = emitDefaultCSource(checked, { headerFileName: `${exampleName}.h`, overflowMode: "checked" });

  expect(source).toBe(normalizeNewlines(readFileSync(`tests/snapshots/${exampleName}.checked.c.snap`, "utf8")));
}

describe("c emitter", () => {
  it("matches external golden snapshots for pricing", () => {
    expectGoldenSnapshot("pricing");
  });

  it("matches external golden snapshots for scalar", () => {
    expectGoldenSnapshot("scalar");
  });

  it("matches checked header snapshot for pricing", () => {
    expectCheckedHeaderSnapshot("pricing");
  });

  it("matches checked source snapshot for pricing", () => {
    expectCheckedSourceSnapshot("pricing");
  });

  it("matches checked header snapshot for scalar", () => {
    expectCheckedHeaderSnapshot("scalar");
  });

  it("matches checked snapshots for scalar checked", () => {
    expectCheckedHeaderSnapshot("scalar_checked");
    expectCheckedSourceSnapshot("scalar_checked");
  });

  it("matches checked snapshots for scalar checked control flow", () => {
    expectCheckedHeaderSnapshot("scalar_control_checked");
    expectCheckedSourceSnapshot("scalar_control_checked");
  });

  it("matches checked snapshots for scalar checked logical short-circuit", () => {
    expectCheckedHeaderSnapshot("scalar_logical_checked");
    expectCheckedSourceSnapshot("scalar_logical_checked");
  });

  it("matches checked snapshots for scalar checked function calls", () => {
    expectCheckedHeaderSnapshot("scalar_calls_checked");
    expectCheckedSourceSnapshot("scalar_calls_checked");
  });

  it("emits golden C header and source for examples/pricing.ik", () => {
    const sourceText = readFileSync("examples/pricing.ik", "utf8");
    const checked = checkedSource("pricing.ik", sourceText);

    expect(emitCHeader(checked)).toMatchInlineSnapshot(`
      "#pragma once

      #include <stdint.h>
      #include <stdbool.h>

      #if defined(_WIN32) || defined(__CYGWIN__)
        #ifdef IK_BUILD_DLL
          #define IK_API __declspec(dllexport)
        #else
          #define IK_API __declspec(dllimport)
        #endif
      #else
        #define IK_API __attribute__((visibility("default")))
      #endif

      #ifdef __cplusplus
      extern "C" {
      #endif

      typedef struct Item {
        int64_t price;
        int64_t qty;
        int64_t discount;
        int64_t tax_rate_ppm;
      } Item;

      IK_API int32_t calc_items(Item* items, int32_t len, int64_t* out);

      #ifdef __cplusplus
      }
      #endif
      "
    `);

    expect(emitDefaultCSource(checked, { headerFileName: "pricing.h" })).toMatchInlineSnapshot(`
      "#include "pricing.h"

      int32_t calc_items(Item* items, int32_t len, int64_t* out) {
        int32_t i;
        int64_t subtotal;
        int64_t after_discount;
        int64_t tax;
        int32_t ik_tmp0;
        bool ik_tmp1;
        int64_t ik_tmp2;
        int64_t ik_tmp3;
        int64_t ik_tmp4;
        int64_t ik_tmp5;
        int64_t ik_tmp6;
        int64_t ik_tmp7;
        int64_t ik_tmp8;
        int64_t ik_tmp9;
        int64_t ik_tmp10;
        int64_t ik_tmp11;
        int32_t ik_tmp12;
        int32_t ik_tmp13;
        int32_t ik_tmp14;

        ik_tmp0 = 0;
        i = ik_tmp0;
        goto bb1;

      bb1:
        ik_tmp1 = i < len;
        if (ik_tmp1) {
          goto bb2;
        } else {
          goto bb3;
        }

      bb2:
        ik_tmp2 = items[i].price;
        ik_tmp3 = items[i].qty;
        ik_tmp4 = ik_tmp2 * ik_tmp3;
        subtotal = ik_tmp4;
        ik_tmp5 = items[i].discount;
        ik_tmp6 = subtotal - ik_tmp5;
        after_discount = ik_tmp6;
        ik_tmp7 = items[i].tax_rate_ppm;
        ik_tmp8 = after_discount * ik_tmp7;
        ik_tmp9 = 1000000;
        ik_tmp10 = ik_tmp8 / ik_tmp9;
        tax = ik_tmp10;
        ik_tmp11 = after_discount + tax;
        out[i] = ik_tmp11;
        ik_tmp12 = 1;
        ik_tmp13 = i + ik_tmp12;
        i = ik_tmp13;
        goto bb1;

      bb3:
        ik_tmp14 = 0;
        return ik_tmp14;
      }
      "
    `);
  });

  it("only declares exported functions in the header and emits private functions as static", () => {
    const checked = checkedSource(
      "helper.ik",
      `
        fn helper(value: i64) -> i64 {
          return value + 1;
        }

        export fn public_entry(value: i64) -> i64 {
          return helper(value);
        }
      `
    );

    expect(emitCHeader(checked)).toContain("IK_API int64_t public_entry(int64_t value);");
    expect(emitCHeader(checked)).not.toContain("helper");
    expect(emitDefaultCSource(checked, { headerFileName: "helper.h" })).toContain("static int64_t helper(int64_t value)");
  });

  it("requires a clean type checked result", () => {
    const checked = check(new SourceFile("bad.ik", "export fn bad() -> i32 { return missing; }"));

    expect(() => emitCHeader(checked)).toThrow("Cannot emit C for a program with diagnostics.");
    expect(() => emitDefaultCSource(checked, { headerFileName: "bad.h" })).toThrow("Cannot emit C for a program with diagnostics.");
  });

  it("accepts explicit unchecked overflow mode without changing output", () => {
    const checked = checkedSource("scalar.ik", "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");

    expect(emitCHeader(checked, { overflowMode: "unchecked" })).toBe(emitCHeader(checked));
    expect(emitDefaultCSource(checked, { headerFileName: "scalar.h", overflowMode: "unchecked" })).toBe(
      emitDefaultCSource(checked, { headerFileName: "scalar.h" })
    );
  });

  it("emits checked scalar C for supported scalar statements and expressions", () => {
    const checked = checkedSource("scalar.ik", "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");

    expect(emitCHeader(checked, { overflowMode: "checked" })).toContain("IK_API IK_Status add(int64_t a, int64_t b, int64_t* ik_return);");
    expect(emitDefaultCSource(checked, { headerFileName: "scalar.h", overflowMode: "checked" })).toContain("__builtin_add_overflow");
    expect(() =>
      emitCFiles(checked, {
        cFile: "build/checked/scalar.c",
        headerFile: "build/checked/scalar.h",
        headerFileName: "scalar.h",
        overflowMode: "checked"
      })
    ).not.toThrow();
  });

  it("emits checked scalar C for let and unary not", () => {
    const checked = checkedSource(
      "scalar-extra.ik",
      `
        export fn calc(a: i64, b: i64) -> bool {
          let sum: i64 = a + b;
          return !(sum < 0);
        }
      `
    );
    const source = emitDefaultCSource(checked, { headerFileName: "scalar-extra.h", overflowMode: "checked" });

    expect(source).toContain("int64_t sum;");
    expect(source).toContain("__builtin_add_overflow(a, b, &ik_tmp0)");
    expect(source).toContain("bool ik_tmp");
    expect(source).toContain(" = !");
  });

  it("keeps checked induction overflow checks at O0 and removes only proven-safe increments at O3", () => {
    const checked = checkedSource(
      "safe-induction.ik",
      `
        export fn fill(out: ptr<i64>, len: i32) -> i32 {
          let i: i32 = 0;
          while i < len {
            out[i] = 0;
            i = i + 1;
          }
          return 0;
        }
      `
    );

    const o0 = emitDefaultCSource(checked, { headerFileName: "safe-induction.h", overflowMode: "checked", optLevel: 0 });
    const o3 = emitDefaultCSource(checked, { headerFileName: "safe-induction.h", overflowMode: "checked", optLevel: 3 });

    expect(o0).toContain("__builtin_add_overflow(i,");
    expect(o3).not.toContain("__builtin_add_overflow(i,");
    expect(o3).toMatch(/ik_tmp\d+ = i \+ ik_tmp\d+;/);
  });

  it("does not remove checked induction overflow checks without the safe loop proof", () => {
    const checked = checkedSource(
      "unsafe-induction.ik",
      `
        export fn fill(out: ptr<i64>, len: i32) -> i32 {
          let i: i32 = 1;
          while i < len {
            out[i] = 0;
            i = i + 1;
          }
          return 0;
        }
      `
    );

    const source = emitDefaultCSource(checked, { headerFileName: "unsafe-induction.h", overflowMode: "checked", optLevel: 3 });

    expect(source).toContain("__builtin_add_overflow(i,");
  });

  describe("expression precedence", () => {
    it.each([
      {
        name: "multiplication before addition",
        source: `
          export fn calc() -> i32 {
            return 1 + 2 * 3;
          }
        `,
        expectedReturn: "return (1 + (2 * 3));"
      },
      {
        name: "parentheses before multiplication",
        source: `
          export fn calc() -> i32 {
            return (1 + 2) * 3;
          }
        `,
        expectedReturn: "return (((1 + 2)) * 3);"
      },
      {
        name: "comparison below addition",
        source: `
          export fn calc(a: i32, b: i32, c: i32, d: i32) -> bool {
            return a + b < c + d;
          }
        `,
        expectedReturn: "return ((a + b) < (c + d));"
      },
      {
        name: "logical and below comparison",
        source: `
          export fn calc(a: i32, b: i32, c: i32, d: i32) -> bool {
            return a < b && c < d;
          }
        `,
        expectedReturn: "return ((a < b) && (c < d));"
      },
      {
        name: "logical or below logical and",
        source: `
          export fn calc(a: bool, b: bool, c: bool) -> bool {
            return a || b && c;
          }
        `,
        expectedReturn: "return (a || (b && c));"
      },
      {
        name: "logical not before logical or",
        source: `
          export fn calc(a: bool, b: bool) -> bool {
            return !a || b;
          }
        `,
        expectedReturn: "return ((!a) || b);"
      },
      {
        name: "unary minus before multiplication",
        source: `
          export fn calc(a: i32, b: i32) -> i32 {
            return -a * b;
          }
        `,
        expectedReturn: "return ((-a) * b);"
      },
      {
        name: "field and index access before multiplication",
        source: `
          struct Item {
            price: i64;
            qty: i64;
          }

          export fn calc(items: ptr<Item>) -> i64 {
            return items[0].price * items[0].qty;
          }
        `,
        expectedReturn: "return (items[0].price * items[0].qty);"
      },
      {
        name: "combined index and field access",
        source: `
          struct Item {
            price: i64;
            qty: i64;
          }

          export fn calc(items: ptr<Item>, i: i32) -> i64 {
            return items[i].price + items[i].qty;
          }
        `,
        expectedReturn: "return (items[i].price + items[i].qty);"
      },
      {
        name: "complex access and arithmetic composition",
        source: `
          struct Item {
            price: i64;
            qty: i64;
            discount: i64;
          }

          export fn calc(items: ptr<Item>, i: i32) -> i64 {
            return items[i].price * items[i].qty - items[i].discount;
          }
        `,
        expectedReturn: "return ((items[i].price * items[i].qty) - items[i].discount);"
      }
    ])("emits C preserving $name", ({ name, source, expectedReturn }) => {
      const checked = checkedSource(`${name}.ik`, source);
      const emitted = emitCSource(checked, { headerFileName: "calc.h" });

      expect(emitted).toContain(expectedReturn);
    });
  });
});
