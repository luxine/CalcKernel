import type { MirFunction, MirTerminator } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";

export interface LoopBackEdge {
  from: string;
  to: string;
}

export interface NaturalLoop {
  header: string;
  backEdge: LoopBackEdge;
  blocks: Set<string>;
  preheader?: string;
  exitBlocks: string[];
}

export const loopAnalysisPass: MirPass = {
  name: "loop-analysis",
  run() {
    return { changed: false };
  }
};

export function analyzeNaturalLoops(func: MirFunction): NaturalLoop[] {
  if (func.blocks.length === 0) {
    return [];
  }

  const labels = func.blocks.map((block) => block.label);
  const labelSet = new Set(labels);
  const successors = buildSuccessors(func);
  const predecessors = buildPredecessors(labels, successors);
  const dominators = computeDominators(labels, successors, predecessors);
  const loops: NaturalLoop[] = [];

  for (const block of func.blocks) {
    for (const target of successors.get(block.label) ?? []) {
      if (!labelSet.has(target)) {
        continue;
      }
      if (!dominators.get(block.label)?.has(target)) {
        continue;
      }

      const loopBlocks = collectNaturalLoopBlocks(target, block.label, predecessors);
      const loop = describeSimpleLoop(func, { from: block.label, to: target }, loopBlocks, predecessors, successors);
      if (loop) {
        loops.push(loop);
      }
    }
  }

  return loops.sort((left, right) => labels.indexOf(left.header) - labels.indexOf(right.header));
}

function describeSimpleLoop(
  func: MirFunction,
  backEdge: LoopBackEdge,
  blocks: Set<string>,
  predecessors: Map<string, Set<string>>,
  successors: Map<string, string[]>
): NaturalLoop | undefined {
  const headerBlock = func.blocks.find((block) => block.label === backEdge.to);
  if (!headerBlock || headerBlock.terminator.kind !== "branch") {
    return undefined;
  }

  const outsideHeaderPredecessors = [...(predecessors.get(backEdge.to) ?? [])].filter((label) => !blocks.has(label));
  const preheader = outsideHeaderPredecessors.length === 1 ? outsideHeaderPredecessors[0] : undefined;
  if (!preheader) {
    return undefined;
  }

  const preheaderBlock = func.blocks.find((block) => block.label === preheader);
  if (!preheaderBlock || preheaderBlock.terminator.kind !== "jump" || preheaderBlock.terminator.label !== backEdge.to) {
    return undefined;
  }

  const exitBlocks = new Set<string>();
  for (const label of blocks) {
    for (const successor of successors.get(label) ?? []) {
      if (!blocks.has(successor)) {
        exitBlocks.add(successor);
      }
    }
  }

  return { header: backEdge.to, backEdge, blocks, preheader, exitBlocks: [...exitBlocks].sort(blockOrder(func)) };
}

function collectNaturalLoopBlocks(header: string, source: string, predecessors: Map<string, Set<string>>): Set<string> {
  const blocks = new Set<string>([header, source]);
  const worklist = [source];

  while (worklist.length > 0) {
    const label = worklist.pop()!;
    for (const predecessor of predecessors.get(label) ?? []) {
      if (blocks.has(predecessor)) {
        continue;
      }
      blocks.add(predecessor);
      worklist.push(predecessor);
    }
  }

  return blocks;
}

function computeDominators(labels: string[], successors: Map<string, string[]>, predecessors: Map<string, Set<string>>): Map<string, Set<string>> {
  const entry = labels[0]!;
  const allLabels = new Set(labels);
  const dominators = new Map<string, Set<string>>();

  for (const label of labels) {
    dominators.set(label, label === entry ? new Set([entry]) : new Set(allLabels));
  }

  let changed = true;
  while (changed) {
    changed = false;
    for (const label of labels.slice(1)) {
      const preds = [...(predecessors.get(label) ?? [])].filter((pred) => dominators.has(pred));
      const next = new Set<string>(allLabels);
      for (const pred of preds) {
        intersectInPlace(next, dominators.get(pred)!);
      }
      next.add(label);

      if (!sameSet(next, dominators.get(label)!)) {
        dominators.set(label, next);
        changed = true;
      }
    }
  }

  for (const [label, successorList] of successors) {
    if (!dominators.has(label)) {
      dominators.set(label, new Set(successorList));
    }
  }

  return dominators;
}

function buildSuccessors(func: MirFunction): Map<string, string[]> {
  const successors = new Map<string, string[]>();
  for (const block of func.blocks) {
    successors.set(block.label, terminatorTargets(block.terminator));
  }
  return successors;
}

function buildPredecessors(labels: string[], successors: Map<string, string[]>): Map<string, Set<string>> {
  const predecessors = new Map(labels.map((label) => [label, new Set<string>()]));
  for (const [label, successorList] of successors) {
    for (const successor of successorList) {
      const preds = predecessors.get(successor);
      if (preds) {
        preds.add(label);
      }
    }
  }
  return predecessors;
}

function terminatorTargets(terminator: MirTerminator): string[] {
  switch (terminator.kind) {
    case "jump":
      return [terminator.label];
    case "branch":
      return [terminator.thenLabel, terminator.elseLabel];
    case "return":
      return [];
  }
}

function intersectInPlace(target: Set<string>, source: Set<string>): void {
  for (const value of target) {
    if (!source.has(value)) {
      target.delete(value);
    }
  }
}

function sameSet(left: Set<string>, right: Set<string>): boolean {
  if (left.size !== right.size) {
    return false;
  }
  for (const value of left) {
    if (!right.has(value)) {
      return false;
    }
  }
  return true;
}

function blockOrder(func: MirFunction): (left: string, right: string) => number {
  const indexes = new Map(func.blocks.map((block, index) => [block.label, index]));
  return (left, right) => (indexes.get(left) ?? Number.MAX_SAFE_INTEGER) - (indexes.get(right) ?? Number.MAX_SAFE_INTEGER);
}
