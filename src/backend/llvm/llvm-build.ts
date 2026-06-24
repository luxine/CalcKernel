import { mkdirSync } from "node:fs";
import { basename, dirname } from "node:path";
import { lowerToMir } from "../../mir/lower.js";
import { printMirModule } from "../../mir/mir-printer.js";
import { runMirPassPipeline } from "../../opt/mir-pass-manager.js";
import type { MirPassDebugFlags } from "../../opt/mir-pass.js";
import { buildMirOptimizationPipeline } from "../../opt/pipeline.js";
import { printMirPassPipeline } from "../../opt/pipeline.js";
import { resolveOptimizationLevel } from "../../optimization/options.js";
import type { OptimizationOptions } from "../../optimization/options.js";
import type { CheckResult } from "../../typeck/checker.js";
import type { BuildPlatform, CommandRunner } from "../c/c-build.js";
import { emitMirLlvmModule } from "./mir-llvm-emitter.js";

export interface BuildLlvmSharedLibraryOptions {
  kind: LlvmBuildKind;
  llFile: string;
  outputPath: string;
  platform: BuildPlatform;
  runCommand: CommandRunner;
  sourceFileName: string;
  targetTriple?: string;
  writeFile: (path: string, text: string) => void;
  optLevel?: OptimizationOptions["optLevel"];
  mirDebug?: MirPassDebugFlags;
  writeDebug?: (text: string) => void;
}

export interface BuildLlvmSharedLibraryResult {
  ok: boolean;
  outputPath: string;
  kind: LlvmBuildKind;
  message?: string;
}

export type LlvmBuildKind = "dynamic" | "object";

export function emitDefaultLlvmIr(
  checked: CheckResult,
  options: { sourceFileName: string; targetTriple?: string; mirDebug?: MirPassDebugFlags; writeDebug?: (text: string) => void } & OptimizationOptions
): string {
  const mir = lowerToMir(checked.checkedProgram);
  const optLevel = resolveOptimizationLevel(options);
  const pipeline = buildMirOptimizationPipeline(optLevel);
  const debug = options.mirDebug ?? {};
  const writeDebug = options.writeDebug ?? (() => {});

  if (debug.printPassPipeline) {
    writeDebug(`MIR pass pipeline: ${printMirPassPipeline(pipeline)}\n`);
  }
  if (debug.printMirBeforeOpt) {
    writeDebug(`MIR before optimization:\n${printMirModule(mir)}`);
  }

  const optimized = runMirPassPipeline(mir, pipeline, {
    optLevel,
    overflowMode: "unchecked",
    targetBackend: "llvm",
    debug
  });

  if (debug.printMirAfterOpt) {
    writeDebug(`MIR after optimization:\n${printMirModule(optimized.module)}`);
  }

  if (optimized.validationErrors.length > 0) {
    const details = optimized.validationErrors
      .map((error) => {
        const location = [error.functionName, error.blockLabel].filter(Boolean).join(":");
        return `  - ${location ? `${location}: ` : ""}${error.message}`;
      })
      .join("\n");
    throw new Error(`internal compiler error: MIR validation failed\n${details}`);
  }

  return emitMirLlvmModule(optimized.module, { sourceFileName: basename(options.sourceFileName), targetTriple: options.targetTriple, optLevel });
}

export function buildLlvmSharedLibrary(checked: CheckResult, options: BuildLlvmSharedLibraryOptions): BuildLlvmSharedLibraryResult {
  const optLevel = resolveOptimizationLevel(options);
  const llvmIr = emitDefaultLlvmIr(checked, {
    sourceFileName: options.sourceFileName,
    targetTriple: options.targetTriple,
    optLevel,
    mirDebug: options.mirDebug,
    writeDebug: options.writeDebug
  });
  options.writeFile(options.llFile, llvmIr);

  const outputPath = buildOutputPath(options.outputPath, options.platform, options.kind);
  const clangProbe = options.runCommand("clang", ["--version"]);
  if (isMissingCommand(clangProbe)) {
    return {
      ok: false,
      outputPath,
      kind: options.kind,
      message:
        "clang was not found. Install clang and make sure it is available on PATH.\n" +
        "You can still run emit-llvm to generate LLVM IR without clang."
    };
  }

  if (clangProbe.status !== 0) {
    return {
      ok: false,
      outputPath,
      kind: options.kind,
      message: clangProbe.stderr || "Unable to run clang --version."
    };
  }

  mkdirSync(dirname(outputPath), { recursive: true });
  const result = options.runCommand("clang", clangArgs(options.llFile, outputPath, options.platform, options.kind, optLevel));
  if (isMissingCommand(result)) {
    return {
      ok: false,
      outputPath,
      kind: options.kind,
      message:
        "clang was not found. Install clang and make sure it is available on PATH.\n" +
        "You can still run emit-llvm to generate LLVM IR without clang."
    };
  }

  if (result.status !== 0) {
    return {
      ok: false,
      outputPath,
      kind: options.kind,
      message: result.stderr || `clang failed with exit code ${result.status ?? "unknown"}.`
    };
  }

  return { ok: true, outputPath, kind: options.kind };
}

export function sharedLibraryOutputPath(outputPath: string, platform: BuildPlatform): string {
  if (/\.(so|dylib|dll)$/i.test(outputPath)) {
    return outputPath;
  }

  switch (platform) {
    case "darwin":
      return `${outputPath}.dylib`;
    case "win32":
      return `${outputPath}.dll`;
    default:
      return `${outputPath}.so`;
  }
}

export function objectOutputPath(outputPath: string, platform: BuildPlatform): string {
  if (/\.(o|obj)$/i.test(outputPath)) {
    return outputPath;
  }

  return platform === "win32" ? `${outputPath}.obj` : `${outputPath}.o`;
}

function buildOutputPath(outputPath: string, platform: BuildPlatform, kind: LlvmBuildKind): string {
  return kind === "object" ? objectOutputPath(outputPath, platform) : sharedLibraryOutputPath(outputPath, platform);
}

function clangArgs(llFile: string, outputPath: string, platform: BuildPlatform, kind: LlvmBuildKind, optLevel: number): string[] {
  const optimizationFlag = `-O${optLevel}`;

  if (kind === "object") {
    return [optimizationFlag, "-c", llFile, "-o", outputPath];
  }

  if (platform === "win32") {
    return [optimizationFlag, "-shared", llFile, "-o", outputPath];
  }

  return [optimizationFlag, "-shared", "-fPIC", llFile, "-o", outputPath];
}

function isMissingCommand(result: { error?: NodeJS.ErrnoException }): boolean {
  return result.error?.code === "ENOENT";
}
