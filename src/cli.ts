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
import { printMirModule } from "./mir/mir-printer.js";
import { validateMirModule } from "./mir/mir-validator.js";
import { formatDiagnostics } from "./source/diagnostics.js";
import { SourceFile } from "./source/source-file.js";
import { check, type CheckResult } from "./typeck/checker.js";

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
    overflowMode
  });
  context.stdout(`OK: emitted C with overflow=${overflowMode}\nWrote ${cPath}\nWrote ${headerPath}\n`);
  return 0;
}

function runEmitMir(args: string[], context: Required<RunCliOptions>): number {
  const parsed = parseFlags(args);
  const file = requireSingleInput(parsed, "emit-mir");
  const outFile = parsed.flags.get("--out");
  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const mir = lowerToMir(checked.result.checkedProgram);
  const validation = validateMirModule(mir);
  if (validation.errors.length > 0) {
    context.stderr("internal compiler error: MIR validation failed\n");
    for (const error of validation.errors) {
      const location = [error.functionName, error.blockLabel].filter(Boolean).join(":");
      context.stderr(`  - ${location ? `${location}: ` : ""}${error.message}\n`);
    }
    return 1;
  }

  const text = printMirModule(mir);

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
  const targetTriple = parsed.flags.get("--target");

  if (overflowMode === "checked") {
    throw unsupportedCheckedLlvmError();
  }

  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const mir = lowerToMir(checked.result.checkedProgram);
  const validation = validateMirModule(mir);
  if (validation.errors.length > 0) {
    context.stderr("internal compiler error: MIR validation failed\n");
    for (const error of validation.errors) {
      const location = [error.functionName, error.blockLabel].filter(Boolean).join(":");
      context.stderr(`  - ${location ? `${location}: ` : ""}${error.message}\n`);
    }
    return 1;
  }

  const text = emitMirLlvmModule(mir, { sourceFileName: file, targetTriple: targetTriple ?? detectNativeLlvmTargetTriple() });

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

  if (overflowMode === "checked") {
    throw unsupportedCheckedWasmError();
  }

  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const mir = lowerToMir(checked.result.checkedProgram);
  const validation = validateMirModule(mir);
  if (validation.errors.length > 0) {
    context.stderr("internal compiler error: MIR validation failed\n");
    for (const error of validation.errors) {
      const location = [error.functionName, error.blockLabel].filter(Boolean).join(":");
      context.stderr(`  - ${location ? `${location}: ` : ""}${error.message}\n`);
    }
    return 1;
  }

  const text = emitMirWatModule(mir);

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

  if (overflowMode === "checked") {
    throw unsupportedCheckedWasmError();
  }

  const checked = checkFile(file, context.cwd);

  if (checked.result.diagnostics.length > 0) {
    context.stderr(formatDiagnostics(checked.source, checked.result.diagnostics));
    return 1;
  }

  const mir = lowerToMir(checked.result.checkedProgram);
  const validation = validateMirModule(mir);
  if (validation.errors.length > 0) {
    context.stderr("internal compiler error: MIR validation failed\n");
    for (const error of validation.errors) {
      const location = [error.functionName, error.blockLabel].filter(Boolean).join(":");
      context.stderr(`  - ${location ? `${location}: ` : ""}${error.message}\n`);
    }
    return 1;
  }

  const wat = emitMirWatModule(mir);
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
    overflowMode
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
    if (!arg.startsWith("--")) {
      positional.push(arg);
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
    "  ikc emit-c <file> --out <c-file> --header <h-file> [--overflow <unchecked|checked>]",
    "  ikc emit-mir <file> [--out <mir-file>]",
    "  ikc emit-llvm <file> [--out <ll-file>] [--target <triple>] [--overflow unchecked]",
    "  ikc emit-wat <file> [--out <wat-file>] [--overflow unchecked]",
    "  ikc emit-wasm <file> --out <wasm-file> [--overflow unchecked]",
    "  ikc build <file> --out <output-path> [--overflow <unchecked|checked>]",
    "  ikc build-llvm <file> --out <output-path> [--kind <dynamic|object>] [--target <triple>] [--overflow unchecked]",
    "",
    "Options:",
    "  --overflow <unchecked|checked>    Arithmetic overflow handling mode. Default: unchecked.",
    ""
  ].join("\n");
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
const modulePath = fileURLToPath(import.meta.url);

if (invokedPath === modulePath) {
  process.exit(runCli());
}
