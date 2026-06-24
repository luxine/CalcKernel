import type { MirBlock, MirFunction, MirInstruction, MirModule, MirTerminator, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";

interface InlineCandidate {
  func: MirFunction;
  block: MirBlock;
  returnValue: MirValue;
}

interface InlineState {
  callIndex: number;
  existingNames: Set<string>;
}

interface RewriteMaps {
  params: Map<string, MirValue>;
  locals: Map<string, string>;
  temps: Map<string, string>;
}

const INLINE_THRESHOLD_BY_LEVEL = {
  2: 8,
  3: 25
} as const;

export const inlineSmallFunctionsPass: MirPass = {
  name: "inline-small-functions",
  run(module, context) {
    if (context.optLevel < 2) {
      return { changed: false };
    }

    const threshold = context.optLevel === 2 ? INLINE_THRESHOLD_BY_LEVEL[2] : INLINE_THRESHOLD_BY_LEVEL[3];
    const cyclicFunctions = findCyclicFunctions(module.functions);
    const candidates = collectCandidates(module.functions, cyclicFunctions, threshold);
    let changed = false;

    for (const func of module.functions) {
      changed = inlineCallsInFunction(func, candidates) || changed;
    }

    if (changed) {
      changed = removeUnreferencedInternalFunctions(module) || changed;
    }

    return { changed };
  }
};

function collectCandidates(functions: MirFunction[], cyclicFunctions: Set<string>, threshold: number): Map<string, InlineCandidate> {
  const candidates = new Map<string, InlineCandidate>();

  for (const func of functions) {
    if (func.exported || cyclicFunctions.has(func.name) || func.blocks.length !== 1) {
      continue;
    }

    const block = func.blocks[0]!;
    if (block.terminator.kind !== "return" || block.instructions.length > threshold || !block.instructions.every(isInlineableInstruction)) {
      continue;
    }

    candidates.set(func.name, { func, block, returnValue: block.terminator.value });
  }

  return candidates;
}

function isInlineableInstruction(instruction: MirInstruction): boolean {
  return (
    instruction.kind === "const_int" ||
    instruction.kind === "const_bool" ||
    instruction.kind === "move" ||
    instruction.kind === "binary" ||
    instruction.kind === "unary" ||
    instruction.kind === "compare"
  );
}

function inlineCallsInFunction(func: MirFunction, candidates: Map<string, InlineCandidate>): boolean {
  const state: InlineState = {
    callIndex: 0,
    existingNames: collectFunctionValueNames(func)
  };
  let changed = false;

  for (const block of func.blocks) {
    const instructions: MirInstruction[] = [];

    for (const instruction of block.instructions) {
      if (instruction.kind !== "call") {
        instructions.push(instruction);
        continue;
      }

      const candidate = candidates.get(instruction.functionName);
      if (!candidate || candidate.func.name === func.name) {
        instructions.push(instruction);
        continue;
      }

      instructions.push(...instantiateCandidate(candidate, instruction, func, state));
      changed = true;
    }

    if (changed) {
      block.instructions = instructions;
    }
  }

  return changed;
}

function instantiateCandidate(candidate: InlineCandidate, call: Extract<MirInstruction, { kind: "call" }>, caller: MirFunction, state: InlineState): MirInstruction[] {
  const prefix = `inl${state.callIndex}`;
  state.callIndex += 1;

  const maps: RewriteMaps = {
    params: new Map(),
    locals: new Map(),
    temps: new Map()
  };

  for (let index = 0; index < candidate.func.params.length; index += 1) {
    const param = candidate.func.params[index]!;
    maps.params.set(param.name, cloneValue(call.args[index]!));
  }

  for (const local of candidate.func.locals) {
    const name = uniqueName(`${prefix}_${local.name}`, state.existingNames);
    maps.locals.set(local.name, name);
    caller.locals.push({ name, type: local.type });
  }

  const instructions = candidate.block.instructions.map((instruction) => cloneInstruction(instruction, maps, prefix, state.existingNames));
  instructions.push({ kind: "move", target: cloneValue(call.target), value: rewriteValue(candidate.returnValue, maps) });
  return instructions;
}

function cloneInstruction(instruction: MirInstruction, maps: RewriteMaps, prefix: string, existingNames: Set<string>): MirInstruction {
  switch (instruction.kind) {
    case "const_int":
      return { kind: "const_int", target: rewriteTarget(instruction.target, maps, prefix, existingNames), value: instruction.value };
    case "const_bool":
      return { kind: "const_bool", target: rewriteTarget(instruction.target, maps, prefix, existingNames), value: instruction.value };
    case "move":
      return { kind: "move", target: rewriteTarget(instruction.target, maps, prefix, existingNames), value: rewriteValue(instruction.value, maps) };
    case "binary":
      return {
        kind: "binary",
        target: rewriteTarget(instruction.target, maps, prefix, existingNames),
        op: instruction.op,
        left: rewriteValue(instruction.left, maps),
        right: rewriteValue(instruction.right, maps)
      };
    case "unary":
      return {
        kind: "unary",
        target: rewriteTarget(instruction.target, maps, prefix, existingNames),
        op: instruction.op,
        operand: rewriteValue(instruction.operand, maps)
      };
    case "compare":
      return {
        kind: "compare",
        target: rewriteTarget(instruction.target, maps, prefix, existingNames),
        op: instruction.op,
        left: rewriteValue(instruction.left, maps),
        right: rewriteValue(instruction.right, maps)
      };
    case "address":
    case "load":
    case "store":
    case "call":
      throw new Error(`Instruction '${instruction.kind}' is not inlineable.`);
  }
}

function rewriteTarget(target: MirValue, maps: RewriteMaps, prefix: string, existingNames: Set<string>): MirValue {
  switch (target.kind) {
    case "temp": {
      let name = maps.temps.get(target.name);
      if (!name) {
        name = uniqueName(`${prefix}_${target.name}`, existingNames);
        maps.temps.set(target.name, name);
      }
      return { ...target, name };
    }
    case "local": {
      const name = maps.locals.get(target.name);
      return name ? { ...target, name } : cloneValue(target);
    }
    case "param":
      return cloneValue(target);
    case "const_int":
    case "const_bool":
      return cloneValue(target);
  }
}

function rewriteValue(value: MirValue, maps: RewriteMaps): MirValue {
  switch (value.kind) {
    case "param":
      return maps.params.get(value.name) ?? cloneValue(value);
    case "local": {
      const name = maps.locals.get(value.name);
      return name ? { ...value, name } : cloneValue(value);
    }
    case "temp": {
      const name = maps.temps.get(value.name);
      return name ? { ...value, name } : cloneValue(value);
    }
    case "const_int":
    case "const_bool":
      return cloneValue(value);
  }
}

function cloneValue(value: MirValue): MirValue {
  return { ...value };
}

function collectFunctionValueNames(func: MirFunction): Set<string> {
  const names = new Set<string>();

  for (const param of func.params) {
    names.add(param.name);
  }
  for (const local of func.locals) {
    names.add(local.name);
  }
  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      const target = instructionTarget(instruction);
      if (target) {
        collectValueName(target, names);
      }
    }
  }

  return names;
}

