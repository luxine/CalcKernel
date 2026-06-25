import type { MirFunction, MirInstruction, MirModule, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";
import { analyzeNaturalLoops, type NaturalLoop } from "./loop-analysis.js";

export const loopInvariantCodeMotionPass: MirPass = {
  name: "loop-invariant-code-motion",
  run(module, context) {
    if (context.optLevel < 3 || context.overflowMode === "checked") {
      return { changed: false };
    }

    let changed = false;
    for (const func of module.functions) {
      changed = hoistLoopInvariants(func, module) || changed;
    }

    return { changed };
  }
};

function hoistLoopInvariants(func: MirFunction, module: MirModule): boolean {
  let changed = false;

  for (const loop of analyzeNaturalLoops(func)) {
    changed = hoistInLoop(func, loop, module) || changed;
  }

  return changed;
}

function hoistInLoop(func: MirFunction, loop: NaturalLoop, _module: MirModule): boolean {
  if (!loop.preheader) {
    return false;
  }

  const preheader = func.blocks.find((block) => block.label === loop.preheader);
  if (!preheader) {
    return false;
  }

  const loopBlocks = new Set(loop.blocks);
  const loopDefinedTemps = collectLoopDefinedTemps(func, loopBlocks);
  const loopAssignedLocals = collectLoopAssignedLocals(func, loopBlocks);
  const hoistedTemps = new Set<string>();
  const hoistedInstructions: MirInstruction[] = [];
  let changed = false;

  for (const block of func.blocks) {
    if (!loopBlocks.has(block.label)) {
      continue;
    }

    const kept: MirInstruction[] = [];
    for (const instruction of block.instructions) {
      if (isHoistableInstruction(instruction, loopDefinedTemps, loopAssignedLocals, hoistedTemps)) {
        hoistedInstructions.push(instruction);
        rememberHoistedTarget(instruction, hoistedTemps);
        changed = true;
      } else {
        kept.push(instruction);
      }
    }

    block.instructions = kept;
  }

  if (hoistedInstructions.length > 0) {
    preheader.instructions.push(...hoistedInstructions);
  }

  return changed;
}

function isHoistableInstruction(
  instruction: MirInstruction,
  loopDefinedTemps: Set<string>,
  loopAssignedLocals: Set<string>,
  hoistedTemps: Set<string>
): boolean {
  switch (instruction.kind) {
    case "const_int":
    case "const_bool":
      return instruction.target.kind === "temp";
    case "const_float":
      return false;
    case "binary":
      if (isFloatValue(instruction.target) || isFloatValue(instruction.left) || isFloatValue(instruction.right)) {
        return false;
      }
      return (
        instruction.target.kind === "temp" &&
        (instruction.op === "+" || instruction.op === "-" || instruction.op === "*") &&
        isInvariantValue(instruction.left, loopDefinedTemps, loopAssignedLocals, hoistedTemps) &&
        isInvariantValue(instruction.right, loopDefinedTemps, loopAssignedLocals, hoistedTemps)
      );
    case "move":
    case "unary":
    case "compare":
    case "address":
    case "load":
    case "store":
    case "call":
      return false;
  }
}

function isInvariantValue(value: MirValue, loopDefinedTemps: Set<string>, loopAssignedLocals: Set<string>, hoistedTemps: Set<string>): boolean {
  switch (value.kind) {
    case "const_int":
    case "const_bool":
    case "const_float":
    case "param":
      return true;
    case "local":
      return !loopAssignedLocals.has(value.name);
    case "temp":
      return !loopDefinedTemps.has(value.name) || hoistedTemps.has(value.name);
  }
}

function collectLoopDefinedTemps(func: MirFunction, loopBlocks: Set<string>): Set<string> {
  const temps = new Set<string>();
  for (const block of func.blocks) {
    if (!loopBlocks.has(block.label)) {
      continue;
    }
    for (const instruction of block.instructions) {
      const target = instructionTarget(instruction);
      if (target?.kind === "temp") {
        temps.add(target.name);
      }
    }
  }
  return temps;
}

function collectLoopAssignedLocals(func: MirFunction, loopBlocks: Set<string>): Set<string> {
  const locals = new Set<string>();
  for (const block of func.blocks) {
    if (!loopBlocks.has(block.label)) {
      continue;
    }
    for (const instruction of block.instructions) {
      const target = instructionTarget(instruction);
      if (target?.kind === "local") {
        locals.add(target.name);
      }
      if (instruction.kind === "store") {
        collectAssignedPlaceLocal(instruction.place, locals);
      }
    }
  }
  return locals;
}

function collectAssignedPlaceLocal(place: Extract<MirInstruction, { kind: "store" }>["place"], locals: Set<string>): void {
  switch (place.kind) {
    case "local":
      locals.add(place.name);
      return;
    case "param":
      return;
    case "deref":
      return;
    case "index":
    case "field":
      collectAssignedPlaceLocal(place.base, locals);
      return;
  }
}

function rememberHoistedTarget(instruction: MirInstruction, hoistedTemps: Set<string>): void {
  const target = instructionTarget(instruction);
  if (target?.kind === "temp") {
    hoistedTemps.add(target.name);
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
    case "address":
    case "load":
    case "call":
      return instruction.target;
    case "store":
      return undefined;
  }
}

function isFloatValue(value: MirValue): boolean {
  return value.type.kind === "primitive" && value.type.name === "f64";
}
