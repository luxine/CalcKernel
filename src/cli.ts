#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { emitCFiles, buildSharedLibrary, type BuildPlatform, type CommandResult, type CommandRunner } from "./backend/c/c-build.js";
import { defaultOverflowMode, type OverflowMode } from "./backend/c/c-options.js";
import { buildLlvmSharedLibrary, type LlvmBuildKind } from "./backend/llvm/llvm-build.js";
import { emitMirLlvmModule } from "./backend/llvm/mir-llvm-emitter.js";
import { detectNativeLlvmTargetTriple } from "./backend/llvm/llvm-target.js";
import { emitMirWatModule } from "./backend/wasm/mir-wat-emitter.js";
import { compileWatToWasm } from "./backend/wasm/wat-to-wasm.js";
import { lowerToMir } from "./mir/lower.js";
import type { MirModule } from "./mir/mir.js";
import { printMirModule } from "./mir/mir-printer.js";
import type { MirValidationError } from "./mir/mir-validator.js";
import { defaultOptimizationLevel, parseOptimizationLevel, type OptimizationLevel } from "./optimization/options.js";
import { runMirPassPipeline } from "./opt/mir-pass-manager.js";
import type { MirPassDebugFlags, MirPassOverflowMode, MirPassTargetBackend } from "./opt/mir-pass.js";
import { buildMirOptimizationPipeline, printMirPassPipeline } from "./opt/pipeline.js";
import { formatDiagnostics } from "./source/diagnostics.js";
import { SourceFile } from "./source/source-file.js";
import { check, type CheckedProgram, type CheckResult } from "./typeck/checker.js";

export type { CommandRunner };

export interface RunCliOptions {
  cwd?: string;
  platform?: BuildPlatform;
  stdout?: (text: string) => void;
  stderr?: (text: string) => void;
  runCommand?: CommandRunner;
}

interface FlagParseResult {
  positional: string[];
  flags: Map<string, string>;
}

const booleanFlags = new Set(["--print-pass-pipeline", "--print-mir-before-opt", "--print-mir-after-opt"]);

export function runCli(argv: string[] = process.argv.slice(2), options: RunCliOptions = {}): number {
  const context = cliContext(options);
  const [command, ...rest] = argv;

  try {
    if (command === "--help" || command === "-h") {
      context.stdout(usage());
      return 0;
    }

    switch (command) {
      case "check":
        return runCheck(rest, context);
      case "emit-c":
        return runEmitC(rest, context);
      case "emit-mir":
        return runEmitMir(rest, context);
      case "emit-llvm":
        return runEmitLlvm(rest, context);
      case "emit-wat":
        return runEmitWat(rest, context);
      case "emit-wasm":
        return runEmitWasm(rest, context);
      case "build":
        return runBuild(rest, context);
      case "build-llvm":
        return runBuildLlvm(rest, context);
      default:
        context.stderr(usage());
        return 2;
    }
  } catch (error) {
    context.stderr(`${error instanceof Error ? error.message : String(error)}\n`);
    return 1;
  }
}

function runCheck(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "check");
  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  context.stdout(`OK: ${file}\n`);
  return 0;
}

function runEmitC(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "emit-c");
  const cFile = requireFlag(parsed, "--out", "emit-c");
  const headerFile = requireFlag(parsed, "--header", "emit-c");
  const overflowMode = parseOverflowMode(parsed);
  const optLevel = parseOptLevel(parsed);
  const debug = parseMirDebugFlags(parsed);
  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const cPath = resolve(context.cwd, cFile);
  const headerPath = resolve(context.cwd, headerFile);
  mkdirSync(dirname(cPath), { recursive: true });
  mkdirSync(dirname(headerPath), { recursive: true });
  emitCFiles(checked.result, {
    cFile: cPath,
    headerFile: headerPath,
    headerFileName: basename(headerPath),
    overflowMode,
    optLevel,
    mirDebug: debug,
    writeDebug: context.stderr
  });
  context.stdout(`OK: emitted C with overflow=${overflowMode}\nWrote ${cPath}\nWrote ${headerPath}\n`);
  return 0;
}

function runEmitMir(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "emit-mir");
  const outFile = parsed.flags.get("--out");
  const optLevel = parseOptLevel(parsed);
  const debug = parseMirDebugFlags(parsed);
  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const optimizedMir = lowerAndOptimizeMir(checked.result.checkedProgram, {
    optLevel,
    overflowMode: "unchecked",
    targetBackend: "mir",
    debug,
    stderr: context.stderr
  });
  if (!optimizedMir.ok) {
    printInternalMirValidationErrors(context.stderr, optimizedMir.validationErrors);
    return 1;
  }

  const text = printMirModule(optimizedMir.module);

  if (!outFile) {
    context.stdout(text);
    return 0;
  }

  const outPath = resolve(context.cwd, outFile);
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, text);
  context.stdout(`OK: emitted MIR\nWrote ${outPath}\n`);
  return 0;
}

