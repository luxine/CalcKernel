import { describe, expect, it } from "vitest";
import type { Expression } from "../src/parser/ast.js";
import { parse } from "../src/parser/parser.js";
import { SourceFile } from "../src/source/source-file.js";

function parseSource(text: string) {
  return parse(new SourceFile("test.ck", text));
}

function parseReturnExpression(text: string): Expression {
  const result = parseSource(text);
  expect(result.diagnostics).toEqual([]);

  const declaration = result.ast.declarations.find((candidate) => candidate.kind === "FunctionDeclaration");
  expect(declaration).toBeDefined();
  if (!declaration) {
    throw new Error("Expected function declaration.");
  }

  expect(declaration.kind).toBe("FunctionDeclaration");
  if (declaration.kind !== "FunctionDeclaration") {
    throw new Error("Expected function declaration.");
  }

  const statement = declaration.body.statements[0];
  expect(statement.kind).toBe("ReturnStatement");
  if (statement.kind !== "ReturnStatement") {
    throw new Error("Expected return statement.");
  }

  return statement.value;
}

describe("parser", () => {
  it("parses struct declarations with typed fields", () => {
    const result = parseSource(`
      struct Item {
        price: i64;
        qty: i32;
      }
    `);

    expect(result.diagnostics).toEqual([]);
    expect(result.ast.declarations[0]).toMatchObject({
      kind: "StructDeclaration",
      name: { kind: "Identifier", name: "Item" },
      fields: [
        { kind: "StructField", name: { name: "price" }, type: { kind: "PrimitiveType", name: "i64" } },
        { kind: "StructField", name: { name: "qty" }, type: { kind: "PrimitiveType", name: "i32" } }
      ]
    });
    expect(result.ast.declarations[0]?.span.start.line).toBe(2);
  });

  it("parses f64 primitive types", () => {
    const result = parseSource(`
      export fn scale(value: f64) -> f64 {
        return value;
      }
    `);

    expect(result.diagnostics).toEqual([]);
    expect(result.ast.declarations[0]).toMatchObject({
      kind: "FunctionDeclaration",
      params: [{ type: { kind: "PrimitiveType", name: "f64" } }],
      returnType: { kind: "PrimitiveType", name: "f64" }
    });
  });

  it("parses export functions, params, return type, and core statements", () => {
    const result = parseSource(`
      export fn calc(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
        let i: i32 = 0;
        while i < len {
          out[i] = items[i].price + compute(i, len);
          i = i + 1;
        }
        return 0;
      }
    `);

    expect(result.diagnostics).toEqual([]);
    expect(result.ast.declarations[0]).toMatchObject({
      kind: "FunctionDeclaration",
      exported: true,
      name: { name: "calc" },
      params: [
        { name: { name: "items" }, type: { kind: "PointerType", elementType: { kind: "NamedType" } } },
        { name: { name: "len" }, type: { kind: "PrimitiveType", name: "i32" } },
        { name: { name: "out" }, type: { kind: "PointerType", elementType: { kind: "PrimitiveType", name: "i64" } } }
      ],
      returnType: { kind: "PrimitiveType", name: "i32" },
      body: {
        statements: [
          { kind: "LetStatement" },
          { kind: "WhileStatement" },
          { kind: "ReturnStatement" }
        ]
      }
    });

    const fn = result.ast.declarations[0];
    expect(fn.kind).toBe("FunctionDeclaration");
    if (fn.kind !== "FunctionDeclaration") {
      return;
    }

    const whileStatement = fn.body.statements[1];
    expect(whileStatement.kind).toBe("WhileStatement");
    if (whileStatement.kind !== "WhileStatement") {
      return;
    }

    const assignment = whileStatement.body.statements[0];
    expect(assignment).toMatchObject({
      kind: "AssignmentStatement",
      target: {
        kind: "IndexExpression",
        object: { kind: "IdentifierExpression", name: "out" }
      },
      value: {
        kind: "BinaryExpression",
        operator: "+",
        left: {
          kind: "FieldExpression",
          object: {
            kind: "IndexExpression",
            object: { kind: "IdentifierExpression", name: "items" }
          },
          field: { name: "price" }
        },
        right: {
          kind: "CallExpression",
          callee: { kind: "IdentifierExpression", name: "compute" }
        }
      }
    });
  });

  it("parses if else statements", () => {
    const result = parseSource(`
      export fn clamp(value: i64) -> i64 {
        if value < 0 {
          return 0;
        } else {
          return value;
        }
      }
    `);

    expect(result.diagnostics).toEqual([]);
    const declaration = result.ast.declarations[0];
    expect(declaration.kind).toBe("FunctionDeclaration");
    if (declaration.kind !== "FunctionDeclaration") {
      return;
    }

    expect(declaration.body.statements[0]).toMatchObject({
      kind: "IfStatement",
      condition: {
        kind: "BinaryExpression",
        operator: "<"
      },
      thenBlock: {
        statements: [{ kind: "ReturnStatement" }]
      },
      elseBlock: {
        statements: [{ kind: "ReturnStatement" }]
      }
    });
  });

  it("preserves binary expression precedence", () => {
    const result = parseSource(`
      export fn expr(a: i64, b: i64, c: i64, d: i64) -> i64 {
        return a + b * c == d || false;
      }
    `);

    expect(result.diagnostics).toEqual([]);
    const declaration = result.ast.declarations[0];
    expect(declaration.kind).toBe("FunctionDeclaration");
    if (declaration.kind !== "FunctionDeclaration") {
      return;
    }

    const statement = declaration.body.statements[0];
    expect(statement.kind).toBe("ReturnStatement");
    if (statement.kind !== "ReturnStatement") {
      return;
    }

    expect(statement.value).toMatchObject({
      kind: "BinaryExpression",
      operator: "||",
      left: {
        kind: "BinaryExpression",
        operator: "==",
        left: {
          kind: "BinaryExpression",
          operator: "+",
          right: {
            kind: "BinaryExpression",
            operator: "*"
          }
        }
      },
      right: {
        kind: "BoolLiteral",
        value: false
      }
    });
  });

  describe("expression precedence", () => {
    it("parses multiplication before addition", () => {
      const expression = parseReturnExpression(`
        export fn calc() -> i32 {
          return 1 + 2 * 3;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "+",
        left: { kind: "IntegerLiteral", text: "1" },
        right: {
          kind: "BinaryExpression",
          operator: "*",
          left: { kind: "IntegerLiteral", text: "2" },
          right: { kind: "IntegerLiteral", text: "3" }
        }
      });
    });

    it("parses float literals", () => {
      const expression = parseReturnExpression(`
        export fn calc() -> f64 {
          return 1.0;
        }
      `);

      expect(expression).toMatchObject({
        kind: "FloatLiteral",
        text: "1.0"
      });
    });

    it("parses unary minus before a float literal", () => {
      const expression = parseReturnExpression(`
        export fn calc() -> f64 {
          return -1.0;
        }
      `);

      expect(expression).toMatchObject({
        kind: "UnaryExpression",
        operator: "-",
        operand: {
          kind: "FloatLiteral",
          text: "1.0"
        }
      });
    });

    it("parses parenthesized addition before multiplication", () => {
      const expression = parseReturnExpression(`
        export fn calc() -> i32 {
          return (1 + 2) * 3;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "*",
        left: {
          kind: "ParenthesizedExpression",
          expression: {
            kind: "BinaryExpression",
            operator: "+",
            left: { kind: "IntegerLiteral", text: "1" },
            right: { kind: "IntegerLiteral", text: "2" }
          }
        },
        right: { kind: "IntegerLiteral", text: "3" }
      });
    });

    it("parses comparison below addition", () => {
      const expression = parseReturnExpression(`
        export fn calc(a: i32, b: i32, c: i32, d: i32) -> bool {
          return a + b < c + d;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "<",
        left: {
          kind: "BinaryExpression",
          operator: "+",
          left: { kind: "IdentifierExpression", name: "a" },
          right: { kind: "IdentifierExpression", name: "b" }
        },
        right: {
          kind: "BinaryExpression",
          operator: "+",
          left: { kind: "IdentifierExpression", name: "c" },
          right: { kind: "IdentifierExpression", name: "d" }
        }
      });
    });

    it("parses logical and below comparison", () => {
      const expression = parseReturnExpression(`
        export fn calc(a: i32, b: i32, c: i32, d: i32) -> bool {
          return a < b && c < d;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "&&",
        left: { kind: "BinaryExpression", operator: "<" },
        right: { kind: "BinaryExpression", operator: "<" }
      });
    });

    it("parses logical or below logical and", () => {
      const expression = parseReturnExpression(`
        export fn calc(a: bool, b: bool, c: bool) -> bool {
          return a || b && c;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "||",
        left: { kind: "IdentifierExpression", name: "a" },
        right: {
          kind: "BinaryExpression",
          operator: "&&",
          left: { kind: "IdentifierExpression", name: "b" },
          right: { kind: "IdentifierExpression", name: "c" }
        }
      });
    });

    it("parses logical not above logical or", () => {
      const expression = parseReturnExpression(`
        export fn calc(a: bool, b: bool) -> bool {
          return !a || b;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "||",
        left: {
          kind: "UnaryExpression",
          operator: "!",
          operand: { kind: "IdentifierExpression", name: "a" }
        },
        right: { kind: "IdentifierExpression", name: "b" }
      });
    });

    it("parses unary minus above multiplication", () => {
      const expression = parseReturnExpression(`
        export fn calc(a: i32, b: i32) -> i32 {
          return -a * b;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "*",
        left: {
          kind: "UnaryExpression",
          operator: "-",
          operand: { kind: "IdentifierExpression", name: "a" }
        },
        right: { kind: "IdentifierExpression", name: "b" }
      });
    });

    it("parses field and index access above multiplication", () => {
      const expression = parseReturnExpression(`
        struct Item {
          price: i64;
          qty: i64;
        }

        export fn calc(items: ptr<Item>) -> i64 {
          return items[0].price * items[0].qty;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "*",
        left: {
          kind: "FieldExpression",
          object: {
            kind: "IndexExpression",
            object: { kind: "IdentifierExpression", name: "items" },
            index: { kind: "IntegerLiteral", text: "0" }
          },
          field: { name: "price" }
        },
        right: {
          kind: "FieldExpression",
          object: {
            kind: "IndexExpression",
            object: { kind: "IdentifierExpression", name: "items" },
            index: { kind: "IntegerLiteral", text: "0" }
          },
          field: { name: "qty" }
        }
      });
    });

    it("parses combined index and field access", () => {
      const expression = parseReturnExpression(`
        struct Item {
          price: i64;
          qty: i64;
        }

        export fn calc(items: ptr<Item>, i: i32) -> i64 {
          return items[i].price + items[i].qty;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "+",
        left: {
          kind: "FieldExpression",
          object: {
            kind: "IndexExpression",
            object: { kind: "IdentifierExpression", name: "items" },
            index: { kind: "IdentifierExpression", name: "i" }
          },
          field: { name: "price" }
        },
        right: {
          kind: "FieldExpression",
          object: {
            kind: "IndexExpression",
            object: { kind: "IdentifierExpression", name: "items" },
            index: { kind: "IdentifierExpression", name: "i" }
          },
          field: { name: "qty" }
        }
      });
    });

    it("parses complex access and arithmetic composition", () => {
      const expression = parseReturnExpression(`
        struct Item {
          price: i64;
          qty: i64;
          discount: i64;
        }

        export fn calc(items: ptr<Item>, i: i32) -> i64 {
          return items[i].price * items[i].qty - items[i].discount;
        }
      `);

      expect(expression).toMatchObject({
        kind: "BinaryExpression",
        operator: "-",
        left: {
          kind: "BinaryExpression",
          operator: "*",
          left: { kind: "FieldExpression", field: { name: "price" } },
          right: { kind: "FieldExpression", field: { name: "qty" } }
        },
        right: { kind: "FieldExpression", field: { name: "discount" } }
      });
    });
  });

  it("adds line and column diagnostics for parser errors", () => {
    const result = parseSource(`
      export fn bad() -> i32 {
        let x: i32 = 1
        return x;
      }
    `);

    expect(result.diagnostics).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          code: "CK1001",
          severity: "error",
          message: "Expected ';' after let statement.",
          fileName: "test.ck",
          line: 4,
          column: 9
        })
      ])
    );
  });
});
