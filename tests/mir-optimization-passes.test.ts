import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { lowerToMir } from "../src/mir/lower.js";
import type { MirFunction, MirInstruction, MirModule, MirType, MirValue } from "../src/mir/mir.js";
import { printMirModule } from "../src/mir/mir-printer.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { runMirPassPipeline } from "../src/opt/mir-pass-manager.js";
import { buildMirOptimizationPipeline } from "../src/opt/pipeline.js";
import { constantFoldingPass } from "../src/opt/passes/constant-folding.js";
import { copyPropagationPass } from "../src/opt/passes/copy-propagation.js";
import { deadCodeEliminationPass } from "../src/opt/passes/dead-code-elimination.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

const i64: MirType = { kind: "primitive", name: "i64" };
const f64: MirType = { kind: "primitive", name: "f64" };
const boolType: MirType = { kind: "primitive", name: "bool" };
const ptrI64: MirType = { kind: "pointer", elementType: i64 };

function param(name: string, type: MirType): MirValue {
  return { kind: "param", name, type };
}

function temp(name: string, type: MirType): MirValue {
  return { kind: "temp", name, type };
}

function local(name: string, type: MirType): MirValue {
  return { kind: "local", name, type };
}

function normalizeNewlines(text: string): string {
  return text.replaceAll("\r\n", "\n");
}

