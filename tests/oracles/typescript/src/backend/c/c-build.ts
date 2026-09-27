import { mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { lowerToMir } from "../../mir/lower.js";
import { printMirModule } from "../../mir/mir-printer.js";
import { runMirPassPipeline } from "../../opt/mir-pass-manager.js";
import type { MirPassDebugFlags } from "../../opt/mir-pass.js";
import { buildMirOptimizationPipeline } from "../../opt/pipeline.js";
import { printMirPassPipeline } from "../../opt/pipeline.js";
import { resolveOptimizationLevel } from "../../optimization/options.js";
import { emitMirCSource } from "./mir-c-emitter.js";
import type { CheckResult } from "../../typeck/checker.js";
import { assertCanEmitC, emitCHeader } from "./c-header-emitter.js";
import { resolveOverflowMode, type CCodegenOptions } from "./c-options.js";

export type CKHostPlatform =
  | "aix"
  | "android"
  | "cygwin"
  | "darwin"
  | "freebsd"
  | "haiku"
  | "linux"
  | "netbsd"
  | "openbsd"
  | "sunos"
  | "win32"
  | (string & {});

export type BuildPlatform = CKHostPlatform;

export interface CKSystemError extends Error {
  code?: string;
  errno?: number;
  syscall?: string;
  path?: string;
}

export interface CommandResult {
  status: number | null;
  stdout: string;
  stderr: string;
  error?: CKSystemError;
}

export type CommandRunner = (command: string, args: string[]) => CommandResult;

interface MirOptimizationDebugOptions {
  mirDebug?: MirPassDebugFlags;
  writeDebug?: (text: string) => void;
}

export interface EmitCFilesOptions extends CCodegenOptions {
  cFile: string;
  headerFile: string;
  headerFileName: string;
  mirDebug?: MirPassDebugFlags;
  writeDebug?: (text: string) => void;
}

export interface EmitDefaultCSourceOptions extends CCodegenOptions, MirOptimizationDebugOptions {
  headerFileName: string;
}

export interface BuildSharedLibraryOptions extends EmitCFilesOptions {
  outputPath: string;
  platform: BuildPlatform;
  runCommand: CommandRunner;
}

export interface BuildSharedLibraryResult {
  ok: boolean;
  outputPath: string;
  message?: string;
}

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];
const buildDllFlag = "-DCK_BUILD_DLL";
let tempFileCounter = 0;

export function emitCFiles(checked: CheckResult, options: EmitCFilesOptions): void {
  const headerText = emitCHeader(checked, { overflowMode: options.overflowMode });
  const sourceText = emitDefaultCSource(checked, {
    headerFileName: options.headerFileName,
    overflowMode: options.overflowMode,
    optLevel: options.optLevel,
    mirDebug: options.mirDebug,
    writeDebug: options.writeDebug
  });

  mkdirSync(dirname(options.cFile), { recursive: true });
  mkdirSync(dirname(options.headerFile), { recursive: true });
  writeFileAtomic(options.headerFile, headerText);
  writeFileAtomic(options.cFile, sourceText);
}

export function emitDefaultCSource(checked: CheckResult, options: EmitDefaultCSourceOptions): string {
  assertCanEmitC(checked);
  const mir = lowerToMir(checked.checkedProgram);
  const optLevel = resolveOptimizationLevel(options);
  const overflowMode = resolveOverflowMode(options);
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
    overflowMode,
    targetBackend: "c",
    debug
  });

  if (debug.printMirAfterOpt) {
    writeDebug(`MIR after optimization:\n${printMirModule(optimized.module)}`);
  }

  if (optimized.validationErrors.length > 0) {
    throw new Error(`internal compiler error: MIR validation failed: ${optimized.validationErrors[0]!.message}`);
  }

  return emitMirCSource(optimized.module, { headerFileName: options.headerFileName, overflowMode, optLevel });
}

export function buildSharedLibrary(checked: CheckResult, options: BuildSharedLibraryOptions): BuildSharedLibraryResult {
  emitCFiles(checked, options);

  const clangProbe = options.runCommand("clang", ["--version"]);
  if (isMissingCommand(clangProbe)) {
    return {
      ok: false,
      outputPath: sharedLibraryOutputPath(options.outputPath, options.platform),
      message: "clang was not found. Install clang and make sure it is available on PATH."
    };
  }

  if (clangProbe.status !== 0) {
    return {
      ok: false,
      outputPath: sharedLibraryOutputPath(options.outputPath, options.platform),
      message: clangProbe.stderr || "Unable to run clang --version."
    };
  }

  const outputPath = sharedLibraryOutputPath(options.outputPath, options.platform);
  mkdirSync(dirname(outputPath), { recursive: true });
  const args = clangArgs(options.cFile, outputPath, options.platform);
  const result = options.runCommand("clang", args);

  if (isMissingCommand(result)) {
    return {
      ok: false,
      outputPath,
      message: "clang was not found. Install clang and make sure it is available on PATH."
    };
  }

  if (result.status !== 0) {
    return {
      ok: false,
      outputPath,
      message: result.stderr || `clang failed with exit code ${result.status ?? "unknown"}.`
    };
  }

  return { ok: true, outputPath };
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

function clangArgs(cFile: string, outputPath: string, platform: BuildPlatform): string[] {
  if (platform === "win32") {
    return [...strictClangFlags, buildDllFlag, "-shared", cFile, "-o", outputPath];
  }

  return [...strictClangFlags, buildDllFlag, "-shared", "-fPIC", cFile, "-o", outputPath];
}

function isMissingCommand(result: CommandResult): boolean {
  return result.error?.code === "ENOENT";
}

function writeFileAtomic(path: string, contents: string): void {
  tempFileCounter += 1;
  const tempPath = `${path}.tmp-${process.pid}-${tempFileCounter}`;

  try {
    writeFileSync(tempPath, contents);
    renameSync(tempPath, path);
  } catch (error) {
    rmSync(tempPath, { force: true });
    throw error;
  }
}
