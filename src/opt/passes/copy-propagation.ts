import type { MirInstruction, MirPlace, MirTerminator, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";

export const copyPropagationPass: MirPass = {
  name: "copy-propagation",
  run(module) {
    let changed = false;

    for (const func of module.functions) {
      for (const block of func.blocks) {
        const copies = new Map<string, MirValue>();

        for (const instruction of block.instructions) {
          changed = rewriteInstruction(instruction, copies) || changed;

          if (instruction.kind === "call" || instruction.kind === "store") {
            copies.clear();
            continue;
          }

          const target = getInstructionTarget(instruction);
          if (target?.kind === "temp") {
            copies.delete(target.name);
          }

          if (instruction.kind === "move" && instruction.target.kind === "temp") {
            copies.set(instruction.target.name, instruction.value);
          }
        }

        changed = rewriteTerminator(block.terminator, copies) || changed;
      }
    }

    return { changed };
  }
};

function rewriteInstruction(instruction: MirInstruction, copies: Map<string, MirValue>): boolean {
  let changed = false;
  switch (instruction.kind) {
    case "const_int":
    case "const_float":
    case "const_bool":
      return false;
    case "move": {
      const value = resolveCopy(instruction.value, copies);
      if (value !== instruction.value) {
        instruction.value = value;
        changed = true;
      }
      return changed;
    }
    case "binary": {
      const left = resolveCopy(instruction.left, copies);
      const right = resolveCopy(instruction.right, copies);
      if (left !== instruction.left) {
        instruction.left = left;
        changed = true;
      }
      if (right !== instruction.right) {
        instruction.right = right;
        changed = true;
      }
      return changed;
    }
    case "unary": {
      const operand = resolveCopy(instruction.operand, copies);
      if (operand !== instruction.operand) {
        instruction.operand = operand;
        changed = true;
      }
      return changed;
    }
    case "compare": {
      const left = resolveCopy(instruction.left, copies);
      const right = resolveCopy(instruction.right, copies);
      if (left !== instruction.left) {
        instruction.left = left;
        changed = true;
      }
      if (right !== instruction.right) {
        instruction.right = right;
        changed = true;
      }
      return changed;
    }
    case "address":
      return rewritePlace(instruction.place, copies);
    case "load":
      return rewritePlace(instruction.place, copies);
    case "store": {
      changed = rewritePlace(instruction.place, copies);
      const value = resolveCopy(instruction.value, copies);
      if (value !== instruction.value) {
        instruction.value = value;
        changed = true;
      }
      return changed;
    }
    case "call": {
      for (let index = 0; index < instruction.args.length; index += 1) {
        const arg = instruction.args[index]!;
        const value = resolveCopy(arg, copies);
        if (value !== arg) {
          instruction.args[index] = value;
          changed = true;
        }
      }
      return changed;
    }
  }
}

function rewriteTerminator(terminator: MirTerminator, copies: Map<string, MirValue>): boolean {
  switch (terminator.kind) {
    case "return": {
      const value = resolveCopy(terminator.value, copies);
      if (value !== terminator.value) {
        terminator.value = value;
        return true;
      }
      return false;
    }
    case "branch": {
      const condition = resolveCopy(terminator.condition, copies);
      if (condition !== terminator.condition) {
        terminator.condition = condition;
        return true;
      }
      return false;
    }
    case "jump":
      return false;
  }
}

function rewritePlace(place: MirPlace, copies: Map<string, MirValue>): boolean {
  switch (place.kind) {
    case "param":
    case "local":
      return false;
    case "deref": {
      const pointer = resolveCopy(place.pointer, copies);
      if (pointer !== place.pointer) {
        place.pointer = pointer;
        return true;
      }
      return false;
    }
    case "field":
      return rewritePlace(place.base, copies);
    case "index": {
      let changed = rewritePlace(place.base, copies);
      const index = resolveCopy(place.index, copies);
      if (index !== place.index) {
        place.index = index;
        changed = true;
      }
      return changed;
    }
  }
}

function resolveCopy(value: MirValue, copies: Map<string, MirValue>): MirValue {
  let current = value;
  const seen = new Set<string>();

  while (current.kind === "temp") {
    if (seen.has(current.name)) {
      return current;
    }
    seen.add(current.name);
    const next = copies.get(current.name);
    if (!next) {
      return current;
    }
    current = next;
  }

  return current;
}

function getInstructionTarget(instruction: MirInstruction): MirValue | undefined {
  switch (instruction.kind) {
    case "const_int":
    case "const_float":
    case "const_bool":
    case "move":
    case "binary":
    case "unary":
    case "compare":
    case "address":
    case "load":
    case "call":
      return instruction.target;
    case "store":
      return undefined;
  }
}