function runEmitLlvm(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "emit-llvm");
  const outFile = parsed.flags.get("--out");
  const overflowMode = parseOverflowMode(parsed);
  const optLevel = parseOptLevel(parsed);
  const debug = parseMirDebugFlags(parsed);
  const targetTriple = parsed.flags.get("--target");

  if (overflowMode === "checked") {
    throw unsupportedCheckedLlvmError();
  }

  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const optimizedMir = lowerAndOptimizeMir(checked.result.checkedProgram, {
    optLevel,
    overflowMode,
    targetBackend: "llvm",
    debug,
    stderr: context.stderr
  });
  if (!optimizedMir.ok) {
    printInternalMirValidationErrors(context.stderr, optimizedMir.validationErrors);
    return 1;
  }

  const text = emitMirLlvmModule(optimizedMir.module, { sourceFileName: file, targetTriple: targetTriple ?? detectNativeLlvmTargetTriple(), optLevel });

  if (!outFile) {
    context.stdout(text);
    return 0;
  }

  const outPath = resolve(context.cwd, outFile);
  writeFileAtomic(outPath, text);
  context.stdout(`OK: emitted LLVM IR ${outFile}\n`);
  return 0;
}

function runEmitWat(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "emit-wat");
  const outFile = parsed.flags.get("--out");
  const overflowMode = parseOverflowMode(parsed);
  const optLevel = parseOptLevel(parsed);
  const debug = parseMirDebugFlags(parsed);

  if (overflowMode === "checked") {
    throw unsupportedCheckedWasmError();
  }

  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const optimizedMir = lowerAndOptimizeMir(checked.result.checkedProgram, {
    optLevel,
    overflowMode,
    targetBackend: "wasm",
    debug,
    stderr: context.stderr
  });
  if (!optimizedMir.ok) {
    printInternalMirValidationErrors(context.stderr, optimizedMir.validationErrors);
    return 1;
  }

  const text = emitMirWatModule(optimizedMir.module, { optLevel });

  if (!outFile) {
    context.stdout(text);
    return 0;
  }

  const outPath = resolve(context.cwd, outFile);
  writeFileAtomic(outPath, text);
  context.stdout(`OK: emitted WAT ${outFile}\n`);
  return 0;
}

function runEmitWasm(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "emit-wasm");
  const outFile = requireFlag(parsed, "--out", "emit-wasm");
  const overflowMode = parseOverflowMode(parsed);
  const optLevel = parseOptLevel(parsed);
  const debug = parseMirDebugFlags(parsed);

  if (overflowMode === "checked") {
    throw unsupportedCheckedWasmError();
  }

  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const optimizedMir = lowerAndOptimizeMir(checked.result.checkedProgram, {
    optLevel,
    overflowMode,
    targetBackend: "wasm",
    debug,
    stderr: context.stderr
  });
  if (!optimizedMir.ok) {
    printInternalMirValidationErrors(context.stderr, optimizedMir.validationErrors);
    return 1;
  }

  const wat = emitMirWatModule(optimizedMir.module, { optLevel });
  const wasm = compileWatToWasm(wat, file);
  const outPath = resolve(context.cwd, outFile);
  writeFileAtomic(outPath, wasm);
  context.stdout(`OK: emitted WASM ${outFile}\n`);
  return 0;
}

function runBuild(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "build");
  const outputPath = requireFlag(parsed, "--out", "build");
  const overflowMode = parseOverflowMode(parsed);
  const optLevel = parseOptLevel(parsed);
  const debug = parseMirDebugFlags(parsed);
  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const absoluteOutputPath = resolve(context.cwd, outputPath);
  const cFile = `${absoluteOutputPath}.c`;
  const headerFile = `${absoluteOutputPath}.h`;
  const result = buildSharedLibrary(checked.result, {
    cFile,
    headerFile,
    headerFileName: basename(headerFile),
    outputPath: absoluteOutputPath,
    platform: context.platform,
    runCommand: context.runCommand,
    overflowMode,
    optLevel,
    mirDebug: debug,
    writeDebug: context.stderr
  });

  if (!result.ok) {
    context.stderr(`${result.message ?? "Build failed."}\n`);
    return 1;
  }

  context.stdout(`OK: built library with overflow=${overflowMode}\n${result.outputPath}\n`);
  return 0;
}

