import { describe, expect, it } from "vitest";
import { emitMirCSource } from "../src/backend/c/mir-c-emitter.js";
import { lowerToMir } from "../src/mir/lower.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function lowerAndEmit(sourceText: string): string {
  const checked = check(new SourceFile("test.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return emitMirCSource(mir, { headerFileName: "test.h" });
}

describe("MIR C emitter unchecked control flow", () => {
  it("emits goto-style C for if/else and while MIR blocks", () => {
    expect(
      lowerAndEmit(`
        export fn max_i32(a: i32, b: i32) -> i32 {
          if a > b {
            return a;
          } else {
            return b;
          }
        }

        export fn positive_or_zero(a: i32) -> i32 {
          if a > 0 {
            return a;
          }
          return 0;
        }

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
      "#include "test.h"

      int32_t max_i32(int32_t a, int32_t b) {
        bool ik_tmp0;

        ik_tmp0 = a > b;
        if (ik_tmp0) {
          goto bb1;
        } else {
          goto bb2;
        }

      bb1:
        return a;

      bb2:
        return b;
      }

      int32_t positive_or_zero(int32_t a) {
        int32_t ik_tmp0;
        bool ik_tmp1;
        int32_t ik_tmp2;

        ik_tmp0 = 0;
        ik_tmp1 = a > ik_tmp0;
        if (ik_tmp1) {
          goto bb1;
        } else {
          goto bb2;
        }

      bb1:
        return a;

      bb2:
        ik_tmp2 = 0;
        return ik_tmp2;
      }

      int64_t sum_to_n(int64_t n) {
        int64_t i;
        int64_t sum;
        int64_t ik_tmp0;
        int64_t ik_tmp1;
        bool ik_tmp2;
        int64_t ik_tmp3;
        int64_t ik_tmp4;
        int64_t ik_tmp5;

        ik_tmp0 = 0;
        i = ik_tmp0;
        ik_tmp1 = 0;
        sum = ik_tmp1;
        goto bb1;

      bb1:
        ik_tmp2 = i < n;
        if (ik_tmp2) {
          goto bb2;
        } else {
          goto bb3;
        }

      bb2:
        ik_tmp3 = sum + i;
        sum = ik_tmp3;
        ik_tmp4 = 1;
        ik_tmp5 = i + ik_tmp4;
        i = ik_tmp5;
        goto bb1;

      bb3:
        return sum;
      }
      "
    `);
  });
});
