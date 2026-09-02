import type { MirFunction, MirInstruction, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";
import type { NaturalLoop } from "./loop-analysis.js";
import { analyzeNaturalLoops } from "./loop-analysis.js";

export interface InductionVariable {
  localName: string;
  step: string;
  blockLabel: string;
}

export const inductionSimplifyPass: MirPass = {
  name: "induction-simplify",
  run(module) {
    for (const func of module.functions) {
      for (const loop of analyzeNaturalLoops(func)) {
        analyzeInductionVariables(func, loop);
      }
    }

    return { changed: false };
  }
};

export function analyzeInductionVariables(func: MirFunction, loop: NaturalLoop): InductionVariable[] {
  const inductions: InductionVariable[] = [];

  for (const block of func.blocks) {
    if (!loop.blocks.has(block.label)) {
      continue;
    }

    const constants = collectIntegerConstants(block.instructions);
    for (let index = 0; index < block.instructions.length - 1; index += 1) {
      const binary = block.instructions[index]!;
      const move = block.instructions[index + 1]!;
      if (binary.kind !== "binary" || move.kind !== "move" || binary.target.kind !== "temp" || move.value.kind !== "temp") {
        continue;
      }
      if (move.value.name !== binary.target.name || move.target.kind !== "local") {
        continue;
      }

      const step = inductionStep(move.target.name, binary, constants);
      if (step !== undefined) {
        inductions.push({ localName: move.target.name, step, blockLabel: block.label });
      }
    }
  }

  return inductions;
}

function collectIntegerConstants(instructions: MirInstruction[]): Map<string, string> {
  const constants = new Map<string, string>();
  for (const instruction of instructions) {
    if (instruction.kind === "const_int" && instruction.target.kind === "temp") {
      constants.set(instruction.target.name, instruction.value);
    }
  }
  return constants;
}

function inductionStep(localName: string, instruction: Extract<MirInstruction, { kind: "binary" }>, constants: Map<string, string>): string | undefined {
  if (instruction.op === "+") {
    const leftLocal = isLocalValue(instruction.left, localName);
    const rightLocal = isLocalValue(instruction.right, localName);
    if (leftLocal) {
      return integerConstant(instruction.right, constants);
    }
    if (rightLocal) {
      return integerConstant(instruction.left, constants);
    }
  }

  if (instruction.op === "-" && isLocalValue(instruction.left, localName)) {
    const value = integerConstant(instruction.right, constants);
    return value === undefined ? undefined : negateDecimalText(value);
  }

  return undefined;
}

function integerConstant(value: MirValue, constants: Map<string, string>): string | undefined {
  switch (value.kind) {
    case "const_int":
      return value.text;
    case "temp":
      return constants.get(value.name);
    default:
      return undefined;
  }
}

function isLocalValue(value: MirValue, localName: string): boolean {
  return value.kind === "local" && value.name === localName;
}

function negateDecimalText(value: string): string {
  return value.startsWith("-") ? value.slice(1) : `-${value}`;
}