function runBuildLlvm(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "build-llvm");
  const outputPath = requireFlag(parsed, "--out", "build-llvm");
  const overflowMode = parseOverflowMode(parsed);
  const optLevel = parseOptLevel(parsed);
  const debug = parseMirDebugFlags(parsed);
  const targetTriple = parsed.flags.get("--target");
  const kind = parseLlvmBuildKind(parsed);

  if (overflowMode === "checked") {
    throw unsupportedCheckedLlvmError();
  }

  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const absoluteOutputPath = resolve(context.cwd, outputPath);
  const llFile = llvmIntermediateFilePath(absoluteOutputPath, kind);
  const result = buildLlvmSharedLibrary(checked.result, {
    kind,
    llFile,
    outputPath: absoluteOutputPath,
    platform: context.platform,
    runCommand: context.runCommand,
    sourceFileName: file,
    targetTriple,
    optLevel,
    mirDebug: debug,
    writeDebug: context.stderr,
    writeFile: writeFileAtomic
  });

  if (!result.ok) {
    context.stderr(`${result.message ?? "LLVM build failed."}\n`);
    return 1;
  }

  context.stdout(`OK: built LLVM ${result.kind === "object" ? "object" : "library"}\n${result.outputPath}\n`);
  return 0;
}

function llvmIntermediateFilePath(outputPath: string, kind: LlvmBuildKind): string {
  if (kind === "object") {
    return `${outputPath.replace(/\.(o|obj)$/i, "")}.ll`;
  }

  return `${outputPath}.ll`;
}

function checkFile(file: string, cwd: string): { result: CheckResult; source: SourceFile } {
  const path = resolve(cwd, file);
  const sourceText = readFileSync(path, "utf8");
  const source = new SourceFile(file, sourceText);
  return {
    result: check(source),
    source
  };
}

function parseFlags(args: string[]): FlagParseResult {
  const positional: string[] = [];
  const flags = new Map<string, string>();

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg.startsWith("-O")) {
      flags.set("--opt-level", arg.slice(2));
      continue;
    }

    if (!arg.startsWith("--")) {
      positional.push(arg);
      continue;
    }

    if (booleanFlags.has(arg)) {
      flags.set(arg, "true");
      continue;
    }

    const value = args[index + 1];
    if (!value || value.startsWith("--")) {
      throw new Error(`Missing value for ${arg}.`);
    }

    flags.set(arg, value);
    index += 1;
  }

  return { positional, flags };
}

function requireSingleInput(parsed: FlagParseResult, command: string): string {
  if (parsed.positional.length !== 1) {
    throw new Error(`Usage error for '${command}'.\n${usage()}`);
  }

  return parsed.positional[0]!;
}

function requireFlag(parsed: FlagParseResult, flag: string, command: string): string {
  const value = parsed.flags.get(flag);
  if (!value) {
    throw new Error(`Usage error for '${command}': missing ${flag}.\n${usage()}`);
  }

  return value;
}

function writeFileAtomic(path: string, data: string | Uint8Array): void {
  mkdirSync(dirname(path), { recursive: true });
  const tempPath = `${path}.tmp-${process.pid}-${Date.now()}`;

  try {
    writeFileSync(tempPath, data);
    renameSync(tempPath, path);
  } catch (error) {
    rmSync(tempPath, { force: true });
    throw error;
  }
}

function unsupportedCheckedWasmError(): Error {
  return new Error(
    "error: WASM backend does not support --overflow checked yet.\n" +
      "help: use --overflow unchecked, or use emit-c/build for checked C output."
  );
}

function unsupportedCheckedLlvmError(): Error {
  return new Error(
    "error: LLVM backend does not support --overflow checked yet.\n" +
      "Use --overflow unchecked, or use the C backend for checked arithmetic."
  );
}

function parseOverflowMode(parsed: FlagParseResult): OverflowMode {
  const value = parsed.flags.get("--overflow") ?? defaultOverflowMode;
  if (value === "unchecked" || value === "checked") {
    return value;
  }

  throw new Error(`Invalid value for --overflow: ${value}. Expected 'unchecked' or 'checked'.`);
}

function parseMirDebugFlags(parsed: FlagParseResult): MirPassDebugFlags {
  return {
    printPassPipeline: parsed.flags.has("--print-pass-pipeline"),
    printMirBeforeOpt: parsed.flags.has("--print-mir-before-opt"),
    printMirAfterOpt: parsed.flags.has("--print-mir-after-opt")
  };
}

function parseOptLevel(parsed: FlagParseResult): OptimizationLevel {
  const value = parsed.flags.get("--opt-level");
  if (value === undefined) {
    return defaultOptimizationLevel;
  }

  const optLevel = parseOptimizationLevel(value);
  if (optLevel !== undefined) {
    return optLevel;
  }

  throw new Error(`Invalid optimization level: ${value}. Expected 0, 1, 2, or 3.`);
}

