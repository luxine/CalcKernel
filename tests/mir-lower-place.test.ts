import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { lowerToMir } from "../src/mir/lower.js";
import { printMirModule } from "../src/mir/mir-printer.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function lowerAndPrint(fileName: string, sourceText: string): string {
  const checked = check(new SourceFile(fileName, sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return printMirModule(mir);
}

describe("MIR place lowering", () => {
  it("lowers examples/pricing.ck with ptr index, field loads, and stores", () => {
    expect(lowerAndPrint("pricing.ck", readFileSync("examples/pricing.ck", "utf8"))).toMatchInlineSnapshot(`
      "struct Item {
        price: i64
        qty: i64
        discount: i64
        tax_rate_ppm: i64
      }

      export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
        local i: i32
        local subtotal: i64
        local after_discount: i64
        local tax: i64

      bb0:
        %t0: i32 = const_int 0
        i: i32 = move %t0
        jump bb1

      bb1:
        %t1: bool = lt i, len
        branch %t1, bb2, bb3

      bb2:
        %t2: i64 = load field(index(items, i), price)
        %t3: i64 = load field(index(items, i), qty)
        %t4: i64 = mul %t2, %t3
        subtotal: i64 = move %t4
        %t5: i64 = load field(index(items, i), discount)
        %t6: i64 = sub subtotal, %t5
        after_discount: i64 = move %t6
        %t7: i64 = load field(index(items, i), tax_rate_ppm)
        %t8: i64 = mul after_discount, %t7
        %t9: i64 = const_int 1000000
        %t10: i64 = div %t8, %t9
        tax: i64 = move %t10
        %t11: i64 = add after_discount, tax
        store index(out, i), %t11
        %t12: i32 = const_int 1
        %t13: i32 = add i, %t12
        i: i32 = move %t13
        jump bb1

      bb3:
        %t14: i32 = const_int 0
        return %t14
      }
      "
    `);
  });

  it("lowers arithmetic inside index expressions before loading the place", () => {
    expect(
      lowerAndPrint(
        "index-plus-one.ck",
        `
          struct Item {
            price: i64;
          }

          export fn read_next(items: ptr<Item>, i: i32) -> i64 {
            return items[i + 1].price;
          }
        `
      )
    ).toMatchInlineSnapshot(`
      "struct Item {
        price: i64
      }

      export fn read_next(items: ptr<Item>, i: i32) -> i64 {
      bb0:
        %t0: i32 = const_int 1
        %t1: i32 = add i, %t0
        %t2: i64 = load field(index(items, %t1), price)
        return %t2
      }
      "
    `);
  });

  it("lowers ptr<f64> and f64 struct field places", () => {
    expect(
      lowerAndPrint(
        "f64-place.ck",
        `
          struct Quote {
            price: f64;
            qty: i64;
          }

          export fn update(items: ptr<Quote>, out: ptr<f64>, i: i32) -> f64 {
            let price: f64 = items[i].price;
            out[i] = price;
            return out[i];
          }
        `
      )
    ).toMatchInlineSnapshot(`
      "struct Quote {
        price: f64
        qty: i64
      }

      export fn update(items: ptr<Quote>, out: ptr<f64>, i: i32) -> f64 {
        local price: f64

      bb0:
        %t0: f64 = load field(index(items, i), price)
        price: f64 = move %t0
        store index(out, i), price
        %t1: f64 = load index(out, i)
        return %t1
      }
      "
    `);
  });
});
