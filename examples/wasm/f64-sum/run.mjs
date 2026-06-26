import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const exampleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(exampleDir, "../../..");
const sourcePath = join(exampleDir, "sum.ck");
const wasmPath = join(repoRoot, "build/examples/wasm/f64-sum/sum.wasm");

function close(actual, expected) {
  return Math.abs(actual - expected) < 0.0000001;
}

function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`);
  }
  return result;
}

async function loadCalcKernel() {
  const distIndex = join(repoRoot, "dist/src/index.js");
  if (!existsSync(distIndex)) {
    throw new Error("Build CK / CalcKernel first with `pnpm build`.");
  }
  return import(pathToFileURL(distIndex).href);
}

function emitWasm() {
  mkdirSync(dirname(wasmPath), { recursive: true });
  run(process.execPath, [join(repoRoot, "dist/src/cli.js"), "emit-wasm", sourcePath, "--out", wasmPath, "-O3"], repoRoot);
}

export async function runExample() {
  emitWasm();
  const { createCKWasmArena } = await loadCalcKernel();
  const bytes = readFileSync(wasmPath);
  const { instance } = await WebAssembly.instantiate(bytes);
  const sumF64 = instance.exports.sum_f64;
  if (typeof sumF64 !== "function") {
    throw new Error("generated WASM did not export sum_f64");
  }

  const arena = createCKWasmArena(instance);
  const input = new Float64Array([1.25, -2.5, 3.75, 4.5, 10.0]);
  const { ptr, view } = arena.copyInF64(input);
  const result = sumF64(ptr, view.length);
  const expected = Array.from(input).reduce((sum, value) => sum + value, 0.0);

  if (!close(result, expected)) {
    throw new Error(`unexpected sum: expected ${expected}, got ${result}`);
  }

  return {
    inputLength: input.length,
    result,
    expected,
    dataViewHotPath: false,
    outputOwnership: "scalar-return"
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = await runExample();
  console.log(`OK f64-sum result=${result.result} inputLength=${result.inputLength}`);
}
