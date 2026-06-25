import { describe, expect, it } from "vitest";
import { validateMirModule } from "../src/mir/mir-validator.js";
import type { MirBlock, MirFunction, MirModule, MirType, MirValue } from "../src/mir/mir.js";

const boolType: MirType = { kind: "primitive", name: "bool" };
const i32: MirType = { kind: "primitive", name: "i32" };
const i64: MirType = { kind: "primitive", name: "i64" };
const f64: MirType = { kind: "primitive", name: "f64" };

function param(name: string, type: MirType): MirValue {
  return { kind: "param", name, type };
}

function paramPlace(name: string, type: MirType) {
  return { kind: "param" as const, name, type };
}

function temp(name: string, type: MirType): MirValue {
  return { kind: "temp", name, type };
}

function local(name: string, type: MirType): MirValue {
  return { kind: "local", name, type };
}

function validAddFunction(): MirFunction {
  return {
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
            target: temp("t0", i64),
            op: "+",
            left: param("a", i64),
            right: param("b", i64)
          }
        ],
        terminator: { kind: "return", value: temp("t0", i64) }
      }
    ]
  };
}

function moduleWith(functions: MirFunction[]): MirModule {
  return { structs: [], functions };
}

function errorMessages(module: MirModule): string[] {
  return validateMirModule(module).errors.map((error) => error.message);
}

