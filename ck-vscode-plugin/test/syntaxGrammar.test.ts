import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

type Capture = {
  name?: string;
};

type GrammarPattern = {
  name?: string;
  match?: string;
  captures?: Record<string, Capture>;
};

type Grammar = {
  repository: Record<string, { patterns: GrammarPattern[] }>;
};

const grammarPath = join(dirname(fileURLToPath(import.meta.url)), "..", "syntaxes", "calckernel.tmLanguage.json");
const grammar = JSON.parse(readFileSync(grammarPath, "utf8")) as Grammar;

describe("CalcKernel TextMate grammar identifier scopes", () => {
  it("scopes local variable declarations after let", () => {
    expect(expectCaptureScope("localVariableDeclarations", "let subtotal: i64 = 0;", "subtotal")).toBe(
      "variable.other.definition.local.calckernel"
    );
  });

  it("scopes function parameters before their type annotations", () => {
    expect(expectCaptureScope("parameters", "items: ptr<Item>", "items")).toBe("variable.parameter.calckernel");
    expect(expectCaptureScope("parameters", "ratio: f64", "ratio")).toBe("variable.parameter.calckernel");
  });

  it("scopes struct field declarations at the start of field lines", () => {
    expect(expectCaptureScope("fieldDeclarations", "  tax_rate_ppm: i64;", "tax_rate_ppm")).toBe(
      "variable.other.member.definition.calckernel"
    );
  });

  it("scopes member access fields after a dot", () => {
    expect(expectCaptureScope("memberAccess", ".price", "price")).toBe("variable.other.member.access.calckernel");
  });

  it("scopes compiler builtin calls", () => {
    expect(expectPatternScope("builtinFunctions", "u32_to_f64")).toBe("support.function.builtin.calckernel");
    expect(expectPatternScope("builtinFunctions", "i32_to_f64")).toBe("support.function.builtin.calckernel");
  });

  it("scopes lowercase variable references without overriding keywords or primitive types", () => {
    expect(expectPatternScope("variableReferences", "after_discount")).toBe("variable.other.readwrite.calckernel");
    expect(findNamedPatternMatch("variableReferences", "while")).toBeUndefined();
    expect(findNamedPatternMatch("variableReferences", "i64")).toBeUndefined();
    expect(findNamedPatternMatch("variableReferences", "f64")).toBeUndefined();
  });

  it("scopes f64 and float literals", () => {
    expect(expectPatternScope("types", "f64")).toBe("storage.type.primitive.calckernel");
    expect(expectPatternScope("numbers", "1.25")).toBe("constant.numeric.float.calckernel");
    expect(expectPatternScope("numbers", "1e-3")).toBe("constant.numeric.float.calckernel");
    expect(expectPatternScope("numbers", "42")).toBe("constant.numeric.integer.calckernel");
  });
});

function expectCaptureScope(repositoryName: string, source: string, capturedText: string): string | undefined {
  for (const pattern of patternsFor(repositoryName)) {
    if (!pattern.match || !pattern.captures) {
      continue;
    }

    const match = new RegExp(pattern.match, "m").exec(source);
    if (!match) {
      continue;
    }

    for (const [captureIndex, capture] of Object.entries(pattern.captures)) {
      if (match[Number(captureIndex)] === capturedText) {
        return capture.name;
      }
    }
  }

  return undefined;
}

function expectPatternScope(repositoryName: string, source: string): string | undefined {
  return findNamedPatternMatch(repositoryName, source)?.name;
}

function findNamedPatternMatch(repositoryName: string, source: string): GrammarPattern | undefined {
  return patternsFor(repositoryName).find((pattern) => {
    if (!pattern.match || !pattern.name) {
      return false;
    }

    const match = new RegExp(pattern.match, "m").exec(source);
    return match?.[0] === source;
  });
}

function patternsFor(repositoryName: string): GrammarPattern[] {
  return grammar.repository[repositoryName]?.patterns ?? [];
}
