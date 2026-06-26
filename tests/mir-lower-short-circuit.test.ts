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

describe("MIR short-circuit lowering", () => {
  it("lowers logical and with RHS evaluation in a separate block", () => {
    expect(
      lowerAndPrint(`
        export fn and_short_circuit(a: i64, b: i64) -> bool {
          return a != 0 && b / a > 1;
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn and_short_circuit(a: i64, b: i64) -> bool {
        local ik_sc0: bool

      bb0:
        %t0: i64 = const_int 0
        %t1: bool = ne a, %t0
        branch %t1, bb1, bb2

      bb1:
        %t2: i64 = div b, a
        %t3: i64 = const_int 1
        %t4: bool = gt %t2, %t3
        ik_sc0: bool = move %t4
        jump bb3

      bb2:
        ik_sc0: bool = move false
        jump bb3

      bb3:
        return ik_sc0
      }
      "
    `);
  });

  it("lowers logical or with RHS evaluation in a separate block", () => {
    expect(
      lowerAndPrint(`
        export fn or_short_circuit(a: i64, b: i64) -> bool {
          return a == 0 || b / a > 1;
        }
      `)
    ).toMatchInlineSnapshot(`
      "export fn or_short_circuit(a: i64, b: i64) -> bool {
        local ik_sc0: bool

      bb0:
        %t0: i64 = const_int 0
        %t1: bool = eq a, %t0
        branch %t1, bb1, bb2

      bb1:
        ik_sc0: bool = move true
        jump bb3

      bb2:
        %t2: i64 = div b, a
        %t3: i64 = const_int 1
        %t4: bool = gt %t2, %t3
        ik_sc0: bool = move %t4
        jump bb3

      bb3:
        return ik_sc0
      }
      "
    `);
  });
});
