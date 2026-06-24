import { mkdirSync } from "node:fs";
import { basename, dirname } from "node:path";
import { lowerToMir } from "../../mir/lower.js";
import { validateMirModule } from "../../mir/mir-validator.js";
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
}

export interface BuildLlvmSharedLibraryResult {
  ok: boolean;
  outputPath: string;
  kind: LlvmBuildKind;
  message?: string;
}

export type LlvmBuildKind = "dynamic" | "object";

export function emitDefaultLlvmIr(checked: CheckResult, options: { sourceFileName: string; targetTriple?: string }): string {
  const mir = lowerToMir(checked.checkedProgram);
  const validation = validateMirModule(mir);
  if (validation.errors.length > 0) {
    const details = validation.errors
      .map((error) => {
        const location = [error.functionName, error.blockLabel].filter(Boolean).join(":");
        return `  - ${location ? `${location}: ` : ""}${error.message}`;
      })
      .join("\n");
    throw new Error(`internal compiler error: MIR validation failed\n${details}`);
  }

  return emitMirLlvmModule(mir, { sourceFileName: basename(options.sourceFileName), targetTriple: options.targetTriple });
}

export function buildLlvmSharedLibrary(checked: CheckResult, options: BuildLlvmSharedLibraryOptions): BuildLlvmSharedLibraryResult {
  const llvmIr = emitDefaultLlvmIr(checked, { sourceFileName: options.sourceFileName, targetTriple: options.targetTriple });
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
  const result = options.runCommand("clang", clangArgs(options.llFile, outputPath, options.platform, options.kind));
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

function clangArgs(llFile: string, outputPath: string, platform: BuildPlatform, kind: LlvmBuildKind): string[] {
  if (kind === "object") {
    return ["-O3", "-c", llFile, "-o", outputPath];
  }

  if (platform === "win32") {
    return ["-O3", "-shared", llFile, "-o", outputPath];
  }

  return ["-O3", "-shared", "-fPIC", llFile, "-o", outputPath];
}

function isMissingCommand(result: { error?: NodeJS.ErrnoException }): boolean {
  return result.error?.code === "ENOENT";
}
