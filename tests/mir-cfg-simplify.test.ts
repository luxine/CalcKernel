import { describe, expect, it } from "vitest";
import { lowerToMir } from "../src/mir/lower.js";
import type { MirBlock, MirModule, MirTerminator, MirType, MirValue } from "../src/mir/mir.js";
import { printMirModule } from "../src/mir/mir-printer.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { runMirPassPipeline } from "../src/opt/mir-pass-manager.js";
import { buildMirOptimizationPipeline } from "../src/opt/pipeline.js";
import { cfgSimplifyPass } from "../src/opt/passes/cfg-simplify.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

const i64: MirType = { kind: "primitive", name: "i64" };
const boolType: MirType = { kind: "primitive", name: "bool" };

function temp(name: string, type: MirType): MirValue {
  return { kind: "temp", name, type };
}

function param(name: string, type: MirType): MirValue {
  return { kind: "param", name, type };
}

function block(label: string, terminator: MirTerminator, instructions: MirBlock["instructions"] = []): MirBlock {
  return { label, instructions, terminator };
}

function moduleWithBlocks(blocks: MirBlock[]): MirModule {
  return {
    structs: [],
    functions: [
      {
        name: "cfg",
        exported: true,
        params: [{ name: "a", type: i64 }],
        returnType: i64,
        locals: [],
        blocks
      }
    ]
  };
}

function runCfg(module: MirModule, optLevel: 1 | 2 | 3): MirModule {
  const result = runMirPassPipeline(module, { optLevel, passes: [cfgSimplifyPass], validateAfterEachPass: true }, {
    optLevel,
    overflowMode: "unchecked",
    targetBackend: "mir",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

function optimizeSource(sourceText: string, optLevel: 0 | 1 | 2 | 3): MirModule {
  const checked = check(new SourceFile("cfg.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  const result = runMirPassPipeline(mir, buildMirOptimizationPipeline(optLevel), {
    optLevel,
    overflowMode: "unchecked",
    targetBackend: "mir",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

describe("CFG simplification pass", () => {
  it("removes unreachable blocks at O1", () => {
    const optimized = runCfg(
      moduleWithBlocks([
        block("bb0", { kind: "return", value: param("a", i64) }),
        block("bb_dead", { kind: "return", value: param("a", i64) })
      ]),
      1
    );

    expect(optimized.functions[0]!.blocks.map((item) => item.label)).toEqual(["bb0"]);
  });

  it("simplifies constant branches at O2", () => {
    const optimized = runCfg(
      moduleWithBlocks([
        block("bb0", { kind: "branch", condition: { kind: "const_bool", value: true, type: boolType }, thenLabel: "bb_then", elseLabel: "bb_else" }),
        block("bb_then", { kind: "return", value: param("a", i64) }),
        block("bb_else", { kind: "return", value: { kind: "const_int", text: "0", type: i64 } })
      ]),
      2
    );

    expect(optimized.functions[0]!.blocks[0]!.terminator).toEqual({ kind: "jump", label: "bb_then" });
    expect(optimized.functions[0]!.blocks.map((item) => item.label)).toEqual(["bb0", "bb_then"]);
  });

  it("rewrites jump chains through empty blocks at O2", () => {
    const optimized = runCfg(
      moduleWithBlocks([
        block("bb0", { kind: "jump", label: "bb1" }),
        block("bb1", { kind: "jump", label: "bb2" }),
        block("bb2", { kind: "return", value: param("a", i64) })
      ]),
      2
    );

    expect(optimized.functions[0]!.blocks[0]!.terminator).toEqual({ kind: "jump", label: "bb2" });
    expect(optimized.functions[0]!.blocks.map((item) => item.label)).toEqual(["bb0", "bb2"]);
  });

  it("keeps while CFG valid after O2 optimization", () => {
    const optimized = optimizeSource(
      `
        export fn sum_to_n(n: i64) -> i64 {
          let i: i64 = 0;
          let sum: i64 = 0;

          while i < n {
            sum = sum + i;
            i = i + 1;
          }

          return sum;
        }
      `,
      2
    );
    const text = printMirModule(optimized);

    expect(text).toContain("branch %t2, bb2, bb3");
    expect(text).toContain("jump bb1");
    expect(validateMirModule(optimized).errors).toEqual([]);
  });

  it("preserves short-circuit RHS blocks at O2", () => {
    const optimized = optimizeSource(
      `
        export fn and_short_circuit(a: i64, b: i64) -> bool {
          return a != 0 && b / a > 1;
        }

        export fn or_short_circuit(a: i64, b: i64) -> bool {
          return a == 0 || b / a > 1;
        }
      `,
      2
    );
    const text = printMirModule(optimized);

    expect(text).toContain("branch %t1, bb1, bb2");
    expect(text).toContain("%t2: i64 = div b, a");
    expect(text).toContain("branch %t1, bb1, bb2");
    expect(text).toContain("%t2: i64 = div b, a");
    expect(validateMirModule(optimized).errors).toEqual([]);
  });
});
