import { describe, expect, it } from "vitest";
import { emitMirCSource } from "../src/backend/c/mir-c-emitter.js";
import { lowerToMir } from "../src/mir/lower.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function lowerAndEmit(sourceText: string): string {
  const checked = check(new SourceFile("test.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return emitMirCSource(mir, { headerFileName: "test.h" });
}

describe("MIR C emitter scalar unchecked", () => {
  it("emits scalar straight-line MIR as readable unchecked C", () => {
    expect(
      lowerAndEmit(`
        fn identity_i64(a: i64) -> i64 {
          return a;
        }

        export fn add_i64(a: i64, b: i64) -> i64 {
          let x: i64 = a + b;
          return x;
        }

        export fn mul_i64(a: i64, b: i64) -> i64 {
          return a * b;
        }

        export fn less_i64(a: i64, b: i64) -> bool {
          return a < b;
        }

        export fn neg_i64(a: i64) -> i64 {
          return -a;
        }

        export fn not_bool(a: bool) -> bool {
          return !a;
        }
      `)
    ).toMatchInlineSnapshot(`
      "#include "test.h"

      static int64_t identity_i64(int64_t a) {
        return a;
      }

      int64_t add_i64(int64_t a, int64_t b) {
        int64_t x;
        int64_t ik_tmp0;

        ik_tmp0 = a + b;
        x = ik_tmp0;
        return x;
      }

      int64_t mul_i64(int64_t a, int64_t b) {
        int64_t ik_tmp0;

        ik_tmp0 = a * b;
        return ik_tmp0;
      }

      bool less_i64(int64_t a, int64_t b) {
        bool ik_tmp0;

        ik_tmp0 = a < b;
        return ik_tmp0;
      }

      int64_t neg_i64(int64_t a) {
        int64_t ik_tmp0;

        ik_tmp0 = -a;
        return ik_tmp0;
      }

      bool not_bool(bool a) {
        bool ik_tmp0;

        ik_tmp0 = !a;
        return ik_tmp0;
      }
      "
    `);
  });

  it("emits unchecked C calls for nested MIR call expressions", () => {
    expect(
      lowerAndEmit(`
        fn add_i64(a: i64, b: i64) -> i64 {
          return a + b;
        }

        fn double_i64(a: i64) -> i64 {
          return a * 2;
        }

        export fn calc(a: i64, b: i64) -> i64 {
          return double_i64(add_i64(a, b));
        }
      `)
    ).toMatchInlineSnapshot(`
      "#include "test.h"

      static int64_t add_i64(int64_t a, int64_t b) {
        int64_t ik_tmp0;

        ik_tmp0 = a + b;
        return ik_tmp0;
      }

      static int64_t double_i64(int64_t a) {
        int64_t ik_tmp0;
        int64_t ik_tmp1;

        ik_tmp0 = 2;
        ik_tmp1 = a * ik_tmp0;
        return ik_tmp1;
      }

      int64_t calc(int64_t a, int64_t b) {
        int64_t ik_tmp0;
        int64_t ik_tmp1;

        ik_tmp0 = add_i64(a, b);
        ik_tmp1 = double_i64(ik_tmp0);
        return ik_tmp1;
      }
      "
    `);
  });

  it("emits f64 MIR as ordinary C double operations", () => {
    const source = lowerAndEmit(`
      struct Quote {
        price: f64;
        tax: f64;
      }

      export fn calc_f64(a: f64, b: f64, quotes: ptr<Quote>, out: ptr<f64>) -> bool {
        let one: f64 = 1.0;
        let sum: f64 = a + b;
        let adjusted: f64 = sum - one;
        let scaled: f64 = adjusted * quotes[0].price;
        out[1] = scaled / quotes[0].tax;
        return -out[1] <= 0.0;
      }
    `);

    expect(source).toContain("bool calc_f64(double a, double b, Quote* quotes, double* out)");
    expect(source).toContain("double one;");
    expect(source).toContain("ik_tmp0 = 1.0;");
    expect(source).toContain("ik_tmp1 = a + b;");
    expect(source).toContain("ik_tmp4 = quotes[ik_tmp3].price;");
    expect(source).toContain("ik_tmp5 = adjusted * ik_tmp4;");
    expect(source).toContain("out[ik_tmp");
    expect(source).toContain("ik_tmp");
    expect(source).toContain(" <= ");
  });

  it("emits explicit int to f64 MIR casts as C double casts", () => {
    const source = lowerAndEmit(`
      export fn cast_i32(value: i32) -> f64 {
        return i32_to_f64(value);
      }

      export fn cast_u32(value: u32) -> f64 {
        return u32_to_f64(value);
      }

      export fn cast_expr(a: i32, b: u32, limit: f64) -> bool {
        let sum: f64 = i32_to_f64(a) + u32_to_f64(b);
        return sum <= limit;
      }
    `);

    expect(source).toContain("double cast_i32(int32_t value)");
    expect(source).toContain("double cast_u32(uint32_t value)");
    expect(source).toContain("bool cast_expr(int32_t a, uint32_t b, double limit)");
    expect(source).toContain("ik_tmp0 = (double)value;");
    expect(source).toContain("ik_tmp0 = (double)a;");
    expect(source).toContain("ik_tmp1 = (double)b;");
    expect(source).toContain("ik_tmp2 = ik_tmp0 + ik_tmp1;");
    expect(source).toContain("ik_tmp3 = sum <= limit;");
  });
});
