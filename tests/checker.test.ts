import { describe, expect, it } from "vitest";
import {
  check,
  getExprType,
  getFieldInfo,
  getFunctionInfo,
  getLetType,
  getStructInfo
} from "../src/typeck/checker.js";
import type { BinaryExpression, LetStatement } from "../src/parser/ast.js";
import { SourceFile } from "../src/source/source-file.js";

function checkSource(text: string) {
  return check(new SourceFile("test.ik", text));
}

function messagesOf(text: string): string[] {
  return checkSource(text).diagnostics.map((diagnostic) => diagnostic.message);
}

interface NegativeCheckerCase {
  name: string;
  source: string;
  keywords: string[];
}

const negativeCheckerCases: NegativeCheckerCase[] = [
  {
    name: "unknown variable",
    source: `
      export fn bad(a: i32) -> i32 {
        return x;
      }
    `,
    keywords: ["unknown", "variable", "x"]
  },
  {
    name: "unknown function",
    source: `
      export fn bad(a: i32) -> i32 {
        return missing(a);
      }
    `,
    keywords: ["unknown", "function", "missing"]
  },
  {
    name: "unknown struct type",
    source: `
      export fn bad(x: ptr<Missing>) -> i32 {
        return 0;
      }
    `,
    keywords: ["unknown", "type", "Missing"]
  },
  {
    name: "unknown struct field",
    source: `
      struct Item {
        price: i64;
      }

      export fn bad(item: ptr<Item>) -> i64 {
        return item[0].qty;
      }
    `,
    keywords: ["field", "qty"]
  },
  {
    name: "duplicate struct name",
    source: `
      struct Item { price: i64; }
      struct Item { qty: i64; }
    `,
    keywords: ["duplicate", "struct", "Item"]
  },
  {
    name: "duplicate function name",
    source: `
      export fn calc() -> i32 { return 1; }
      export fn calc() -> i32 { return 2; }
    `,
    keywords: ["duplicate", "function", "calc"]
  },
  {
    name: "duplicate variable name",
    source: `
      export fn bad() -> i32 {
        let x: i32 = 1;
        let x: i32 = 2;
        return x;
      }
    `,
    keywords: ["duplicate", "variable", "x"]
  },
  {
    name: "wrong return type",
    source: `
      export fn bad() -> i32 {
        return true;
      }
    `,
    keywords: ["return", "expected", "i32", "bool"]
  },
  {
    name: "if condition is not bool",
    source: `
      export fn bad(a: i32) -> i32 {
        if a {
          return 1;
        }
        return 0;
      }
    `,
    keywords: ["if", "condition", "bool", "i32"]
  },
  {
    name: "while condition is not bool",
    source: `
      export fn bad(a: i32) -> i32 {
        while a {
          return 1;
        }
        return 0;
      }
    `,
    keywords: ["while", "condition", "bool", "i32"]
  },
  {
    name: "i32 plus i64 is not allowed",
    source: `
      export fn bad(a: i32, b: i64) -> i32 {
        return a + b;
      }
    `,
    keywords: ["arithmetic", "+", "same", "type"]
  },
  {
    name: "bool plus i32 is not allowed",
    source: `
      export fn bad(a: bool, b: i32) -> i32 {
        return a + b;
      }
    `,
    keywords: ["arithmetic", "+", "integer"]
  },
  {
    name: "wrong function argument count",
    source: `
      export fn add(a: i32, b: i32) -> i32 {
        return a + b;
      }

      export fn bad() -> i32 {
        return add(1);
      }
    `,
    keywords: ["function", "add", "expects", "2", "got", "1"]
  },
  {
    name: "wrong function argument type",
    source: `
      export fn add(a: i32, b: i32) -> i32 {
        return a + b;
      }

      export fn bad() -> i32 {
        return add(1, true);
      }
    `,
    keywords: ["argument", "2", "add", "expects", "i32", "bool"]
  },
  {
    name: "pointer index type is not i32 or u32",
    source: `
      struct Item {
        price: i64;
      }

      export fn bad(items: ptr<Item>, idx: bool) -> i64 {
        return items[idx].price;
      }
    `,
    keywords: ["index", "i32", "u32", "bool"]
  },
  {
    name: "assignment target is an integer literal",
    source: `
      export fn bad() -> i32 {
        1 = 2;
        return 0;
      }
    `,
    keywords: ["assignment", "target"]
  },
  {
    name: "assignment target is a function call",
    source: `
      export fn foo() -> i32 {
        return 1;
      }

      export fn bad() -> i32 {
        foo() = 2;
        return 0;
      }
    `,
    keywords: ["assignment", "target"]
  },
  {
    name: "assignment type mismatch",
    source: `
      export fn bad() -> i32 {
        let x: i32 = 1;
        x = true;
        return x;
      }
    `,
    keywords: ["assign", "bool", "i32"]
  },
  {
    name: "missing return",
    source: `
      export fn bad(a: i32) -> i32 {
        let x: i32 = a;
      }
    `,
    keywords: ["missing", "return"]
  },
  {
    name: "duplicate struct field",
    source: `
      struct Item {
        price: i64;
        price: i64;
      }
    `,
    keywords: ["duplicate", "field", "price"]
  }
];

