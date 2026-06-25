import { describe, expect, it } from "vitest";
import { lowerToMir } from "../src/mir/lower.js";
import { printMirModule } from "../src/mir/mir-printer.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function lowerAndPrint(sourceText: string): string {
  const checked = check(new SourceFile("test.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return printMirModule(mir);
}

describe("MIR scalar lowering", () => {
  it("lowers scalar straight-line functions to typed MIR", () => {
    expect(
      lowerAndPrint(`
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

        export fn literal_i32() -> i32 {
          let x: i32 = 1;
          return x;
        }

        export fn assign_i64(a: i64, b: i64) -> i64 {
          let x: i64 = a;
          x = b - 1;
          return x;
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn add_i64(a: i64, b: i64) -> i64 {
        local x: i64

      bb0:
        %t0: i64 = add a, b
        x: i64 = move %t0
        return x
      }

      export fn mul_i64(a: i64, b: i64) -> i64 {
      bb0:
        %t0: i64 = mul a, b
        return %t0
      }

      export fn less_i64(a: i64, b: i64) -> bool {
      bb0:
        %t0: bool = lt a, b
        return %t0
      }

      export fn neg_i64(a: i64) -> i64 {
      bb0:
        %t0: i64 = neg a
        return %t0
      }

      export fn not_bool(a: bool) -> bool {
      bb0:
        %t0: bool = not a
        return %t0
      }

      export fn literal_i32() -> i32 {
        local x: i32

      bb0:
        %t0: i32 = const_int 1
        x: i32 = move %t0
        return x
      }

      export fn assign_i64(a: i64, b: i64) -> i64 {
        local x: i64

      bb0:
        x: i64 = move a
        %t0: i64 = const_int 1
        %t1: i64 = sub b, %t0
        x: i64 = move %t1
        return x
      }
      "
    `);
  });

  it("lowers f64 literals, arithmetic, unary minus, and comparisons", () => {
    expect(
      lowerAndPrint(`
        export fn literal_f64() -> f64 {
          return 1.0;
        }

        export fn arithmetic_f64(a: f64, b: f64) -> f64 {
          let x: f64 = a + b;
          let y: f64 = x - a;
          let z: f64 = y * b;
          return z / b;
        }

        export fn neg_f64(a: f64) -> f64 {
          return -a;
        }

        export fn eq_f64(a: f64, b: f64) -> bool {
          return a == b;
        }

        export fn ne_f64(a: f64, b: f64) -> bool {
          return a != b;
        }

        export fn lt_f64(a: f64, b: f64) -> bool {
          return a < b;
        }

        export fn le_f64(a: f64, b: f64) -> bool {
          return a <= b;
        }

        export fn gt_f64(a: f64, b: f64) -> bool {
          return a > b;
        }

        export fn ge_f64(a: f64, b: f64) -> bool {
          return a >= b;
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn literal_f64() -> f64 {
      bb0:
        %t0: f64 = const_float 1.0
        return %t0
      }

      export fn arithmetic_f64(a: f64, b: f64) -> f64 {
        local x: f64
        local y: f64
        local z: f64

      bb0:
        %t0: f64 = add a, b
        x: f64 = move %t0
        %t1: f64 = sub x, a
        y: f64 = move %t1
        %t2: f64 = mul y, b
        z: f64 = move %t2
        %t3: f64 = div z, b
        return %t3
      }

      export fn neg_f64(a: f64) -> f64 {
      bb0:
        %t0: f64 = neg a
        return %t0
      }

      export fn eq_f64(a: f64, b: f64) -> bool {
      bb0:
        %t0: bool = eq a, b
        return %t0
      }

      export fn ne_f64(a: f64, b: f64) -> bool {
      bb0:
        %t0: bool = ne a, b
        return %t0
      }

      export fn lt_f64(a: f64, b: f64) -> bool {
      bb0:
        %t0: bool = lt a, b
        return %t0
      }

      export fn le_f64(a: f64, b: f64) -> bool {
      bb0:
        %t0: bool = le a, b
        return %t0
      }

      export fn gt_f64(a: f64, b: f64) -> bool {
      bb0:
        %t0: bool = gt a, b
        return %t0
      }

      export fn ge_f64(a: f64, b: f64) -> bool {
      bb0:
        %t0: bool = ge a, b
        return %t0
      }
      "
    `);
  });
});
