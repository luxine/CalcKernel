import type { MirPass } from "./mir-pass.js";
import type { OptimizationLevel } from "./opt-level.js";
import { constantFoldingPass } from "./passes/constant-folding.js";
import { copyPropagationPass } from "./passes/copy-propagation.js";
import { cfgSimplifyPass } from "./passes/cfg-simplify.js";
import { addressCsePass } from "./passes/address-cse.js";
import { deadCodeEliminationPass } from "./passes/dead-code-elimination.js";
import { inlineSmallFunctionsPass } from "./passes/inline-small-functions.js";
import { inductionSimplifyPass } from "./passes/induction-simplify.js";
import { loopAnalysisPass } from "./passes/loop-analysis.js";
import { loopInvariantCodeMotionPass } from "./passes/loop-invariant-code-motion.js";
import { localCsePass } from "./passes/local-cse.js";

export interface MirOptimizationPipeline {
  optLevel: OptimizationLevel;
  passes: MirPass[];
  validateAfterEachPass: boolean;
}

export function buildMirOptimizationPipeline(optLevel: OptimizationLevel): MirOptimizationPipeline {
  const passes =
    optLevel === 0
      ? []
      : optLevel === 1
        ? [constantFoldingPass, copyPropagationPass, deadCodeEliminationPass, cfgSimplifyPass]
        : optLevel === 2
          ? [
              constantFoldingPass,
              copyPropagationPass,
              inlineSmallFunctionsPass,
              constantFoldingPass,
              copyPropagationPass,
              localCsePass,
              copyPropagationPass,
              addressCsePass,
              deadCodeEliminationPass,
              cfgSimplifyPass,
              deadCodeEliminationPass
            ]
          : [
            constantFoldingPass,
            copyPropagationPass,
            inlineSmallFunctionsPass,
            constantFoldingPass,
            copyPropagationPass,
            loopAnalysisPass,
            loopInvariantCodeMotionPass,
            inductionSimplifyPass,
            constantFoldingPass,
            copyPropagationPass,
            localCsePass,
            copyPropagationPass,
            addressCsePass,
            deadCodeEliminationPass,
            cfgSimplifyPass,
            deadCodeEliminationPass
          ];

  return {
    optLevel,
    passes,
    validateAfterEachPass: true
  };
}

export function printMirPassPipeline(pipeline: MirOptimizationPipeline): string {
  if (pipeline.passes.length === 0) {
    return `O${pipeline.optLevel}: <validator only>`;
  }

  return `O${pipeline.optLevel}: ${pipeline.passes.map((pass) => pass.name).join(" -> ")}`;
}