function collectValueName(value: MirValue, names: Set<string>): void {
  switch (value.kind) {
    case "param":
    case "local":
    case "temp":
      names.add(value.name);
      return;
    case "const_int":
    case "const_bool":
      return;
  }
}

function uniqueName(base: string, existingNames: Set<string>): string {
  if (!existingNames.has(base)) {
    existingNames.add(base);
    return base;
  }

  let suffix = 1;
  while (existingNames.has(`${base}_${suffix}`)) {
    suffix += 1;
  }
  const name = `${base}_${suffix}`;
  existingNames.add(name);
  return name;
}

function removeUnreferencedInternalFunctions(module: MirModule): boolean {
  const referenced = new Set<string>();

  for (const func of module.functions) {
    for (const block of func.blocks) {
      for (const instruction of block.instructions) {
        if (instruction.kind === "call") {
          referenced.add(instruction.functionName);
        }
      }
    }
  }

  const before = module.functions.length;
  module.functions = module.functions.filter((func) => func.exported || referenced.has(func.name));
  return module.functions.length !== before;
}

function findCyclicFunctions(functions: MirFunction[]): Set<string> {
  const graph = new Map<string, Set<string>>();
  for (const func of functions) {
    graph.set(func.name, collectCallees(func));
  }

  const cyclic = new Set<string>();
  const visited = new Set<string>();
  const active = new Set<string>();
  const stack: string[] = [];

  for (const func of functions) {
    visit(func.name, graph, visited, active, stack, cyclic);
  }

  return cyclic;
}

function visit(name: string, graph: Map<string, Set<string>>, visited: Set<string>, active: Set<string>, stack: string[], cyclic: Set<string>): void {
  if (active.has(name)) {
    const cycleStart = stack.indexOf(name);
    for (const cycleName of stack.slice(cycleStart)) {
      cyclic.add(cycleName);
    }
    cyclic.add(name);
    return;
  }
  if (visited.has(name)) {
    return;
  }

  visited.add(name);
  active.add(name);
  stack.push(name);

  for (const callee of graph.get(name) ?? []) {
    if (graph.has(callee)) {
      visit(callee, graph, visited, active, stack, cyclic);
    }
  }

  stack.pop();
  active.delete(name);
}

function collectCallees(func: MirFunction): Set<string> {
  const callees = new Set<string>();
  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      if (instruction.kind === "call") {
        callees.add(instruction.functionName);
      }
    }
  }
  return callees;
}

function instructionTarget(instruction: MirInstruction): MirValue | undefined {
  switch (instruction.kind) {
    case "const_int":
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
