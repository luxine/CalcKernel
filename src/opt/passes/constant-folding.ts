import type { MirBinaryOp, MirCompareOp, MirInstruction, MirType, MirUnaryOp, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";

type KnownConstant = { kind: "int"; value: bigint; type: MirType } | { kind: "bool"; value: boolean; type: MirType };

export const constantFoldingPass: MirPass = {
  name: "constant-folding",
  run(module, context) {
    if (context.overflowMode === "checked") {
      return { changed: false };
    }

    let changed = false;

    for (const func of module.functions) {
      for (const block of func.blocks) {
        const constants = new Map<string, KnownConstant>();

        for (let index = 0; index < block.instructions.length; index += 1) {
          const instruction = block.instructions[index]!;
          const folded = foldInstruction(instruction, constants);
          if (folded) {
            block.instructions[index] = folded;
            rememberInstructionConstant(folded, constants);
            changed = true;
          } else {
            forgetInstructionTarget(instruction, constants);
            rememberInstructionConstant(instruction, constants);
          }
        }
      }
    }

    return { changed };
  }
};

function foldInstruction(instruction: MirInstruction, constants: Map<string, KnownConstant>): MirInstruction | undefined {
  switch (instruction.kind) {
    case "binary": {
      const left = getKnownConstant(instruction.left, constants);
      const right = getKnownConstant(instruction.right, constants);
      if (left?.kind !== "int" || right?.kind !== "int") {
        return undefined;
      }
      const value = foldBinary(instruction.op, left.value, right.value, instruction.target.type);
      return value === undefined ? undefined : { kind: "const_int", target: instruction.target, value: value.toString() };
    }
    case "compare": {
      const left = getKnownConstant(instruction.left, constants);
      const right = getKnownConstant(instruction.right, constants);
      if (!left || !right || left.kind !== right.kind) {
        return undefined;
      }
      const value =
        left.kind === "int" && right.kind === "int"
          ? foldIntCompare(instruction.op, left.value, right.value)
          : left.kind === "bool" && right.kind === "bool"
            ? foldBoolCompare(instruction.op, left.value, right.value)
            : undefined;
      return value === undefined ? undefined : { kind: "const_bool", target: instruction.target, value };
    }
    case "unary": {
      const operand = getKnownConstant(instruction.operand, constants);
      if (!operand) {
        return undefined;
      }
      return foldUnary(instruction.op, operand, instruction.target);
    }
    default:
      return undefined;
  }
}

function rememberInstructionConstant(instruction: MirInstruction, constants: Map<string, KnownConstant>): void {
  switch (instruction.kind) {
    case "const_int":
      remember(instruction.target, { kind: "int", value: BigInt(instruction.value), type: instruction.target.type }, constants);
      return;
    case "const_bool":
      remember(instruction.target, { kind: "bool", value: instruction.value, type: instruction.target.type }, constants);
      return;
    default:
      return;
  }
}

function forgetInstructionTarget(instruction: MirInstruction, constants: Map<string, KnownConstant>): void {
  const target = "target" in instruction ? instruction.target : undefined;
  if (target?.kind === "temp") {
    constants.delete(target.name);
  }
}

function getKnownConstant(value: MirValue, constants: Map<string, KnownConstant>): KnownConstant | undefined {
  switch (value.kind) {
    case "const_int":
      return { kind: "int", value: BigInt(value.text), type: value.type };
    case "const_bool":
      return { kind: "bool", value: value.value, type: value.type };
    case "temp":
      return constants.get(value.name);
    default:
      return undefined;
  }
}

function remember(value: MirValue, constant: KnownConstant, constants: Map<string, KnownConstant>): void {
  if (value.kind === "temp") {
    constants.set(value.name, constant);
  }
}

function foldBinary(op: MirBinaryOp, left: bigint, right: bigint, type: MirType): bigint | undefined {
  if (!isIntegerType(type)) {
    return undefined;
  }

  if ((op === "/" || op === "%") && right === 0n) {
    return undefined;
  }
  if ((op === "/" || op === "%") && isSignedIntegerType(type) && left === integerMin(type) && right === -1n) {
    return undefined;
  }

  let result: bigint;
  switch (op) {
    case "+":
      result = left + right;
      break;
    case "-":
      result = left - right;
      break;
    case "*":
      result = left * right;
      break;
    case "/":
      result = left / right;
      break;
    case "%":
      result = left % right;
      break;
  }

  return fitsIntegerType(result, type) ? result : undefined;
}

function foldUnary(op: MirUnaryOp, operand: KnownConstant, target: MirValue): MirInstruction | undefined {
  if (op === "not") {
    return operand.kind === "bool" ? { kind: "const_bool", target, value: !operand.value } : undefined;
  }

  if (operand.kind !== "int" || !isIntegerType(target.type)) {
    return undefined;
  }

  const value = -operand.value;
  return fitsIntegerType(value, target.type) ? { kind: "const_int", target, value: value.toString() } : undefined;
}

function foldIntCompare(op: MirCompareOp, left: bigint, right: bigint): boolean | undefined {
  switch (op) {
    case "==":
      return left === right;
    case "!=":
      return left !== right;
    case "<":
      return left < right;
    case "<=":
      return left <= right;
    case ">":
      return left > right;
    case ">=":
      return left >= right;
  }
}

function foldBoolCompare(op: MirCompareOp, left: boolean, right: boolean): boolean | undefined {
  switch (op) {
    case "==":
      return left === right;
    case "!=":
      return left !== right;
    default:
      return undefined;
  }
}

function fitsIntegerType(value: bigint, type: MirType): boolean {
  return isIntegerType(type) && value >= integerMin(type) && value <= integerMax(type);
}

function integerMin(type: MirType): bigint {
  if (type.kind !== "primitive") {
    throw new Error("Expected primitive integer MIR type.");
  }
  switch (type.name) {
    case "i32":
      return -(1n << 31n);
    case "i64":
      return -(1n << 63n);
    case "u32":
    case "u64":
      return 0n;
    case "bool":
      throw new Error("Expected integer MIR type.");
  }
}

function integerMax(type: MirType): bigint {
  if (type.kind !== "primitive") {
    throw new Error("Expected primitive integer MIR type.");
  }
  switch (type.name) {
    case "i32":
      return (1n << 31n) - 1n;
    case "i64":
      return (1n << 63n) - 1n;
    case "u32":
      return (1n << 32n) - 1n;
    case "u64":
      return (1n << 64n) - 1n;
    case "bool":
      throw new Error("Expected integer MIR type.");
  }
}

function isIntegerType(type: MirType): boolean {
  return type.kind === "primitive" && type.name !== "bool";
}

function isSignedIntegerType(type: MirType): boolean {
  return type.kind === "primitive" && (type.name === "i32" || type.name === "i64");
}
