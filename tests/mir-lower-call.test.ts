import { describe, expect, it } from "vitest";
import { lowerToMir } from "../src/mir/lower.js";
import { printMirModule } from "../src/mir/mir-printer.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function lowerAndPrint(sourceText: string): string {
  const checked = check(new SourceFile("test.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return printMirModule(mir);
}

describe("MIR function call lowering", () => {
  it("lowers simple, nested, and argument-expression calls", () => {
    expect(
      lowerAndPrint(`
        fn add_i64(a: i64, b: i64) -> i64 {
          return a + b;
        }

        fn double_i64(a: i64) -> i64 {
          return a * 2;
        }

        export fn simple(a: i64, b: i64) -> i64 {
          return add_i64(a, b);
        }

        export fn calc(a: i64, b: i64) -> i64 {
          return double_i64(add_i64(a, b));
        }

        export fn call_args(a: i64, b: i64) -> i64 {
          return add_i64(a + 1, b * 2);
        }
      `)
    ).toMatchInlineSnapshot(`
      "fn add_i64(a: i64, b: i64) -> i64 {
      bb0:
        %t0: i64 = add a, b
        return %t0
      }

      fn double_i64(a: i64) -> i64 {
      bb0:
        %t0: i64 = const_int 2
        %t1: i64 = mul a, %t0
        return %t1
      }

      export fn simple(a: i64, b: i64) -> i64 {
      bb0:
        %t0: i64 = call add_i64(a, b)
        return %t0
      }

      export fn calc(a: i64, b: i64) -> i64 {
      bb0:
        %t0: i64 = call add_i64(a, b)
        %t1: i64 = call double_i64(%t0)
        return %t1
      }

      export fn call_args(a: i64, b: i64) -> i64 {
      bb0:
        %t0: i64 = const_int 1
        %t1: i64 = add a, %t0
        %t2: i64 = const_int 2
        %t3: i64 = mul b, %t2
        %t4: i64 = call add_i64(%t1, %t3)
        return %t4
      }
      "
    `);
  });

  it("lowers explicit int to f64 compiler builtins to MIR casts", () => {
    expect(
      lowerAndPrint(`
        export fn from_i32(n: i32) -> f64 {
          return i32_to_f64(n);
        }

        export fn from_u32(n: u32) -> f64 {
          let x: f64 = u32_to_f64(n);
          return x + 1.0;
        }

        export fn literal_casts() -> f64 {
          return i32_to_f64(1) + u32_to_f64(2);
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn from_i32(n: i32) -> f64 {
      bb0:
        %t0: f64 = cast i32_to_f64 n
        return %t0
      }

      export fn from_u32(n: u32) -> f64 {
        local x: f64

      bb0:
        %t0: f64 = cast u32_to_f64 n
        x: f64 = move %t0
        %t1: f64 = const_float 1.0
        %t2: f64 = add x, %t1
        return %t2
      }

      export fn literal_casts() -> f64 {
      bb0:
        %t0: i32 = const_int 1
        %t1: f64 = cast i32_to_f64 %t0
        %t2: u32 = const_int 2
        %t3: f64 = cast u32_to_f64 %t2
        %t4: f64 = add %t1, %t3
        return %t4
      }
      "
    `);
  });
});