function expectDiagnosticWithKeywords(source: string, keywords: string[]) {
  const diagnostics = checkSource(source).diagnostics;
  const matchingDiagnostic = diagnostics.find((diagnostic) => {
    const message = diagnostic.message.toLowerCase();
    return keywords.every((keyword) => message.includes(keyword.toLowerCase()));
  });

  expect(diagnostics.length, `expected at least one diagnostic for keywords: ${keywords.join(", ")}`).toBeGreaterThan(0);
  expect(
    matchingDiagnostic,
    `expected diagnostic containing keywords ${keywords.join(", ")} in messages:\n${diagnostics.map((diagnostic) => diagnostic.message).join("\n")}`
  ).toBeDefined();
  expect(matchingDiagnostic?.line).toEqual(expect.any(Number));
  expect(matchingDiagnostic?.column).toEqual(expect.any(Number));
  expect(matchingDiagnostic?.line).toBeGreaterThan(0);
  expect(matchingDiagnostic?.column).toBeGreaterThan(0);
}

describe("checker", () => {
  it("accepts a valid pricing-style program and records expression types", () => {
    const result = checkSource(`
      struct Item {
        price: i64;
        qty: i64;
        tax_rate_ppm: i64;
      }

      export fn tax(base: i64, ppm: i64) -> i64 {
        return base * ppm / 1000000;
      }

      export fn calc(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
        let i: i32 = 0;
        while i < len {
          let subtotal: i64 = items[i].price * items[i].qty;
          if subtotal > 0 {
            out[i] = subtotal + tax(subtotal, items[i].tax_rate_ppm);
          } else {
            out[i] = 0;
          }
          i = i + 1;
        }
        return 0;
      }
    `);

    expect(result.diagnostics).toEqual([]);
    expect(result.typedAst.program.kind).toBe("Program");
    expect([...result.typedAst.expressionTypes.values()].some((type) => type.kind === "primitive" && type.name === "bool")).toBe(true);
    expect([...result.typedAst.expressionTypes.values()].some((type) => type.kind === "pointer")).toBe(true);
  });

  it("exposes a stable checked program contract for MIR lowering", () => {
    const result = checkSource(`
      struct Item {
        price: i64;
        qty: i64;
      }

      export fn add(a: i64, b: i64) -> i64 {
        return a + b;
      }

      export fn calc(item: ptr<Item>) -> i64 {
        let subtotal: i64 = item[0].price + add(1, 2);
        return subtotal;
      }
    `);

    expect(result.diagnostics).toEqual([]);

    const checkedProgram = result.checkedProgram;
    const addInfo = getFunctionInfo(checkedProgram, "add");
    const calcInfo = getFunctionInfo(checkedProgram, "calc");
    const itemInfo = getStructInfo(checkedProgram, "Item");
    const priceInfo = getFieldInfo(checkedProgram, "Item", "price");

    expect(addInfo?.exported).toBe(true);
    expect(addInfo?.returnType).toEqual({ kind: "primitive", name: "i64" });
    expect(addInfo?.params.map((param) => [param.name, param.type])).toEqual([
      ["a", { kind: "primitive", name: "i64" }],
      ["b", { kind: "primitive", name: "i64" }]
    ]);
    expect(calcInfo?.params[0]?.type).toEqual({
      kind: "pointer",
      elementType: { kind: "struct", name: "Item" }
    });
    expect(itemInfo?.fields.map((field) => field.name)).toEqual(["price", "qty"]);
    expect(priceInfo?.type).toEqual({ kind: "primitive", name: "i64" });

    const calcDeclaration = calcInfo?.declaration;
    expect(calcDeclaration).toBeDefined();
    const letStatement = calcDeclaration!.body.statements[0] as LetStatement;
    const initializer = letStatement.initializer as BinaryExpression;

    expect(getLetType(checkedProgram, letStatement)).toEqual({ kind: "primitive", name: "i64" });
    expect(getExprType(checkedProgram, initializer)).toEqual({ kind: "primitive", name: "i64" });
    expect(getExprType(checkedProgram, initializer.left)).toEqual({ kind: "primitive", name: "i64" });
    expect(getExprType(checkedProgram, initializer.right)).toEqual({ kind: "primitive", name: "i64" });
  });

  it("reports duplicate declarations, variables, and unknown names", () => {
    expect(messagesOf(`
      struct Item { price: i64; }
      struct Item { qty: i64; }

      export fn f(a: i32, a: i32) -> i32 {
        let x: Missing = 0;
        let x: i32 = y;
        return x;
      }

      export fn f() -> i32 {
        return 0;
      }
    `)).toEqual(
      expect.arrayContaining([
        "Duplicate struct 'Item'.",
        "Duplicate function 'f'.",
        "Duplicate variable 'a'.",
        "Unknown type 'Missing'.",
        "Duplicate variable 'x'.",
        "Unknown variable 'y'."
      ])
    );
  });

  it("reports field, call, condition, return, assignment, arithmetic, and index errors", () => {
    expect(messagesOf(`
      struct Item { price: i64; }

      export fn takes_i64(value: i64) -> i64 {
        return value;
      }

      export fn bad(items: ptr<Item>, flag: bool, idx64: i64) -> i32 {
        let value: i64 = items[idx64].missing;
        let mixed: i64 = value + flag;
        if value {
          value = flag;
        } else {
          value = takes_i64(flag, value);
        }
        while 1 {
          value = value + 1;
        }
        return value == 0;
      }
    `)).toEqual(
      expect.arrayContaining([
        "Index expression requires i32 or u32 index, got i64.",
        "Struct 'Item' has no field 'missing'.",
        "Arithmetic operator '+' requires integer operands of the same type.",
        "If condition must be bool, got i64.",
        "Cannot assign bool to i64.",
        "Function 'takes_i64' expects 1 argument but got 2.",
        "Argument 1 of function 'takes_i64' expects i64 but got bool.",
        "While condition must be bool, got i32.",
        "Return type mismatch: expected i32 but got bool."
      ])
    );
  });

  it("reports diagnostics with line and column", () => {
    const result = checkSource(`
      export fn bad() -> i32 {
        return missing;
      }
    `);

    expect(result.diagnostics).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          code: "IK2001",
          severity: "error",
          message: "Unknown variable 'missing'.",
          fileName: "test.ik",
          line: 3,
          column: 16
        })
      ])
    );
    expect(result.diagnostics[0]?.span).toMatchObject({
      start: { line: 3, column: 16 }
    });
  });

  describe("negative cases", () => {
    it.each(negativeCheckerCases)("$name", ({ source, keywords }) => {
      expectDiagnosticWithKeywords(source, keywords);
    });
  });
});
