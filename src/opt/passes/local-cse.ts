import { printMirType } from "../../mir/mir-printer.js";
import type { MirInstruction, MirPlace, MirType, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";

interface CseEntry {
  value: MirValue;
  dependencies: Set<string>;
}

export const localCsePass: MirPass = {
  name: "local-cse",
  run(module) {
    let changed = false;

    for (const func of module.functions) {
      for (const block of func.blocks) {
        const expressions = new Map<string, CseEntry>();

        for (let index = 0; index < block.instructions.length; index += 1) {
          const instruction = block.instructions[index]!;

          if (instruction.kind === "store" || instruction.kind === "call") {
            expressions.clear();
          }

          const key = cseKey(instruction);
          const target = instructionTarget(instruction);
          if (key && target?.kind === "temp") {
            const existing = expressions.get(key);
            if (existing) {
              block.instructions[index] = { kind: "move", target, value: existing.value };
              changed = true;
            } else {
              expressions.set(key, { value: target, dependencies: collectInstructionDependencies(instruction) });
            }
          }

          if (target?.kind === "local" || target?.kind === "param") {
            invalidateDependency(expressions, dependencyKey(target));
          }
        }
      }
    }

    return { changed };
  }
};

function cseKey(instruction: MirInstruction): string | undefined {
  switch (instruction.kind) {
    case "binary":
      if (isFloatType(instruction.target.type)) {
        return floatBinaryCseKey(instruction);
      }
      return `binary:${instruction.op}:${printMirType(instruction.target.type)}:${orderedValueKeys(instruction.op, instruction.left, instruction.right).join(":")}`;
    case "compare":
      if (isFloatType(instruction.left.type) || isFloatType(instruction.right.type)) {
        return undefined;
      }
      return `compare:${instruction.op}:${printMirType(instruction.left.type)}:${orderedValueKeys(instruction.op, instruction.left, instruction.right).join(":")}`;
    case "unary":
      if (isFloatType(instruction.target.type) || isFloatType(instruction.operand.type)) {
        return floatUnaryCseKey(instruction);
      }
      return `unary:${instruction.op}:${printMirType(instruction.target.type)}:${valueKey(instruction.operand)}`;
    case "cast":
      return `cast:${instruction.op}:${printMirType(instruction.value.type)}:${printMirType(instruction.target.type)}:${valueKey(instruction.value)}`;
    default:
      return undefined;
  }
}

function floatBinaryCseKey(instruction: Extract<MirInstruction, { kind: "binary" }>): string | undefined {
  if (!isFloatType(instruction.target.type) || !isFloatType(instruction.left.type) || !isFloatType(instruction.right.type)) {
    return undefined;
  }
  if (instruction.op !== "+" && instruction.op !== "-" && instruction.op !== "*") {
    return undefined;
  }
  return `float-binary:${instruction.op}:${printMirType(instruction.target.type)}:${valueKey(instruction.left)}:${valueKey(instruction.right)}`;
}

function floatUnaryCseKey(instruction: Extract<MirInstruction, { kind: "unary" }>): string | undefined {
  if (!isFloatType(instruction.target.type) || !isFloatType(instruction.operand.type)) {
    return undefined;
  }
  if (instruction.op !== "neg") {
    return undefined;
  }
  return `float-unary:${instruction.op}:${printMirType(instruction.target.type)}:${valueKey(instruction.operand)}`;
}

function orderedValueKeys(op: string, left: MirValue, right: MirValue): [string, string] {
  const keys: [string, string] = [valueKey(left), valueKey(right)];
  if (op === "+" || op === "*" || op === "==" || op === "!=") {
    return keys[0] <= keys[1] ? keys : [keys[1], keys[0]];
  }
  return keys;
}

function collectInstructionDependencies(instruction: MirInstruction): Set<string> {
  const dependencies = new Set<string>();

  switch (instruction.kind) {
    case "binary":
    case "compare":
      collectValueDependencies(instruction.left, dependencies);
      collectValueDependencies(instruction.right, dependencies);
      break;
    case "unary":
      collectValueDependencies(instruction.operand, dependencies);
      break;
    case "cast":
      collectValueDependencies(instruction.value, dependencies);
      break;
    default:
      break;
  }

  return dependencies;
}

function collectValueDependencies(value: MirValue, dependencies: Set<string>): void {
  if (value.kind === "local" || value.kind === "param") {
    dependencies.add(dependencyKey(value));
  }
}

function invalidateDependency(expressions: Map<string, CseEntry>, dependency: string): void {
  for (const [key, entry] of expressions) {
    if (entry.dependencies.has(dependency)) {
      expressions.delete(key);
    }
  }
}

function dependencyKey(value: Extract<MirValue, { kind: "local" | "param" }>): string {
  return `${value.kind}:${value.name}`;
}

function valueKey(value: MirValue): string {
  switch (value.kind) {
    case "param":
    case "local":
    case "temp":
      return `${value.kind}:${value.name}:${printMirType(value.type)}`;
    case "const_int":
      return `const_int:${value.text}:${printMirType(value.type)}`;
    case "const_float":
      return `const_float:${value.text}:${printMirType(value.type)}`;
    case "const_bool":
      return `const_bool:${value.value ? "true" : "false"}`;
  }
}

function instructionTarget(instruction: MirInstruction): MirValue | undefined {
  switch (instruction.kind) {
    case "const_int":
    case "const_float":
    case "const_bool":
    case "move":
    case "binary":
    case "unary":
    case "compare":
    case "cast":
    case "address":
    case "load":
    case "call":
      return instruction.target;
    case "store":
      return undefined;
  }
}

function isFloatType(type: MirType): boolean {
  return type.kind === "primitive" && type.name === "f64";
}

export function placeKey(place: MirPlace): string {
  switch (place.kind) {
    case "param":
    case "local":
      return `${place.kind}:${place.name}:${printMirType(place.type)}`;
    case "deref":
      return `deref:${valueKey(place.pointer)}:${printMirType(place.type)}`;
    case "index":
      return `index:${placeKey(place.base)}:${valueKey(place.index)}:${printMirType(place.type)}`;
    case "field":
      return `field:${placeKey(place.base)}:${place.fieldName}:${printMirType(place.type)}`;
  }
}
