import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { emitMirWatModule } from "../src/backend/wasm/mir-wat-emitter.js";
import { lowerToMir } from "../src/mir/lower.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function normalizeNewlines(text: string): string {
  return text.replaceAll("\r\n", "\n");
}

function lowerAndEmitWat(sourceText: string): string {
  const checked = check(new SourceFile("scalar_wat.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return emitMirWatModule(mir);
}

describe("MIR WAT emitter scalar unchecked", () => {
  it("emits scalar straight-line MIR as stable WAT", () => {
    const wat = lowerAndEmitWat(`
      fn identity_i64(a: i64) -> i64 {
        return a;
      }

      export fn scalar_i32(a: i32, b: i32) -> i32 {
        let x: i32 = a + b;
        x = x - b;
        x = x * b;
        x = x / b;
        return x % b;
      }

      export fn add_i64(a: i64, b: i64) -> i64 {
        return a + b;
      }

      export fn less_i64(a: i64, b: i64) -> bool {
        return a < b;
      }

      export fn div_u64(a: u64, b: u64) -> u64 {
        return a / b;
      }

      export fn gt_u32(a: u32, b: u32) -> bool {
        return a > b;
      }

      export fn neg_i64(a: i64) -> i64 {
        return -a;
      }

      export fn not_bool(a: bool) -> bool {
        return !a;
      }

      export fn bool_true() -> bool {
        return true;
      }
    `);

    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/scalar.wat.snap", "utf8")));
  });

  it("emits f64 scalar arithmetic, unary neg, and comparisons as stable WAT", () => {
    const wat = lowerAndEmitWat(`
      export fn calc_f64(a: f64, b: f64) -> f64 {
        let one: f64 = 1.0;
        let sum: f64 = a + b;
        let diff: f64 = sum - one;
        let prod: f64 = diff * b;
        return prod / 2.0;
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
    `);

    expect(wat).toContain("(param $a f64)");
    expect(wat).toContain("(result f64)");
    expect(wat).toContain("f64.const 1.0");
    expect(wat).toContain("f64.const 2.0");
    expect(wat).toContain("f64.add");
    expect(wat).toContain("f64.sub");
    expect(wat).toContain("f64.mul");
    expect(wat).toContain("f64.div");
    expect(wat).toContain("f64.neg");
    expect(wat).toContain("f64.eq");
    expect(wat).toContain("f64.ne");
    expect(wat).toContain("f64.lt");
    expect(wat).toContain("f64.le");
    expect(wat).toContain("f64.gt");
    expect(wat).toContain("f64.ge");
    expect(wat).not.toContain("f64.div_s");
    expect(wat).not.toContain("f64.div_u");
    expect(wat).not.toContain("f64.rem");
    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/wasm_f64_scalar.wat.snap", "utf8")));
  });

  it("emits explicit i32/u32 to f64 casts with signed and unsigned WASM conversion opcodes", () => {
    const wat = lowerAndEmitWat(`
      export fn cast_i32(value: i32) -> f64 {
        return i32_to_f64(value);
      }

      export fn cast_u32(value: u32) -> f64 {
        return u32_to_f64(value);
      }

      export fn cast_expr(a: i32, b: u32) -> f64 {
        return i32_to_f64(a) + u32_to_f64(b);
      }
    `);

    expect(wat).toContain("f64.convert_i32_s");
    expect(wat).toContain("f64.convert_i32_u");
    expect(wat).toContain("f64.add");
    expect(wat).not.toContain("f64.convert_i64_s");
    expect(wat).not.toContain("f64.convert_i64_u");
    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/wasm_casts.wat.snap", "utf8")));
  });
});
