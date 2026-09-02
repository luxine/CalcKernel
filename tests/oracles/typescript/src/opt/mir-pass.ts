import type { MirModule } from "../mir/mir.js";
import type { Diagnostic } from "../source/diagnostics.js";
import type { OptimizationLevel } from "./opt-level.js";

export type MirPassTargetBackend = "mir" | "c" | "wasm" | "llvm";
export type MirPassOverflowMode = "unchecked" | "checked";

export interface MirPassDebugFlags {
  printPassPipeline?: boolean;
  printMirBeforeOpt?: boolean;
  printMirAfterOpt?: boolean;
}

export interface MirPassContext {
  optLevel: OptimizationLevel;
  overflowMode: MirPassOverflowMode;
  targetBackend: MirPassTargetBackend;
  debug: MirPassDebugFlags;
}

export interface MirPassResult {
  changed: boolean;
  diagnostics?: Diagnostic[];
}

export interface MirPass {
  name: string;
  run(module: MirModule, context: MirPassContext): MirPassResult;
}
