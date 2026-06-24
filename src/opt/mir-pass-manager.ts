import type { MirModule } from "../mir/mir.js";
import { validateMirModule, type MirValidationError } from "../mir/mir-validator.js";
import type { Diagnostic } from "../source/diagnostics.js";
import type { MirPassContext } from "./mir-pass.js";
import type { MirOptimizationPipeline } from "./pipeline.js";

export interface MirPassRecord {
  name: string;
  changed: boolean;
}

export interface MirPassManagerResult {
  module: MirModule;
  changed: boolean;
  records: MirPassRecord[];
  diagnostics: Diagnostic[];
  validationErrors: MirValidationError[];
}

export function runMirPassPipeline(module: MirModule, pipeline: MirOptimizationPipeline, context: MirPassContext): MirPassManagerResult {
  const records: MirPassRecord[] = [];
  const diagnostics: Diagnostic[] = [];
  const validationErrors: MirValidationError[] = [];
  let changed = false;

  for (const pass of pipeline.passes) {
    const result = pass.run(module, context);
    records.push({ name: pass.name, changed: result.changed });
    changed = changed || result.changed;

    if (result.diagnostics) {
      diagnostics.push(...result.diagnostics);
    }

    if (pipeline.validateAfterEachPass) {
      validationErrors.push(...validateMirModule(module).errors);
    }
  }

  if (pipeline.passes.length === 0 || !pipeline.validateAfterEachPass) {
    validationErrors.push(...validateMirModule(module).errors);
  }

  return { module, changed, records, diagnostics, validationErrors };
}
