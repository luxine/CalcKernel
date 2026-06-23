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
});
