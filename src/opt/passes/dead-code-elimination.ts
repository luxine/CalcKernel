import type { MirFunction, MirInstruction, MirPlace, MirTerminator, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";

export const deadCodeEliminationPass: MirPass = {
  name: "dead-code-elimination",
  run(module) {
    let changed = false;

    for (const func of module.functions) {
      changed = eliminateDeadCodeInFunction(func) || changed;
    }

    return { changed };
  }
};

function eliminateDeadCodeInFunction(func: MirFunction): boolean {
  let changed = false;
  let removed = true;

  while (removed) {
    removed = false;
    const usedTemps = collectUsedTemps(func);

    for (const block of func.blocks) {
      const before = block.instructions.length;
      block.instructions = block.instructions.filter((instruction) => !isRemovableUnusedInstruction(instruction, usedTemps));
      if (block.instructions.length !== before) {
        removed = true;
        changed = true;
      }
    }
  }

  return changed;
}

function isRemovableUnusedInstruction(instruction: MirInstruction, usedTemps: Set<string>): boolean {
  if (!isPureRemovableInstruction(instruction)) {
    return false;
  }

  return instruction.target.kind === "temp" && !usedTemps.has(instruction.target.name);
}

function isPureRemovableInstruction(
  instruction: MirInstruction
): instruction is Extract<MirInstruction, { kind: "const_int" | "const_bool" | "move" | "binary" | "unary" | "compare" | "address" }> {
  return (
    instruction.kind === "const_int" ||
    instruction.kind === "const_bool" ||
    instruction.kind === "move" ||
    instruction.kind === "binary" ||
    instruction.kind === "unary" ||
    instruction.kind === "compare" ||
    instruction.kind === "address"
  );
}

function collectUsedTemps(func: MirFunction): Set<string> {
  const used = new Set<string>();

  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      collectInstructionUses(instruction, used);
    }
    collectTerminatorUses(block.terminator, used);
  }

  return used;
}

function collectInstructionUses(instruction: MirInstruction, used: Set<string>): void {
  switch (instruction.kind) {
    case "const_int":
    case "const_bool":
      return;
    case "move":
      collectValueUse(instruction.value, used);
      return;
    case "binary":
    case "compare":
      collectValueUse(instruction.left, used);
      collectValueUse(instruction.right, used);
      return;
    case "unary":
      collectValueUse(instruction.operand, used);
      return;
    case "address":
      collectPlaceUses(instruction.place, used);
      return;
    case "load":
      collectPlaceUses(instruction.place, used);
      return;
    case "store":
      collectPlaceUses(instruction.place, used);
      collectValueUse(instruction.value, used);
      return;
    case "call":
      for (const arg of instruction.args) {
        collectValueUse(arg, used);
      }
      return;
  }
}

function collectTerminatorUses(terminator: MirTerminator, used: Set<string>): void {
  switch (terminator.kind) {
    case "return":
      collectValueUse(terminator.value, used);
      return;
    case "branch":
      collectValueUse(terminator.condition, used);
      return;
    case "jump":
      return;
  }
}

function collectPlaceUses(place: MirPlace, used: Set<string>): void {
  switch (place.kind) {
    case "param":
    case "local":
      return;
    case "deref":
      collectValueUse(place.pointer, used);
      return;
    case "field":
      collectPlaceUses(place.base, used);
      return;
    case "index":
      collectPlaceUses(place.base, used);
      collectValueUse(place.index, used);
      return;
  }
}

function collectValueUse(value: MirValue, used: Set<string>): void {
  if (value.kind === "temp") {
    used.add(value.name);
  }
}
