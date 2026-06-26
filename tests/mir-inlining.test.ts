import { describe, expect, it } from "vitest";
import { lowerToMir } from "../src/mir/lower.js";
import type { MirBlock, MirFunction, MirModule, MirType, MirValue } from "../src/mir/mir.js";
import { printMirModule } from "../src/mir/mir-printer.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { runMirPassPipeline } from "../src/opt/mir-pass-manager.js";
import { buildMirOptimizationPipeline } from "../src/opt/pipeline.js";
import { inlineSmallFunctionsPass } from "../src/opt/passes/inline-small-functions.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

const i64: MirType = { kind: "primitive", name: "i64" };

function temp(name: string, type: MirType): MirValue {
  return { kind: "temp", name, type };
}

function param(name: string, type: MirType): MirValue {
  return { kind: "param", name, type };
}

function lower(sourceText: string): MirModule {
  const checked = check(new SourceFile("inline.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return mir;
}

function optimize(sourceText: string, optLevel: 0 | 1 | 2 | 3): MirModule {
  const result = runMirPassPipeline(lower(sourceText), buildMirOptimizationPipeline(optLevel), {
    optLevel,
    overflowMode: "unchecked",
    targetBackend: "c",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

function runInlinePass(module: MirModule, optLevel: 2 | 3): MirModule {
  const result = runMirPassPipeline(module, { optLevel, passes: [inlineSmallFunctionsPass], validateAfterEachPass: true }, {
    optLevel,
    overflowMode: "unchecked",
    targetBackend: "c",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

function instructionBlock(instructions: MirBlock["instructions"]): MirBlock {
  return { label: "bb0", instructions, terminator: { kind: "return", value: temp(`t${instructions.length - 1}`, i64) } };
}

function thresholdModule(instructionCount: number): MirModule {
  const helperInstructions: MirBlock["instructions"] = [];
  for (let index = 0; index < instructionCount; index += 1) {
    helperInstructions.push({
      kind: "binary",
      target: temp(`t${index}`, i64),
      op: "+",
      left: index === 0 ? param("x", i64) : temp(`t${index - 1}`, i64),
      right: { kind: "const_int", text: "1", type: i64 }
    });
  }

  const helper: MirFunction = {
    name: "helper",
    exported: false,
    params: [{ name: "x", type: i64 }],
    returnType: i64,
    locals: [],
    blocks: [instructionBlock(helperInstructions)]
  };
  const caller: MirFunction = {
    name: "calc",
    exported: true,
    params: [{ name: "a", type: i64 }],
    returnType: i64,
    locals: [],
    blocks: [
      {
        label: "bb0",
        instructions: [{ kind: "call", target: temp("t0", i64), functionName: "helper", args: [param("a", i64)] }],
        terminator: { kind: "return", value: temp("t0", i64) }
      }
    ]
  };

  return { structs: [], functions: [helper, caller] };
}

describe("small function inlining", () => {
  it("inlines a small non-exported helper at O2", () => {
    const optimized = optimize(
      `
        fn add_one(x: i64) -> i64 {
          return x + 1;
        }

        export fn calc(a: i64) -> i64 {
          return add_one(a) * 2;
        }
      `,
      2
    );
    const text = printMirModule(optimized);

    expect(text).not.toContain("call add_one");
    expect(text).not.toContain("fn add_one");
    expect(text).toContain("export fn calc");
    expect(text).toContain("add a,");
    expect(text).toContain("mul");
  });

  it("inlines f64 helper values without adding float algebra assumptions", () => {
    const optimized = optimize(
      `
        fn add_one(x: f64) -> f64 {
          return x + 1.0;
        }

        export fn calc(a: f64) -> f64 {
          return add_one(a);
        }
      `,
      2
    );
    const text = printMirModule(optimized);

    expect(text).not.toContain("call add_one");
    expect(text).not.toContain("fn add_one");
    expect(text).toContain("const_float 1.0");
    expect(text).toContain("add a,");
  });

  it("does not inline exported functions", () => {
    const optimized = optimize(
      `
        export fn add_one(x: i64) -> i64 {
          return x + 1;
        }

        export fn calc(a: i64) -> i64 {
          return add_one(a);
        }
      `,
      2
    );

    expect(printMirModule(optimized)).toContain("call add_one(a)");
    expect(printMirModule(optimized)).toContain("export fn add_one");
  });

  it("does not inline recursive helpers", () => {
    const optimized = optimize(
      `
        fn recurse(x: i64) -> i64 {
          return recurse(x);
        }

        export fn calc(a: i64) -> i64 {
          return recurse(a);
        }
      `,
      2
    );

    expect(printMirModule(optimized)).toContain("call recurse(a)");
    expect(printMirModule(optimized)).toContain("fn recurse");
  });

  it("uses a smaller O2 threshold and a larger O3 threshold", () => {
    const o2 = runInlinePass(thresholdModule(9), 2);
    const o3 = runInlinePass(thresholdModule(9), 3);

    expect(printMirModule(o2)).toContain("call helper(a)");
    expect(printMirModule(o3)).not.toContain("call helper");
    expect(printMirModule(o3)).not.toContain("fn helper");
  });
});