interface LowerAndOptimizeMirOptions {
  optLevel: OptimizationLevel;
  overflowMode: MirPassOverflowMode;
  targetBackend: MirPassTargetBackend;
  debug: MirPassDebugFlags;
  stderr: (text: string) => void;
}

type LowerAndOptimizeMirResult =
  | { ok: true; module: MirModule }
  | { ok: false; validationErrors: MirValidationError[] };

function lowerAndOptimizeMir(checkedProgram: CheckedProgram, options: LowerAndOptimizeMirOptions): LowerAndOptimizeMirResult {
  const mir = lowerToMir(checkedProgram);
  const pipeline = buildMirOptimizationPipeline(options.optLevel);

  if (options.debug.printPassPipeline) {
    options.stderr(`MIR pass pipeline: ${printMirPassPipeline(pipeline)}\n`);
  }

  if (options.debug.printMirBeforeOpt) {
    options.stderr(`MIR before optimization:\n${printMirModule(mir)}`);
  }

  const result = runMirPassPipeline(mir, pipeline, {
    optLevel: options.optLevel,
    overflowMode: options.overflowMode,
    targetBackend: options.targetBackend,
    debug: options.debug
  });

  if (options.debug.printMirAfterOpt) {
    options.stderr(`MIR after optimization:\n${printMirModule(result.module)}`);
  }

  if (result.validationErrors.length > 0) {
    return { ok: false, validationErrors: result.validationErrors };
  }

  return { ok: true, module: result.module };
}

function printInternalMirValidationErrors(stderr: (text: string) => void, validationErrors: MirValidationError[]): void {
  stderr("internal compiler error: MIR validation failed\n");
  for (const error of validationErrors) {
    const location = [error.functionName, error.blockLabel].filter(Boolean).join(":");
    stderr(`  - ${location ? `${location}: ` : ""}${error.message}\n`);
  }
}

function parseLlvmBuildKind(parsed: FlagParseResult): LlvmBuildKind {
  const value = parsed.flags.get("--kind") ?? "dynamic";
  if (value === "dynamic" || value === "object") {
    return value;
  }

  throw new Error(`Invalid value for --kind: ${value}. Expected 'dynamic' or 'object'.`);
}

function defaultCommandRunner(command: string, args: string[]): CommandResult {
  const result = spawnSync(command, args, { encoding: "utf8" });
  return {
    status: result.status,
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
    error: result.error as NodeJS.ErrnoException | undefined
  };
}

function cliContext(options: RunCliOptions): Required<RunCliOptions> {
  return {
    cwd: options.cwd ?? process.cwd(),
    platform: options.platform ?? process.platform,
    stdout: options.stdout ?? ((text) => process.stdout.write(text)),
    stderr: options.stderr ?? ((text) => process.stderr.write(text)),
    runCommand: options.runCommand ?? defaultCommandRunner
  };
}

function usage(): string {
  return [
    "Usage:",
    "  ikc check <file>",
    "  ikc emit-c <file> --out <c-file> --header <h-file> [--overflow <unchecked|checked>] [--opt-level <0|1|2|3>]",
    "  ikc emit-mir <file> [--out <mir-file>] [--opt-level <0|1|2|3>]",
    "  ikc emit-llvm <file> [--out <ll-file>] [--target <triple>] [--overflow unchecked] [--opt-level <0|1|2|3>]",
    "  ikc emit-wat <file> [--out <wat-file>] [--overflow unchecked] [--opt-level <0|1|2|3>]",
    "  ikc emit-wasm <file> --out <wasm-file> [--overflow unchecked] [--opt-level <0|1|2|3>]",
    "  ikc build <file> --out <output-path> [--overflow <unchecked|checked>] [--opt-level <0|1|2|3>]",
    "  ikc build-llvm <file> --out <output-path> [--kind <dynamic|object>] [--target <triple>] [--overflow unchecked] [--opt-level <0|1|2|3>]",
    "",
    "Options:",
    "  --overflow <unchecked|checked>    Arithmetic overflow handling mode. Default: unchecked.",
    "  --opt-level <0|1|2|3>            MIR optimization level. Default: 0.",
    "  -O0, -O1, -O2, -O3              Alias for --opt-level.",
    "  --print-pass-pipeline           Print the selected MIR pass pipeline to stderr.",
    "  --print-mir-before-opt          Print MIR before optimization to stderr.",
    "  --print-mir-after-opt           Print MIR after optimization to stderr.",
    ""
  ].join("\n");
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
const modulePath = fileURLToPath(import.meta.url);

if (invokedPath === modulePath) {
  process.exit(runCli());
}
