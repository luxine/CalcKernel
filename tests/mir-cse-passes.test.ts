import { describe, expect, it } from "vitest";
import { emitMirCSource } from "../src/backend/c/mir-c-emitter.js";
import { lowerToMir } from "../src/mir/lower.js";
import type { MirFunction, MirModule, MirType, MirValue } from "../src/mir/mir.js";
import { printMirModule } from "../src/mir/mir-printer.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { runMirPassPipeline } from "../src/opt/mir-pass-manager.js";
import { buildMirOptimizationPipeline } from "../src/opt/pipeline.js";
import { addressCsePass } from "../src/opt/passes/address-cse.js";
import { localCsePass } from "../src/opt/passes/local-cse.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

const i64: MirType = { kind: "primitive", name: "i64" };
const i32: MirType = { kind: "primitive", name: "i32" };
const u32: MirType = { kind: "primitive", name: "u32" };
const f64: MirType = { kind: "primitive", name: "f64" };
const boolType: MirType = { kind: "primitive", name: "bool" };

function param(name: string, type: MirType): MirValue {
  return { kind: "param", name, type };
}

function temp(name: string, type: MirType): MirValue {
  return { kind: "temp", name, type };
}

function runSinglePass(module: MirModule, pass: typeof localCsePass | typeof addressCsePass): MirModule {
  const result = runMirPassPipeline(module, { optLevel: 2, passes: [pass], validateAfterEachPass: true }, {
    optLevel: 2,
    overflowMode: "unchecked",
    targetBackend: "c",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

function lower(sourceText: string): MirModule {
  const checked = check(new SourceFile("cse.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return mir;
}

function optimizeForC(sourceText: string, optLevel: 0 | 1 | 2 | 3): MirModule {
  const mir = lower(sourceText);
  const result = runMirPassPipeline(mir, buildMirOptimizationPipeline(optLevel), {
    optLevel,
    overflowMode: "unchecked",
    targetBackend: "c",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

describe("MIR CSE passes", () => {
  it("replaces repeated pure arithmetic with a move from the first value", () => {
    const func: MirFunction = {
      name: "calc",
      exported: true,
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
            { kind: "binary", target: temp("t0", i64), op: "+", left: param("a", i64), right: param("b", i64) },
            { kind: "binary", target: temp("t1", i64), op: "+", left: param("a", i64), right: param("b", i64) },
            { kind: "compare", target: temp("t2", boolType), op: ">", left: temp("t1", i64), right: param("a", i64) }
          ],
          terminator: { kind: "return", value: temp("t1", i64) }
        }
      ]
    };

    const optimized = runSinglePass({ structs: [], functions: [func] }, localCsePass);

    expect(optimized.functions[0]!.blocks[0]!.instructions[1]).toEqual({ kind: "move", target: temp("t1", i64), value: temp("t0", i64) });
    expect(optimized.functions[0]!.blocks[0]!.instructions[2]).toMatchObject({ kind: "compare", left: temp("t1", i64) });
  });

  it("applies same-order local CSE to f64 addition without reordering operands", () => {
    const func: MirFunction = {
      name: "calc_f64",
      exported: true,
      params: [
        { name: "a", type: f64 },
        { name: "b", type: f64 }
      ],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "binary", target: temp("t0", f64), op: "+", left: param("a", f64), right: param("b", f64) },
            { kind: "binary", target: temp("t1", f64), op: "+", left: param("b", f64), right: param("a", f64) },
            { kind: "binary", target: temp("t2", f64), op: "+", left: param("a", f64), right: param("b", f64) }
          ],
          terminator: { kind: "return", value: temp("t2", f64) }
        }
      ]
    };

    const optimized = runSinglePass({ structs: [], functions: [func] }, localCsePass);
    const instructions = optimized.functions[0]!.blocks[0]!.instructions;

    expect(instructions[1]).toEqual({ kind: "binary", target: temp("t1", f64), op: "+", left: param("b", f64), right: param("a", f64) });
    expect(instructions[2]).toEqual({ kind: "move", target: temp("t2", f64), value: temp("t0", f64) });
  });

  it("applies same-order local CSE to f64 multiplication without treating it as commutative", () => {
    const func: MirFunction = {
      name: "mul_f64",
      exported: true,
      params: [
        { name: "a", type: f64 },
        { name: "b", type: f64 }
      ],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "binary", target: temp("t0", f64), op: "*", left: param("a", f64), right: param("b", f64) },
            { kind: "binary", target: temp("t1", f64), op: "*", left: param("b", f64), right: param("a", f64) },
            { kind: "binary", target: temp("t2", f64), op: "*", left: param("a", f64), right: param("b", f64) }
          ],
          terminator: { kind: "return", value: temp("t2", f64) }
        }
      ]
    };

    const optimized = runSinglePass({ structs: [], functions: [func] }, localCsePass);
    const instructions = optimized.functions[0]!.blocks[0]!.instructions;

    expect(instructions[0]).toEqual({ kind: "binary", target: temp("t0", f64), op: "*", left: param("a", f64), right: param("b", f64) });
    expect(instructions[1]).toEqual({ kind: "binary", target: temp("t1", f64), op: "*", left: param("b", f64), right: param("a", f64) });
    expect(instructions[2]).toEqual({ kind: "move", target: temp("t2", f64), value: temp("t0", f64) });
  });

  it("applies same-order local CSE to f64 subtraction and unary negation", () => {
    const func: MirFunction = {
      name: "sub_neg_f64",
      exported: true,
      params: [
        { name: "a", type: f64 },
        { name: "b", type: f64 }
      ],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "binary", target: temp("t0", f64), op: "-", left: param("a", f64), right: param("b", f64) },
            { kind: "binary", target: temp("t1", f64), op: "-", left: param("a", f64), right: param("b", f64) },
            { kind: "unary", target: temp("t2", f64), op: "neg", operand: temp("t1", f64) },
            { kind: "unary", target: temp("t3", f64), op: "neg", operand: temp("t1", f64) }
          ],
          terminator: { kind: "return", value: temp("t3", f64) }
        }
      ]
    };

    const optimized = runSinglePass({ structs: [], functions: [func] }, localCsePass);
    const instructions = optimized.functions[0]!.blocks[0]!.instructions;

    expect(instructions[1]).toEqual({ kind: "move", target: temp("t1", f64), value: temp("t0", f64) });
    expect(instructions[3]).toEqual({ kind: "move", target: temp("t3", f64), value: temp("t2", f64) });
  });

  it("does not apply f64 local CSE to division or comparisons", () => {
    const func: MirFunction = {
      name: "div_compare_f64",
      exported: true,
      params: [
        { name: "a", type: f64 },
        { name: "b", type: f64 }
      ],
      returnType: boolType,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "binary", target: temp("t0", f64), op: "/", left: param("a", f64), right: param("b", f64) },
            { kind: "binary", target: temp("t1", f64), op: "/", left: param("a", f64), right: param("b", f64) },
            { kind: "compare", target: temp("t2", boolType), op: "==", left: temp("t0", f64), right: temp("t1", f64) },
            { kind: "compare", target: temp("t3", boolType), op: "==", left: temp("t0", f64), right: temp("t1", f64) }
          ],
          terminator: { kind: "return", value: temp("t3", boolType) }
        }
      ]
    };

    const optimized = runSinglePass({ structs: [], functions: [func] }, localCsePass);
    const instructions = optimized.functions[0]!.blocks[0]!.instructions;

    expect(instructions[1]).toEqual({ kind: "binary", target: temp("t1", f64), op: "/", left: param("a", f64), right: param("b", f64) });
    expect(instructions[3]).toEqual({ kind: "compare", target: temp("t3", boolType), op: "==", left: temp("t0", f64), right: temp("t1", f64) });
  });

  it("applies same-kind local CSE to explicit casts without merging different cast kinds", () => {
    const func: MirFunction = {
      name: "cast_cse",
      exported: true,
      params: [
        { name: "a", type: i32 },
        { name: "b", type: u32 }
      ],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "cast", target: temp("t0", f64), op: "i32_to_f64", value: param("a", i32) },
            { kind: "cast", target: temp("t1", f64), op: "u32_to_f64", value: param("b", u32) },
            { kind: "cast", target: temp("t2", f64), op: "i32_to_f64", value: param("a", i32) },
            { kind: "cast", target: temp("t3", f64), op: "u32_to_f64", value: param("b", u32) },
            { kind: "binary", target: temp("t4", f64), op: "+", left: temp("t2", f64), right: temp("t3", f64) }
          ],
          terminator: { kind: "return", value: temp("t4", f64) }
        }
      ]
    };

    const optimized = runSinglePass({ structs: [], functions: [func] }, localCsePass);
    const instructions = optimized.functions[0]!.blocks[0]!.instructions;

    expect(instructions[1]).toEqual({ kind: "cast", target: temp("t1", f64), op: "u32_to_f64", value: param("b", u32) });
    expect(instructions[2]).toEqual({ kind: "move", target: temp("t2", f64), value: temp("t0", f64) });
    expect(instructions[3]).toEqual({ kind: "move", target: temp("t3", f64), value: temp("t1", f64) });
  });

  it("enables same-order f64 local CSE only at O2 and O3", () => {
    const sourceText = `
      export fn repeated(a: f64, b: f64) -> f64 {
        let x: f64 = a + b;
        let y: f64 = a + b;
        return x + y;
      }
    `;

    const o0 = printMirModule(optimizeForC(sourceText, 0));
    const o1 = printMirModule(optimizeForC(sourceText, 1));
    const o2 = printMirModule(optimizeForC(sourceText, 2));
    const o3 = printMirModule(optimizeForC(sourceText, 3));

    expect(o0.match(/add a, b/g)).toHaveLength(2);
    expect(o1.match(/add a, b/g)).toHaveLength(2);
    expect(o2.match(/add a, b/g)).toHaveLength(1);
    expect(o3.match(/add a, b/g)).toHaveLength(1);
    expect(o2).toContain("add x, y");
    expect(o3).toContain("add x, y");
  });

  it("does not CSE ordinary loads across a store", () => {
    const optimized = optimizeForC(
      `
        struct Item {
          price: i64;
        }

        export fn calc(items: ptr<Item>, out: ptr<i64>) -> i64 {
          let first: i64 = items[0].price;
          out[0] = 1;
          let second: i64 = items[0].price;
          return first + second;
        }
      `,
      2
    );
    const text = printMirModule(optimized);

    expect((text.match(/load field/g) ?? [])).toHaveLength(2);
    expect(text).toMatch(/store deref\(%addr\d+\), %t/);
  });

  it("reuses ptr<Struct>[i] address calculations for repeated field access", () => {
    const optimized = runSinglePass(
      lower(`
        struct Item {
          price: i64;
          qty: i64;
        }

        export fn calc(items: ptr<Item>, i: i32) -> i64 {
          return items[i].price + items[i].qty;
        }
      `),
      addressCsePass
    );
    const text = printMirModule(optimized);

    expect(text).toContain("%addr0: ptr<Item> = address index(items, i)");
    expect(text).toContain("load field(deref(%addr0), price)");
    expect(text).toContain("load field(deref(%addr0), qty)");
    expect((text.match(/ = address index\(items, i\)/g) ?? [])).toHaveLength(1);
  });

  it("materializes scalar indexed stores as pointer locals for C hot paths", () => {
    const optimized = runSinglePass(
      lower(`
        export fn write_i64(out: ptr<i64>, i: i32, value: i64) -> i32 {
          out[i] = value;
          return 0;
        }
      `),
      addressCsePass
    );
    const text = printMirModule(optimized);

    expect(text).toContain("%addr0: ptr<i64> = address index(out, i)");
    expect(text).toContain("store deref(%addr0), value");
  });

  it("materializes ptr<f64> indexed places without changing value expressions", () => {
    const optimized = runSinglePass(
      lower(`
        export fn write_f64(out: ptr<f64>, i: i32, value: f64) -> f64 {
          out[i] = value;
          return out[i];
        }
      `),
      addressCsePass
    );
    const text = printMirModule(optimized);

    expect(text).toContain("%addr0: ptr<f64> = address index(out, i)");
    expect(text).toContain("store deref(%addr0), value");
    expect(text).toContain("%addr1: ptr<f64> = address index(out, i)");
    expect(text).toContain("load deref(%addr1)");
  });

  it("clears address CSE state after calls", () => {
    const optimized = runSinglePass(
      lower(`
        struct Item {
          price: i64;
          qty: i64;
        }

        fn opaque(x: i64) -> i64 {
          return x;
        }

        export fn calc(items: ptr<Item>, i: i32) -> i64 {
          let first: i64 = items[i].price;
          let ignored: i64 = opaque(first);
          return items[i].qty + ignored;
        }
      `),
      addressCsePass
    );
    const text = printMirModule(optimized);

    expect((text.match(/ = address index\(items, i\)/g) ?? [])).toHaveLength(2);
  });

  it("emits C that reuses the indexed struct pointer at O2", () => {
    const sourceText = `
          struct Item {
            price: i64;
            qty: i64;
          }

          export fn calc(items: ptr<Item>, i: i32) -> i64 {
            return items[i].price + items[i].qty;
          }
        `;
    const checked = check(new SourceFile("cse.ck", sourceText));
    expect(checked.diagnostics).toEqual([]);
    const mir = optimizeForC(sourceText, 2);
    const c = emitMirCSource(mir, { headerFileName: "cse.h" });

    expect(c).toContain("Item* ik_tmp_addr0;");
    expect(c).toContain("ik_tmp_addr0 = &items[i];");
    expect(c).toContain("ik_tmp0 = ik_tmp_addr0->price;");
    expect(c).toContain("ik_tmp1 = ik_tmp_addr0->qty;");
  });
});
