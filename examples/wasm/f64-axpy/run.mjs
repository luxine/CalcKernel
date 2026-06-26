import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const exampleDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(exampleDir, "../../..");
const sourcePath = join(exampleDir, "axpy.ck");
const wasmPath = join(repoRoot, "build/examples/wasm/f64-axpy/axpy.wasm");

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
  const axpyF64 = instance.exports.axpy_f64;
  if (typeof axpyF64 !== "function") {
    throw new Error("generated WASM did not export axpy_f64");
  }

  const arena = createCKWasmArena(instance);
  const xInput = new Float64Array([1.0, 2.0, 3.0, 4.0]);
  const yInput = new Float64Array([0.5, -1.0, 10.0, 20.0]);
  const a = 2.0;

  arena.ensureBytes((xInput.length + yInput.length) * Float64Array.BYTES_PER_ELEMENT);
  const { ptr: xPtr, view: xView } = arena.copyInF64(xInput);
  const { ptr: yPtr, view: yView } = arena.copyInF64(yInput);

  const checksum = axpyF64(a, xPtr, yPtr, xView.length);
  const outputView = arena.viewF64(yPtr, yView.length);
  const expected = Array.from(xInput, (value, index) => a * value + yInput[index]);
  const expectedChecksum = expected.reduce((sum, value) => sum + value, 0.0);

  if (!close(checksum, expectedChecksum)) {
    throw new Error(`unexpected checksum: expected ${expectedChecksum}, got ${checksum}`);
  }
  for (let index = 0; index < expected.length; index += 1) {
    if (!close(outputView[index], expected[index])) {
      throw new Error(`unexpected output[${index}]: expected ${expected[index]}, got ${outputView[index]}`);
    }
  }

  const copyOutput = Array.from(arena.copyOutF64(yPtr, outputView.length));
  for (let index = 0; index < expected.length; index += 1) {
    if (!close(copyOutput[index], expected[index])) {
      throw new Error(`unexpected copied output[${index}]: expected ${expected[index]}, got ${copyOutput[index]}`);
    }
  }

  return {
    checksum,
    expectedChecksum,
    output: Array.from(outputView),
    expected,
    copyOutput,
    dataViewHotPath: false,
    outputOwnership: "wasm-memory-view"
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = await runExample();
  console.log(`OK f64-axpy checksum=${result.checksum} output=${result.output.join(",")}`);
}
