import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { emitMirLlvmModule } from "../src/backend/llvm/mir-llvm-emitter.js";
import type { MirModule } from "../src/mir/mir.js";

function normalizeNewlines(text: string): string {
  return text.replaceAll("\r\n", "\n");
}

const skeletonModule: MirModule = {
  structs: [
    {
      name: "Item",
      fields: [
        { name: "price", type: { kind: "primitive", name: "i64" } },
        { name: "qty", type: { kind: "primitive", name: "i64" } },
        { name: "discount", type: { kind: "primitive", name: "i64" } },
        { name: "tax_rate_ppm", type: { kind: "primitive", name: "i64" } }
      ]
    }
  ],
  functions: [
    {
      name: "add_i64",
      exported: true,
      params: [
        { name: "a", type: { kind: "primitive", name: "i64" } },
        { name: "b", type: { kind: "primitive", name: "i64" } }
      ],
      returnType: { kind: "primitive", name: "i64" },
      locals: [],
      blocks: []
    },
    {
      name: "helper",
      exported: false,
      params: [{ name: "a", type: { kind: "primitive", name: "i64" } }],
      returnType: { kind: "primitive", name: "i64" },
      locals: [],
      blocks: []
    },
    {
      name: "is_positive",
      exported: true,
      params: [{ name: "value", type: { kind: "primitive", name: "i64" } }],
      returnType: { kind: "primitive", name: "bool" },
      locals: [],
      blocks: []
    }
  ]
};

describe("LLVM module skeleton emitter", () => {
  it("emits stable module headers, struct declarations, and function skeletons", () => {
    const llvm = emitMirLlvmModule(skeletonModule, {
      sourceFileName: "/tmp/work/input.ck",
      targetTriple: "x86_64-unknown-linux-gnu"
    });

    expect(llvm).toBe(normalizeNewlines(readFileSync("tests/snapshots/llvm_skeleton.ll.snap", "utf8")));
  });

  it("omits target triple when none is provided", () => {
    const llvm = emitMirLlvmModule({ structs: [], functions: [] }, { sourceFileName: "custom/path/pricing.ck" });

    expect(llvm).toBe(`; ModuleID = 'calckernel'
source_filename = "pricing.ck"
`);
    expect(llvm).not.toContain("target triple");
  });
});
