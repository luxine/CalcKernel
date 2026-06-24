import { describe, expect, it } from "vitest";
import { LlvmIrWriter } from "../src/backend/llvm/llvm-ir-writer.js";

describe("LLVM IR writer", () => {
  it("emits stable lines with two-space indentation", () => {
    const writer = new LlvmIrWriter();

    writer.line("define i64 @add_i64(i64 %a, i64 %b) {");
    writer.indent(() => {
      writer.line("entry:");
      writer.indent(() => {
        writer.line("%ik_tmp0 = add i64 %a, %b");
        writer.line("ret i64 %ik_tmp0");
      });
    });
    writer.line("}");

    expect(writer.toString()).toBe(`define i64 @add_i64(i64 %a, i64 %b) {
  entry:
    %ik_tmp0 = add i64 %a, %b
    ret i64 %ik_tmp0
}
`);
  });

  it("supports blank lines and block helpers", () => {
    const writer = new LlvmIrWriter();

    writer.line("; module");
    writer.blankLine();
    writer.block("define i32 @main() {", "}", () => {
      writer.line("entry:");
      writer.indent(() => {
        writer.line("ret i32 0");
      });
    });

    expect(writer.toString()).toBe(`; module

define i32 @main() {
  entry:
    ret i32 0
}
`);
  });
});
