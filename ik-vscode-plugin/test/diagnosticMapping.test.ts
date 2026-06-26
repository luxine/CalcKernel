import { describe, expect, it } from "vitest";
import { spanToRangeCoordinates, type SourceSpanLike } from "../src/diagnosticMapping";

function span(startLine: number, startColumn: number, endLine: number, endColumn: number): SourceSpanLike {
  return {
    start: { line: startLine, column: startColumn, offset: 0 },
    end: { line: endLine, column: endColumn, offset: 0 }
  };
}

describe("spanToRangeCoordinates", () => {
  it("maps a same-line one-based CalcKernel span to a zero-based VSCode range", () => {
    expect(spanToRangeCoordinates("let x: i64 = 0;", span(1, 5, 1, 6))).toEqual({
      start: { line: 0, character: 4 },
      end: { line: 0, character: 5 }
    });
  });

  it("expands an empty same-line range by one character when possible", () => {
    expect(spanToRangeCoordinates("abc", span(1, 2, 1, 2))).toEqual({
      start: { line: 0, character: 1 },
      end: { line: 0, character: 2 }
    });
  });

  it("preserves multiline ranges after converting to zero-based coordinates", () => {
    expect(spanToRangeCoordinates("first\nsecond", span(1, 3, 2, 4))).toEqual({
      start: { line: 0, character: 2 },
      end: { line: 1, character: 3 }
    });
  });

  it("clamps columns that extend past the end of a line", () => {
    expect(spanToRangeCoordinates("abc", span(1, 10, 1, 12))).toEqual({
      start: { line: 0, character: 2 },
      end: { line: 0, character: 3 }
    });
  });

  it("handles diagnostics positioned at an empty EOF line", () => {
    expect(spanToRangeCoordinates("abc\n", span(2, 1, 2, 1))).toEqual({
      start: { line: 1, character: 0 },
      end: { line: 1, character: 0 }
    });
  });
});
