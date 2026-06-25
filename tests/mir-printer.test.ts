import { describe, expect, it } from "vitest";
import { printMirModule } from "../src/mir/mir-printer.js";
import type { MirModule, MirType } from "../src/mir/mir.js";

const i32: MirType = { kind: "primitive", name: "i32" };
const i64: MirType = { kind: "primitive", name: "i64" };
const f64: MirType = { kind: "primitive", name: "f64" };

describe("MIR printer", () => {
  it("prints structs and functions in a stable text format", () => {
    const module: MirModule = {
      structs: [
        {
          name: "Item",
          fields: [
            { name: "price", type: i64 },
            { name: "qty", type: i64 }
          ]
        }
      ],
      functions: [
        {
          name: "add_i64",
          exported: false,
          params: [
            { name: "a", type: i64 },
            { name: "b", type: i64 }
          ],
          returnType: i64,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                {
                  kind: "binary",
                  target: { kind: "temp", name: "t0", type: i64 },
                  op: "+",
                  left: { kind: "param", name: "a", type: i64 },
                  right: { kind: "param", name: "b", type: i64 }
                }
              ],
              terminator: {
                kind: "return",
                value: { kind: "temp", name: "t0", type: i64 }
              }
            }
          ]
        },
        {
          name: "max_i32",
          exported: true,
          params: [
            { name: "a", type: i32 },
            { name: "b", type: i32 }
          ],
          returnType: i32,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                {
                  kind: "compare",
                  target: { kind: "temp", name: "t0", type: { kind: "primitive", name: "bool" } },
                  op: ">",
                  left: { kind: "param", name: "a", type: i32 },
                  right: { kind: "param", name: "b", type: i32 }
                }
              ],
              terminator: {
                kind: "branch",
                condition: { kind: "temp", name: "t0", type: { kind: "primitive", name: "bool" } },
                thenLabel: "bb1",
                elseLabel: "bb2"
              }
            },
            {
              label: "bb1",
              instructions: [],
              terminator: {
                kind: "return",
                value: { kind: "param", name: "a", type: i32 }
              }
            },
            {
              label: "bb2",
              instructions: [],
              terminator: {
                kind: "return",
                value: { kind: "param", name: "b", type: i32 }
              }
            }
          ]
        }
      ]
    };

    expect(printMirModule(module)).toMatchInlineSnapshot(`
      "struct Item {
        price: i64
        qty: i64
      }

      fn add_i64(a: i64, b: i64) -> i64 {
      bb0:
        %t0: i64 = add a, b
        return %t0
      }

      export fn max_i32(a: i32, b: i32) -> i32 {
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

  it("prints const_float with source-stable text", () => {
    const module: MirModule = {
      structs: [],
      functions: [
        {
          name: "literal_f64",
          exported: true,
          params: [],
          returnType: f64,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                {
                  kind: "const_float",
                  target: { kind: "temp", name: "t0", type: f64 },
                  value: "1.0e-3"
                }
              ],
              terminator: {
                kind: "return",
                value: { kind: "temp", name: "t0", type: f64 }
              }
            }
          ]
        }
      ]
    };

    expect(printMirModule(module)).toMatchInlineSnapshot(`
      "export fn literal_f64() -> f64 {
      bb0:
        %t0: f64 = const_float 1.0e-3
        return %t0
      }
      "
    `);
  });

  it("prints explicit int to f64 casts in a stable text format", () => {
    const module: MirModule = {
      structs: [],
      functions: [
        {
          name: "from_i32",
          exported: true,
          params: [{ name: "a", type: i32 }],
          returnType: f64,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                {
                  kind: "cast",
                  target: { kind: "temp", name: "t0", type: f64 },
                  op: "i32_to_f64",
                  value: { kind: "param", name: "a", type: i32 }
                }
              ],
              terminator: {
                kind: "return",
                value: { kind: "temp", name: "t0", type: f64 }
              }
            }
          ]
        }
      ]
    };

    expect(printMirModule(module)).toMatchInlineSnapshot(`
      "export fn from_i32(a: i32) -> f64 {
      bb0:
        %t0: f64 = cast i32_to_f64 a
        return %t0
      }
      "
    `);
  });
});
