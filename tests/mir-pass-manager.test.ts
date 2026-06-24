import { describe, expect, it } from "vitest";
import type { MirModule, MirType } from "../src/mir/mir.js";
import { buildMirOptimizationPipeline, printMirPassPipeline } from "../src/opt/pipeline.js";
import { identityPass } from "../src/opt/passes/identity-pass.js";
import { runMirPassPipeline } from "../src/opt/mir-pass-manager.js";
import type { MirPass } from "../src/opt/mir-pass.js";

const i64: MirType = { kind: "primitive", name: "i64" };

function validModule(): MirModule {
  return {
    structs: [],
    functions: [
      {
        name: "add_i64",
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
              {
                kind: "binary",
                target: { kind: "temp", name: "t0", type: i64 },
                op: "+",
                left: { kind: "param", name: "a", type: i64 },
                right: { kind: "param", name: "b", type: i64 }
              }
            ],
            terminator: { kind: "return", value: { kind: "temp", name: "t0", type: i64 } }
          }
        ]
      }
    ]
  };
}

describe("MIR pass manager", () => {
  it("builds the initial opt-level pipelines", () => {
    expect(buildMirOptimizationPipeline(0).passes.map((pass) => pass.name)).toEqual([]);
    expect(buildMirOptimizationPipeline(1).passes.map((pass) => pass.name)).toEqual([
      "constant-folding",
      "copy-propagation",
      "dead-code-elimination",
      "cfg-simplify"
    ]);
    expect(buildMirOptimizationPipeline(2).passes.map((pass) => pass.name)).toEqual([
      "constant-folding",
      "copy-propagation",
      "inline-small-functions",
      "constant-folding",
      "copy-propagation",
      "local-cse",
      "copy-propagation",
      "address-cse",
      "dead-code-elimination",
      "cfg-simplify",
      "dead-code-elimination"
    ]);
    expect(buildMirOptimizationPipeline(3).passes.map((pass) => pass.name)).toEqual([
      "constant-folding",
      "copy-propagation",
      "inline-small-functions",
      "constant-folding",
      "copy-propagation",
      "loop-analysis",
      "loop-invariant-code-motion",
      "induction-simplify",
      "constant-folding",
      "copy-propagation",
      "local-cse",
      "copy-propagation",
      "address-cse",
      "dead-code-elimination",
      "cfg-simplify",
      "dead-code-elimination"
    ]);
  });

  it("prints pass pipelines in a stable order", () => {
    expect(printMirPassPipeline(buildMirOptimizationPipeline(0))).toBe("O0: <validator only>");
    expect(printMirPassPipeline(buildMirOptimizationPipeline(3))).toBe(
      "O3: constant-folding -> copy-propagation -> inline-small-functions -> constant-folding -> copy-propagation -> loop-analysis -> loop-invariant-code-motion -> induction-simplify -> constant-folding -> copy-propagation -> local-cse -> copy-propagation -> address-cse -> dead-code-elimination -> cfg-simplify -> dead-code-elimination"
    );
  });

  it("runs no optimization pass at O0 but still validates MIR", () => {
    const module = validModule();
    const result = runMirPassPipeline(module, buildMirOptimizationPipeline(0), {
      optLevel: 0,
      overflowMode: "unchecked",
      targetBackend: "c",
      debug: {}
    });

    expect(result.changed).toBe(false);
    expect(result.records).toEqual([]);
    expect(result.validationErrors).toEqual([]);
  });

  it("runs identity pass without changing MIR", () => {
    const module = validModule();
    const before = JSON.stringify(module);
    const result = runMirPassPipeline(module, { optLevel: 1, passes: [identityPass], validateAfterEachPass: true }, {
      optLevel: 1,
      overflowMode: "unchecked",
      targetBackend: "wasm",
      debug: {}
    });

    expect(result.changed).toBe(false);
    expect(result.records).toEqual([{ name: "identity", changed: false }]);
    expect(JSON.stringify(result.module)).toBe(before);
    expect(result.validationErrors).toEqual([]);
  });

  it("keeps pass order stable and aggregates changed state", () => {
    const calls: string[] = [];
    const first: MirPass = {
      name: "first",
      run() {
        calls.push("first");
        return { changed: false };
      }
    };
    const second: MirPass = {
      name: "second",
      run() {
        calls.push("second");
        return { changed: true };
      }
    };

    const result = runMirPassPipeline(validModule(), { optLevel: 2, passes: [first, second], validateAfterEachPass: true }, {
      optLevel: 2,
      overflowMode: "unchecked",
      targetBackend: "llvm",
      debug: {}
    });

    expect(calls).toEqual(["first", "second"]);
    expect(result.changed).toBe(true);
    expect(result.records).toEqual([
      { name: "first", changed: false },
      { name: "second", changed: true }
    ]);
  });

  it("can validate MIR after every pass", () => {
    const breakReturn: MirPass = {
      name: "break-return",
      run(module) {
        module.functions[0]!.blocks[0]!.terminator = { kind: "return", value: { kind: "param", name: "missing", type: i64 } };
        return { changed: true };
      }
    };

    const result = runMirPassPipeline(validModule(), { optLevel: 3, passes: [breakReturn], validateAfterEachPass: true }, {
      optLevel: 3,
      overflowMode: "unchecked",
      targetBackend: "c",
      debug: {}
    });

    expect(result.changed).toBe(true);
    expect(result.records).toEqual([{ name: "break-return", changed: true }]);
    expect(result.validationErrors.map((error) => error.message)).toContain("Unknown param 'missing' in function 'add_i64'.");
  });
});
