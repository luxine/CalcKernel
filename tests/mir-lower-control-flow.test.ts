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

describe("MIR control flow lowering", () => {
  it("lowers if else returns to branch blocks without a join", () => {
    expect(
      lowerAndPrint(`
        export fn max_i32(a: i32, b: i32) -> i32 {
          if a > b {
            return a;
          } else {
            return b;
          }
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn max_i32(a: i32, b: i32) -> i32 {
      bb0:
        %t0: bool = gt a, b
        branch %t0, bb1, bb2

      bb1:
        return a

      bb2:
        return b
      }
      "
    `);
  });

  it("lowers if without else to then and join blocks", () => {
    expect(
      lowerAndPrint(`
        export fn positive_or_zero(a: i32) -> i32 {
          if a > 0 {
            return a;
          }
          return 0;
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn positive_or_zero(a: i32) -> i32 {
      bb0:
        %t0: i32 = const_int 0
        %t1: bool = gt a, %t0
        branch %t1, bb1, bb2

      bb1:
        return a

      bb2:
        %t2: i32 = const_int 0
        return %t2
      }
      "
    `);
  });

  it("lowers while loops to condition, body, and exit blocks", () => {
    expect(
      lowerAndPrint(`
        export fn sum_to_n(n: i64) -> i64 {
          let i: i64 = 0;
          let sum: i64 = 0;

          while i < n {
            sum = sum + i;
            i = i + 1;
          }

          return sum;
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn sum_to_n(n: i64) -> i64 {
        local i: i64
        local sum: i64

      bb0:
        %t0: i64 = const_int 0
        i: i64 = move %t0
        %t1: i64 = const_int 0
        sum: i64 = move %t1
        jump bb1

      bb1:
        %t2: bool = lt i, n
        branch %t2, bb2, bb3

      bb2:
        %t3: i64 = add sum, i
        sum: i64 = move %t3
        %t4: i64 = const_int 1
        %t5: i64 = add i, %t4
        i: i64 = move %t5
        jump bb1

      bb3:
        return sum
      }
      "
    `);
  });

  it("does not add a jump after a return inside while body control flow", () => {
    expect(
      lowerAndPrint(`
        export fn first_positive_or_zero(n: i64) -> i64 {
          let i: i64 = 0;

          while i < n {
            if i > 0 {
              return i;
            }
            i = i + 1;
          }

          return 0;
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn first_positive_or_zero(n: i64) -> i64 {
        local i: i64

      bb0:
        %t0: i64 = const_int 0
        i: i64 = move %t0
        jump bb1

      bb1:
        %t1: bool = lt i, n
        branch %t1, bb2, bb3

      bb2:
        %t2: i64 = const_int 0
        %t3: bool = gt i, %t2
        branch %t3, bb4, bb5

      bb4:
        return i

      bb5:
        %t4: i64 = const_int 1
        %t5: i64 = add i, %t4
        i: i64 = move %t5
        jump bb1

      bb3:
        %t6: i64 = const_int 0
        return %t6
      }
      "
    `);
  });
});
