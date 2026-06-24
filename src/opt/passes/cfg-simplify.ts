import type { MirBlock, MirFunction, MirTerminator, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";

export const cfgSimplifyPass: MirPass = {
  name: "cfg-simplify",
  run(module, context) {
    let changed = false;

    for (const func of module.functions) {
      if (context.optLevel >= 2) {
        changed = simplifyConstantBranches(func) || changed;
        changed = simplifyJumpTargets(func) || changed;
      }

      changed = removeUnreachableBlocks(func) || changed;

      if (context.optLevel >= 2) {
        changed = simplifyJumpTargets(func) || changed;
        changed = removeUnreachableBlocks(func) || changed;
      }
    }

    return { changed };
  }
};

function simplifyConstantBranches(func: MirFunction): boolean {
  let changed = false;
  const constants = collectConstBoolTemps(func);

  for (const block of func.blocks) {
    if (block.terminator.kind !== "branch") {
      continue;
    }

    const condition = getKnownBool(block.terminator.condition, constants);
    if (condition === undefined) {
      continue;
    }

    block.terminator = { kind: "jump", label: condition ? block.terminator.thenLabel : block.terminator.elseLabel };
    changed = true;
  }

  return changed;
}

function simplifyJumpTargets(func: MirFunction): boolean {
  let changed = false;
  const blocksByLabel = new Map(func.blocks.map((block) => [block.label, block]));

  for (const block of func.blocks) {
    const terminator = block.terminator;
    switch (terminator.kind) {
      case "jump": {
        const label = resolveEmptyJumpTarget(terminator.label, blocksByLabel);
        if (label !== terminator.label) {
          block.terminator = { kind: "jump", label };
          changed = true;
        }
        break;
      }
      case "branch": {
        const thenLabel = resolveEmptyJumpTarget(terminator.thenLabel, blocksByLabel);
        const elseLabel = resolveEmptyJumpTarget(terminator.elseLabel, blocksByLabel);
        if (thenLabel === elseLabel) {
          block.terminator = { kind: "jump", label: thenLabel };
          changed = true;
          break;
        }
        if (thenLabel !== terminator.thenLabel || elseLabel !== terminator.elseLabel) {
          block.terminator = { ...terminator, thenLabel, elseLabel };
          changed = true;
        }
        break;
      }
      case "return":
        break;
    }
  }

  return changed;
}

function removeUnreachableBlocks(func: MirFunction): boolean {
  if (func.blocks.length === 0) {
    return false;
  }

  const reachable = collectReachableLabels(func);
  const before = func.blocks.length;
  func.blocks = func.blocks.filter((block) => reachable.has(block.label));
  return func.blocks.length !== before;
}

function collectReachableLabels(func: MirFunction): Set<string> {
  const blocksByLabel = new Map(func.blocks.map((block) => [block.label, block]));
  const reachable = new Set<string>();
  const worklist = [func.blocks[0]!.label];

  while (worklist.length > 0) {
    const label = worklist.pop()!;
    if (reachable.has(label)) {
      continue;
    }
    reachable.add(label);

    const block = blocksByLabel.get(label);
    if (!block) {
      continue;
    }

    for (const target of getTerminatorTargets(block.terminator)) {
      if (!reachable.has(target)) {
        worklist.push(target);
      }
    }
  }

  return reachable;
}

function getTerminatorTargets(terminator: MirTerminator): string[] {
  switch (terminator.kind) {
    case "jump":
      return [terminator.label];
    case "branch":
      return [terminator.thenLabel, terminator.elseLabel];
    case "return":
      return [];
  }
}

function resolveEmptyJumpTarget(label: string, blocksByLabel: Map<string, MirBlock>): string {
  let current = label;
  const seen = new Set<string>();

  while (!seen.has(current)) {
    seen.add(current);
    const block = blocksByLabel.get(current);
    if (!block || block.instructions.length !== 0 || block.terminator.kind !== "jump") {
      return current;
    }
    current = block.terminator.label;
  }

  return label;
}

function collectConstBoolTemps(func: MirFunction): Map<string, boolean> {
  const constants = new Map<string, boolean>();

  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      if (instruction.kind === "const_bool" && instruction.target.kind === "temp") {
        constants.set(instruction.target.name, instruction.value);
      }
    }
  }

  return constants;
}

function getKnownBool(value: MirValue, constants: Map<string, boolean>): boolean | undefined {
  switch (value.kind) {
    case "const_bool":
      return value.value;
    case "temp":
      return constants.get(value.name);
    default:
      return undefined;
  }
}