function lower(sourceText: string): MirModule {
  const checked = check(new SourceFile("test.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return mir;
}

function functionByName(module: MirModule, name: string): MirFunction {
  const func = module.functions.find((candidate) => candidate.name === name);
  expect(func).toBeDefined();
  return func!;
}

function functionInstructions(module: MirModule, name: string): MirInstruction[] {
  return functionByName(module, name).blocks.flatMap((block) => block.instructions);
}

function hasF64Binary(module: MirModule, functionName: string, op: "+" | "-" | "*" | "/"): boolean {
  return functionInstructions(module, functionName).some(
    (instruction) =>
      instruction.kind === "binary" &&
      instruction.op === op &&
      instruction.target.type.kind === "primitive" &&
      instruction.target.type.name === "f64"
  );
}

function hasConstFloat(module: MirModule, functionName: string, value: string): boolean {
  return functionInstructions(module, functionName).some((instruction) => instruction.kind === "const_float" && instruction.value === value);
}

function f64SensitiveOptimizationSource(): string {
  return `
    export fn nan_mul_zero(x: f64) -> f64 {
      return x * 0.0;
    }

    export fn nan_div_self(x: f64) -> f64 {
      return x / x;
    }

    export fn signed_zero_add(x: f64) -> f64 {
      return x + 0.0;
    }

    export fn inf_div_zero() -> f64 {
      return 1.0 / 0.0;
    }
  `;
}

function optimize(module: MirModule, optLevel: 0 | 1 | 2 | 3, overflowMode: "unchecked" | "checked" = "unchecked"): MirModule {
  const result = runMirPassPipeline(module, buildMirOptimizationPipeline(optLevel), {
    optLevel,
    overflowMode,
    targetBackend: "c",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

function runSinglePass(module: MirModule, passName: "constant-folding" | "copy-propagation" | "dead-code-elimination"): MirModule {
  const pass =
    passName === "constant-folding" ? constantFoldingPass : passName === "copy-propagation" ? copyPropagationPass : deadCodeEliminationPass;
  const result = runMirPassPipeline(module, { optLevel: 1, passes: [pass], validateAfterEachPass: true }, {
    optLevel: 1,
    overflowMode: "unchecked",
    targetBackend: "c",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

describe("MIR optimization passes", () => {
  it("folds safe integer, comparison, and unary constants", () => {
    const mir = optimize(
      lower(`
        export fn calc_i32() -> i32 {
          return 1 + 2 * 3;
        }

        export fn less() -> bool {
          return 1 < 2;
        }

        export fn neg() -> i32 {
          return -1;
        }

        export fn not_false() -> bool {
          return !false;
        }
      `),
      1
    );
    const text = printMirModule(mir);

    expect(text).toContain("%t4: i32 = const_int 7");
    expect(text).toContain("%t2: bool = const_bool true");
    expect(text).toContain("%t1: i32 = const_int -1");
    expect(text).toContain("%t1: bool = const_bool true");
    expect(text).not.toContain(" = mul ");
    expect(text).not.toContain(" = add ");
    expect(text).not.toContain(" = lt ");
    expect(text).not.toContain(" = neg ");
    expect(text).not.toContain(" = not ");
  });

  it("does not fold overflow or division by zero", () => {
    const mir = optimize(
      lower(`
        export fn add_overflow() -> i32 {
          return 2147483647 + 1;
        }

        export fn div_zero() -> i32 {
          return 1 / 0;
        }

        export fn div_overflow() -> i32 {
          return (-2147483647 - 1) / -1;
        }
      `),
      1
    );
    const text = printMirModule(mir);

    expect(text).toContain("add %t0, %t1");
    expect(text).toContain("div %t0, %t1");
  });

  it("does not fold arithmetic in checked mode", () => {
    const mir = optimize(
      lower(`
        export fn calc() -> i64 {
          return 1 + 2;
        }
      `),
      1,
      "checked"
    );

    expect(printMirModule(mir)).toContain("%t2: i64 = add %t0, %t1");
  });

  it("does not constant fold f64 at O1", () => {
    const mir = optimize(
      lower(`
        export fn calc() -> f64 {
          return 1.0 + 2.0;
        }
      `),
      1
    );
    const text = printMirModule(mir);

    expect(text).toContain("const_float 1.0");
    expect(text).toContain("const_float 2.0");
    expect(text).toContain("add %t0, %t1");
  });

  it("keeps f64 emit-mir valid at O0, O1, O2, and O3", () => {
    const source = `
      export fn calc(a: f64, b: f64) -> f64 {
        let x: f64 = 1.0 + 2.0;
        return a + b + x;
      }
    `;
    const lowered = printMirModule(lower(source));
    const o0 = printMirModule(optimize(lower(source), 0));

    expect(o0).toBe(lowered);
    for (const level of [1, 2, 3] as const) {
      const text = printMirModule(optimize(lower(source), level));
      expect(text).toContain("const_float 1.0");
      expect(text).toContain("const_float 2.0");
      expect(text).toContain("add");
    }
  });

  it("keeps NaN, signed-zero, and Infinity-sensitive f64 algebra unchanged across O0, O1, O2, and O3", () => {
    const source = f64SensitiveOptimizationSource();
    const lowered = printMirModule(lower(source));

    expect(printMirModule(optimize(lower(source), 0))).toBe(lowered);

    for (const level of [1, 2, 3] as const) {
      const optimized = optimize(lower(source), level);

      expect(hasF64Binary(optimized, "nan_mul_zero", "*")).toBe(true);
      expect(hasConstFloat(optimized, "nan_mul_zero", "0.0")).toBe(true);
      expect(hasF64Binary(optimized, "nan_div_self", "/")).toBe(true);
      expect(hasF64Binary(optimized, "signed_zero_add", "+")).toBe(true);
      expect(hasConstFloat(optimized, "signed_zero_add", "0.0")).toBe(true);
      expect(hasF64Binary(optimized, "inf_div_zero", "/")).toBe(true);
      expect(hasConstFloat(optimized, "inf_div_zero", "1.0")).toBe(true);
      expect(hasConstFloat(optimized, "inf_div_zero", "0.0")).toBe(true);
    }
  });

  it("keeps f64 division by zero as ordinary f64 MIR in checked optimization mode", () => {
    const optimized = optimize(
      lower(`
        export fn div_zero_f64() -> f64 {
          return 1.0 / 0.0;
        }
      `),
      3,
      "checked"
    );

    expect(hasF64Binary(optimized, "div_zero_f64", "/")).toBe(true);
    expect(hasConstFloat(optimized, "div_zero_f64", "1.0")).toBe(true);
    expect(hasConstFloat(optimized, "div_zero_f64", "0.0")).toBe(true);
  });

  it("keeps checked integer arithmetic unfurled at O3", () => {
    const optimized = optimize(
      lower(`
        export fn calc() -> i64 {
          return 1 + 2;
        }
      `),
      3,
      "checked"
    );

    expect(printMirModule(optimized)).toContain("%t2: i64 = add %t0, %t1");
  });

  it("propagates simple temp copies without crossing calls or stores", () => {
    const module: MirModule = {
      structs: [],
      functions: [
        {
          name: "copy",
          exported: true,
          params: [{ name: "a", type: i64 }],
          returnType: i64,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                { kind: "move", target: temp("t0", i64), value: param("a", i64) },
                { kind: "move", target: temp("t1", i64), value: temp("t0", i64) },
                { kind: "binary", target: temp("t2", i64), op: "+", left: temp("t1", i64), right: param("a", i64) },
                { kind: "call", target: temp("t3", i64), functionName: "opaque", args: [temp("t2", i64)] },
                { kind: "move", target: temp("t4", i64), value: temp("t3", i64) },
                { kind: "store", place: { kind: "local", name: "sink", type: i64 }, value: temp("t4", i64) },
                { kind: "move", target: temp("t5", i64), value: temp("t4", i64) }
              ],
              terminator: { kind: "return", value: temp("t5", i64) }
            }
          ]
        },
        {
          name: "opaque",
          exported: false,
          params: [{ name: "x", type: i64 }],
          returnType: i64,
          locals: [],
          blocks: [{ label: "bb0", instructions: [], terminator: { kind: "return", value: param("x", i64) } }]
        }
      ]
    };
    module.functions[0]!.locals.push({ name: "sink", type: i64 });

    const optimized = runSinglePass(module, "copy-propagation");
    const instructions = optimized.functions[0]!.blocks[0]!.instructions;

    expect(instructions[2]).toMatchObject({ kind: "binary", left: param("a", i64) });
    expect(instructions[5]).toMatchObject({ kind: "store", value: temp("t3", i64) });
    expect(optimized.functions[0]!.blocks[0]!.terminator).toEqual({ kind: "return", value: temp("t4", i64) });
  });

  it("propagates f64 temp copies as a type-agnostic rewrite", () => {
    const module: MirModule = {
      structs: [],
      functions: [
        {
          name: "copy_f64",
          exported: true,
          params: [{ name: "a", type: f64 }],
          returnType: f64,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                { kind: "move", target: temp("t0", f64), value: param("a", f64) },
                { kind: "binary", target: temp("t1", f64), op: "+", left: temp("t0", f64), right: param("a", f64) }
              ],
              terminator: { kind: "return", value: temp("t1", f64) }
            }
          ]
        }
      ]
    };

    const optimized = runSinglePass(module, "copy-propagation");
    expect(optimized.functions[0]!.blocks[0]!.instructions[1]).toMatchObject({ kind: "binary", left: param("a", f64) });
  });

  it("deletes unused pure temp instructions", () => {
    const module: MirModule = {
      structs: [],
      functions: [
        {
          name: "dce",
          exported: true,
          params: [{ name: "a", type: i64 }],
          returnType: i64,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                { kind: "const_int", target: temp("unused_const", i64), value: "1" },
                { kind: "binary", target: temp("unused_binary", i64), op: "+", left: param("a", i64), right: param("a", i64) },
                { kind: "unary", target: temp("unused_unary", i64), op: "neg", operand: param("a", i64) },
                { kind: "compare", target: temp("unused_compare", boolType), op: "==", left: param("a", i64), right: param("a", i64) },
                { kind: "move", target: temp("used", i64), value: param("a", i64) }
              ],
              terminator: { kind: "return", value: temp("used", i64) }
            }
          ]
        }
      ]
    };

    const optimized = runSinglePass(module, "dead-code-elimination");
    expect(optimized.functions[0]!.blocks[0]!.instructions).toEqual([{ kind: "move", target: temp("used", i64), value: param("a", i64) }]);
  });

  it("deletes unused f64 pure instructions without touching control flow", () => {
    const module: MirModule = {
      structs: [],
      functions: [
        {
          name: "dce_f64",
          exported: true,
          params: [{ name: "a", type: f64 }],
          returnType: f64,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                { kind: "const_float", target: temp("unused_const", f64), value: "1.0" },
                { kind: "binary", target: temp("unused_binary", f64), op: "+", left: param("a", f64), right: param("a", f64) },
                { kind: "move", target: temp("used", f64), value: param("a", f64) }
              ],
              terminator: { kind: "return", value: temp("used", f64) }
            }
          ]
        }
      ]
    };

    const optimized = runSinglePass(module, "dead-code-elimination");
    expect(optimized.functions[0]!.blocks[0]!.instructions).toEqual([{ kind: "move", target: temp("used", f64), value: param("a", f64) }]);
    expect(optimized.functions[0]!.blocks[0]!.terminator).toEqual({ kind: "return", value: temp("used", f64) });
  });

  it("keeps stores, calls, and terminator values", () => {
    const caller: MirFunction = {
      name: "caller",
      exported: true,
      params: [
        { name: "out", type: ptrI64 },
        { name: "a", type: i64 }
      ],
      returnType: i64,
      locals: [{ name: "sink", type: i64 }],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "store", place: { kind: "local", name: "sink", type: i64 }, value: param("a", i64) },
            { kind: "call", target: temp("unused_call", i64), functionName: "callee", args: [param("a", i64)] }
          ],
          terminator: { kind: "return", value: param("a", i64) }
        }
      ]
    };
    const callee: MirFunction = {
      name: "callee",
      exported: false,
      params: [{ name: "x", type: i64 }],
      returnType: i64,
      locals: [],
      blocks: [{ label: "bb0", instructions: [], terminator: { kind: "return", value: param("x", i64) } }]
    };

    const optimized = runSinglePass({ structs: [], functions: [caller, callee] }, "dead-code-elimination");
    expect(optimized.functions[0]!.blocks[0]!.instructions).toHaveLength(2);
    expect(optimized.functions[0]!.blocks[0]!.instructions[0]!.kind).toBe("store");
    expect(optimized.functions[0]!.blocks[0]!.instructions[1]!.kind).toBe("call");
  });

  it("leaves MIR unchanged at O0 and changes it at O1", () => {
    const source = `
      export fn calc() -> i32 {
        return 1 + 2 * 3;
      }
    `;
    const o0 = optimize(lower(source), 0);
    const o1 = optimize(lower(source), 1);

    expect(printMirModule(o0)).toContain("mul %t1, %t2");
    expect(printMirModule(o0)).toContain("add %t0, %t3");
    expect(printMirModule(o1)).not.toBe(printMirModule(o0));
    expect(printMirModule(o1)).toContain("const_int 7");
  });

  it("matches optimized MIR snapshot for pricing", () => {
    const checked = check(new SourceFile("pricing.ik", readFileSync("examples/pricing.ik", "utf8")));
    expect(checked.diagnostics).toEqual([]);
    const mir = lowerToMir(checked.checkedProgram);
    const optimized = optimize(mir, 1);

    expect(printMirModule(optimized)).toBe(normalizeNewlines(readFileSync("tests/snapshots/pricing.optimized.mir.snap", "utf8")));
  });
});