describe("MIR validator", () => {
  it("accepts valid scalar MIR", () => {
    expect(validateMirModule(moduleWith([validAddFunction()])).errors).toEqual([]);
  });

  it("accepts f64 arithmetic, unary neg, comparison, and return MIR", () => {
    const fn: MirFunction = {
      name: "calc_f64",
      exported: true,
      params: [
        { name: "a", type: f64 },
        { name: "b", type: f64 }
      ],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "const_float", target: temp("t0", f64), value: "1.0" },
            { kind: "binary", target: temp("t1", f64), op: "+", left: param("a", f64), right: temp("t0", f64) },
            { kind: "unary", target: temp("t2", f64), op: "neg", operand: temp("t1", f64) },
            { kind: "compare", target: temp("t3", boolType), op: "<", left: temp("t2", f64), right: param("b", f64) }
          ],
          terminator: { kind: "return", value: temp("t2", f64) }
        }
      ]
    };

    expect(validateMirModule(moduleWith([fn])).errors).toEqual([]);
  });

  it("accepts f64 load and store place MIR", () => {
    const quoteType: MirType = { kind: "struct", name: "Quote" };
    const module: MirModule = {
      structs: [{ name: "Quote", fields: [{ name: "price", type: f64 }] }],
      functions: [
        {
          name: "places_f64",
          exported: true,
          params: [
            { name: "items", type: { kind: "pointer", elementType: quoteType } },
            { name: "out", type: { kind: "pointer", elementType: f64 } },
            { name: "i", type: i32 }
          ],
          returnType: f64,
          locals: [],
          blocks: [
            {
              label: "bb0",
              instructions: [
                {
                  kind: "load",
                  target: temp("t0", f64),
                  place: {
                    kind: "field",
                    base: {
                    kind: "index",
                      base: paramPlace("items", { kind: "pointer", elementType: quoteType }),
                      index: param("i", i32),
                      type: quoteType
                    },
                    fieldName: "price",
                    type: f64
                  }
                },
                {
                  kind: "store",
                  place: {
                    kind: "index",
                    base: paramPlace("out", { kind: "pointer", elementType: f64 }),
                    index: param("i", i32),
                    type: f64
                  },
                  value: temp("t0", f64)
                }
              ],
              terminator: { kind: "return", value: temp("t0", f64) }
            }
          ]
        }
      ]
    };

    expect(validateMirModule(module).errors).toEqual([]);
  });

  it("accepts explicit i32 and u32 to f64 cast MIR", () => {
    const fn: MirFunction = {
      name: "casts",
      exported: true,
      params: [
        { name: "a", type: i32 },
        { name: "b", type: { kind: "primitive", name: "u32" } }
      ],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "cast", target: temp("t0", f64), op: "i32_to_f64", value: param("a", i32) },
            { kind: "cast", target: temp("t1", f64), op: "u32_to_f64", value: param("b", { kind: "primitive", name: "u32" }) },
            { kind: "binary", target: temp("t2", f64), op: "+", left: temp("t0", f64), right: temp("t1", f64) }
          ],
          terminator: { kind: "return", value: temp("t2", f64) }
        }
      ]
    };

    expect(validateMirModule(moduleWith([fn])).errors).toEqual([]);
  });

  it("rejects invalid explicit cast MIR", () => {
    const u32: MirType = { kind: "primitive", name: "u32" };
    const fn: MirFunction = {
      name: "bad_casts",
      exported: true,
      params: [
        { name: "a", type: u32 },
        { name: "flag", type: boolType }
      ],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "cast", target: temp("t0", f64), op: "i32_to_f64", value: param("a", u32) },
            { kind: "cast", target: temp("t1", i32), op: "u32_to_f64", value: param("a", u32) },
            { kind: "cast", target: temp("t2", f64), op: "i64_to_f64" as "i32_to_f64", value: param("flag", boolType) }
          ],
          terminator: { kind: "return", value: temp("t0", f64) }
        }
      ]
    };

    expect(errorMessages(moduleWith([fn]))).toEqual(
      expect.arrayContaining([
        "Cast 'i32_to_f64' input in function 'bad_casts' must be i32, got u32.",
        "Cast 'u32_to_f64' result in function 'bad_casts' must be f64, got i32.",
        "Unsupported cast 'i64_to_f64' in function 'bad_casts'."
      ])
    );
  });

  it("rejects duplicate block labels", () => {
    const fn = validAddFunction();
    fn.blocks.push({
      label: "bb0",
      instructions: [],
      terminator: { kind: "return", value: param("a", i64) }
    });

    expect(errorMessages(moduleWith([fn]))).toContain("Duplicate block label 'bb0' in function 'add_i64'.");
  });

  it("rejects missing branch targets", () => {
    const blocks: MirBlock[] = [
      {
        label: "bb0",
        instructions: [],
        terminator: { kind: "branch", condition: param("flag", boolType), thenLabel: "bb1", elseLabel: "missing" }
      },
      {
        label: "bb1",
        instructions: [],
        terminator: { kind: "return", value: param("value", i64) }
      }
    ];
    const fn: MirFunction = {
      name: "branch_missing",
      exported: false,
      params: [
        { name: "flag", type: boolType },
        { name: "value", type: i64 }
      ],
      returnType: i64,
      locals: [],
      blocks
    };

    expect(errorMessages(moduleWith([fn]))).toContain("Branch target 'missing' does not exist in function 'branch_missing'.");
  });

  it("rejects branch conditions that are not bool", () => {
    const fn: MirFunction = {
      name: "bad_branch",
      exported: false,
      params: [{ name: "a", type: i64 }],
      returnType: i64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [],
          terminator: { kind: "branch", condition: param("a", i64), thenLabel: "bb1", elseLabel: "bb2" }
        },
        { label: "bb1", instructions: [], terminator: { kind: "return", value: param("a", i64) } },
        { label: "bb2", instructions: [], terminator: { kind: "return", value: param("a", i64) } }
      ]
    };

    expect(errorMessages(moduleWith([fn]))).toContain("Branch condition in function 'bad_branch' must be bool, got i64.");
  });

  it("rejects binary type mismatches", () => {
    const fn = validAddFunction();
    fn.blocks[0].instructions[0] = {
      kind: "binary",
      target: temp("t0", i64),
      op: "+",
      left: param("a", i64),
      right: local("x", i32)
    };
    fn.locals.push({ name: "x", type: i32 });

    expect(errorMessages(moduleWith([fn]))).toContain("Binary operands for '+' in function 'add_i64' must have the same type, got i64 and i32.");
  });

  it("rejects f64 modulo", () => {
    const fn: MirFunction = {
      name: "bad_mod_f64",
      exported: true,
      params: [
        { name: "a", type: f64 },
        { name: "b", type: f64 }
      ],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [{ kind: "binary", target: temp("t0", f64), op: "%", left: param("a", f64), right: param("b", f64) }],
          terminator: { kind: "return", value: temp("t0", f64) }
        }
      ]
    };

    expect(errorMessages(moduleWith([fn]))).toContain("Binary operator '%' in function 'bad_mod_f64' does not support f64 operands.");
  });

  it("rejects f64 values on integer-only const_int and bool unary paths", () => {
    const fn: MirFunction = {
      name: "bad_f64_paths",
      exported: true,
      params: [{ name: "a", type: f64 }],
      returnType: f64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            { kind: "const_int", target: temp("t0", f64), value: "1" },
            { kind: "unary", target: temp("t1", boolType), op: "not", operand: param("a", f64) }
          ],
          terminator: { kind: "return", value: param("a", f64) }
        }
      ]
    };

    expect(errorMessages(moduleWith([fn]))).toEqual(
      expect.arrayContaining([
        "const_int target in function 'bad_f64_paths' must be integer, got f64.",
        "Unary not in function 'bad_f64_paths' requires bool operand, got f64."
      ])
    );
  });

  it("rejects return type mismatches", () => {
    const fn = validAddFunction();
    fn.params.push({ name: "flag", type: boolType });
    fn.blocks[0].terminator = { kind: "return", value: param("flag", boolType) };

    expect(errorMessages(moduleWith([fn]))).toContain("Return type mismatch in function 'add_i64': expected i64, got bool.");
  });

  it("rejects unknown values", () => {
    const fn = validAddFunction();
    fn.blocks[0].terminator = { kind: "return", value: temp("missing", i64) };

    expect(errorMessages(moduleWith([fn]))).toContain("Unknown temp '%missing' in function 'add_i64'.");
  });

  it("rejects call argument mismatches", () => {
    const callee = validAddFunction();
    const caller: MirFunction = {
      name: "caller",
      exported: true,
      params: [{ name: "a", type: i64 }],
      returnType: i64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            {
              kind: "call",
              target: temp("t0", i64),
              functionName: "add_i64",
              args: [param("a", i64)]
            }
          ],
          terminator: { kind: "return", value: temp("t0", i64) }
        }
      ]
    };

    expect(errorMessages(moduleWith([callee, caller]))).toContain("Call to 'add_i64' in function 'caller' expects 2 argument(s), got 1.");
  });

  it("rejects call argument type mismatches", () => {
    const callee = validAddFunction();
    const caller: MirFunction = {
      name: "caller",
      exported: true,
      params: [
        { name: "a", type: i64 },
        { name: "flag", type: boolType }
      ],
      returnType: i64,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            {
              kind: "call",
              target: temp("t0", i64),
              functionName: "add_i64",
              args: [param("a", i64), param("flag", boolType)]
            }
          ],
          terminator: { kind: "return", value: temp("t0", i64) }
        }
      ]
    };

    expect(errorMessages(moduleWith([callee, caller]))).toContain("Call argument 2 to 'add_i64' in function 'caller' must be i64, got bool.");
  });

  it("rejects call result type mismatches", () => {
    const callee = validAddFunction();
    const caller: MirFunction = {
      name: "caller",
      exported: true,
      params: [
        { name: "a", type: i64 },
        { name: "b", type: i64 }
      ],
      returnType: boolType,
      locals: [],
      blocks: [
        {
          label: "bb0",
          instructions: [
            {
              kind: "call",
              target: temp("t0", boolType),
              functionName: "add_i64",
              args: [param("a", i64), param("b", i64)]
            }
          ],
          terminator: { kind: "return", value: temp("t0", boolType) }
        }
      ]
    };

    expect(errorMessages(moduleWith([callee, caller]))).toContain("Call result for 'add_i64' in function 'caller' must be i64, got bool.");
  });

  it("rejects index places whose base is not a pointer", () => {
    const itemType: MirType = { kind: "struct", name: "Item" };
    const module: MirModule = {
      structs: [{ name: "Item", fields: [{ name: "price", type: i64 }] }],
      functions: [
        {
          name: "bad_index",
          exported: true,
          params: [],
          returnType: i64,
          locals: [{ name: "item", type: itemType }],
          blocks: [
            {
              label: "bb0",
              instructions: [
                {
                  kind: "load",
                  target: temp("t0", i64),
                  place: {
                    kind: "index",
                    base: { kind: "local", name: "item", type: itemType },
                    index: { kind: "const_int", text: "0", type: i32 },
                    type: i64
                  }
                }
              ],
              terminator: { kind: "return", value: temp("t0", i64) }
            }
          ]
        }
      ]
    };

    expect(errorMessages(module)).toContain("Index base in function 'bad_index' must be pointer, got Item.");
  });

  it("rejects field places for unknown fields", () => {
    const itemType: MirType = { kind: "struct", name: "Item" };
    const module: MirModule = {
      structs: [{ name: "Item", fields: [{ name: "price", type: i64 }] }],
      functions: [
        {
          name: "bad_field",
          exported: true,
          params: [],
          returnType: i64,
          locals: [{ name: "item", type: itemType }],
          blocks: [
            {
              label: "bb0",
              instructions: [
                {
                  kind: "load",
                  target: temp("t0", i64),
                  place: {
                    kind: "field",
                    base: { kind: "local", name: "item", type: itemType },
                    fieldName: "qty",
                    type: i64
                  }
                }
              ],
              terminator: { kind: "return", value: temp("t0", i64) }
            }
          ]
        }
      ]
    };

    expect(errorMessages(module)).toContain("Unknown field 'qty' on struct 'Item' in function 'bad_field'.");
  });

  it("rejects store value type mismatches", () => {
    const fn: MirFunction = {
      name: "bad_store",
      exported: true,
      params: [{ name: "flag", type: boolType }],
      returnType: i64,
      locals: [{ name: "x", type: i64 }],
      blocks: [
        {
          label: "bb0",
          instructions: [
            {
              kind: "store",
              place: { kind: "local", name: "x", type: i64 },
              value: param("flag", boolType)
            }
          ],
          terminator: { kind: "return", value: local("x", i64) }
        }
      ]
    };

    expect(errorMessages(moduleWith([fn]))).toContain("Store type mismatch in function 'bad_store': place is i64, value is bool.");
  });
});
