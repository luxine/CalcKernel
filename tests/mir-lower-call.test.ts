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
});
