import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

interface WasmInstanceLike {
  exports: Record<string, unknown>;
}

interface WasmRuntime {
  Module: new (bytes: Uint8Array) => unknown;
  Instance: new (module: unknown) => WasmInstanceLike;
  instantiate(bytes: Uint8Array): Promise<{ instance: WasmInstanceLike }>;
}

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-wasm-"));
}

function getWasmRuntime(): WasmRuntime | undefined {
  return (globalThis as { WebAssembly?: WasmRuntime }).WebAssembly;
}

function supportsWasmI64BigInt(): boolean {
  const wasm = getWasmRuntime();
  if (!wasm || typeof BigInt !== "function") {
    return false;
  }

  try {
    const bytes = Uint8Array.from([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x06, 0x01, 0x60, 0x01, 0x7e, 0x01, 0x7e,
      0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x01, 0x02, 0x69, 0x64, 0x00, 0x00, 0x0a, 0x06, 0x01, 0x04,
      0x00, 0x20, 0x00, 0x0b
    ]);
    const module = new wasm.Module(bytes);
    const instance = new wasm.Instance(module);
    const id = instance.exports.id as (value: bigint) => bigint;
    return id(1n) === 1n;
  } catch {
    return false;
  }
}

function closeDouble(actual: number, expected: number): boolean {
  return Math.abs(actual - expected) < 0.0000001;
}

describe("Node.js scalar WASM e2e", () => {
  const wasmI64BigIntAvailable = supportsWasmI64BigInt();

  it.skipIf(!wasmI64BigIntAvailable)(
    wasmI64BigIntAvailable
      ? "loads generated scalar WASM and calls exported functions"
      : "loads generated scalar WASM and calls exported functions (skipped because Node.js i64 BigInt WebAssembly interop is unavailable)",
    async () => {
      const cwd = tempDir();
      writeFileSync(join(cwd, "wasm_scalar.ik"), readFileSync("examples/wasm_scalar.ik", "utf8"));
      const wasmFile = join(cwd, "build/wasm_scalar.wasm");
      let stdout = "";
      let stderr = "";

      const exitCode = runCli(["emit-wasm", "wasm_scalar.ik", "--out", "build/wasm_scalar.wasm"], {
        cwd,
        stdout: (text) => {
          stdout += text;
        },
        stderr: (text) => {
          stderr += text;
        }
      });

      expect(exitCode).toBe(0);
      expect(stderr).toBe("");
      expect(stdout).toBe("OK: emitted WASM build/wasm_scalar.wasm\n");

      const wasm = getWasmRuntime();
      expect(wasm).toBeDefined();
      const bytes = readFileSync(wasmFile);
      const { instance } = await wasm!.instantiate(bytes);
      const addI32 = instance.exports.add_i32 as (a: number, b: number) => number;
      const addI64 = instance.exports.add_i64 as (a: bigint, b: bigint) => bigint;
      const lessI64 = instance.exports.less_i64 as (a: bigint, b: bigint) => number;
      const divU64 = instance.exports.div_u64 as (a: bigint, b: bigint) => bigint;

      expect(addI32(1, 2)).toBe(3);
      expect(addI64(1n, 2n)).toBe(3n);
      expect(lessI64(1n, 2n)).toBe(1);
      expect(divU64(10n, 2n)).toBe(5n);
    }
  );

  it("loads generated f64 scalar WASM and calls exported functions with JS Number values", async () => {
    const wasm = getWasmRuntime();
    if (!wasm) {
      console.warn("skipped because WebAssembly is unavailable");
      return;
    }

    const cwd = tempDir();
    writeFileSync(
      join(cwd, "wasm_f64.ik"),
      `
        export fn calc_f64(a: f64, b: f64) -> f64 {
          let one: f64 = 1.0;
          let sum: f64 = a + b;
          let diff: f64 = sum - one;
          let prod: f64 = diff * b;
          return prod / 2.0;
        }

        export fn neg_f64(a: f64) -> f64 {
          return -a;
        }

        export fn le_f64(a: f64, b: f64) -> bool {
          return a <= b;
        }
      `
    );
    const wasmFile = join(cwd, "build/wasm_f64.wasm");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wasm", "wasm_f64.ik", "--out", "build/wasm_f64.wasm"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(0);
    expect(stderr).toBe("");
    expect(stdout).toBe("OK: emitted WASM build/wasm_f64.wasm\n");

    const bytes = readFileSync(wasmFile);
    const { instance } = await wasm.instantiate(bytes);
    const calcF64 = instance.exports.calc_f64 as (a: number, b: number) => number;
    const negF64 = instance.exports.neg_f64 as (a: number) => number;
    const leF64 = instance.exports.le_f64 as (a: number, b: number) => number;

    const calcResult = calcF64(5.0, 3.0);
    const negResult = negF64(7.25);

    expect(typeof calcResult).toBe("number");
    expect(typeof negResult).toBe("number");
    expect(closeDouble(calcResult, 10.5)).toBe(true);
    expect(closeDouble(negResult, -7.25)).toBe(true);
    expect(leF64(3.5, 3.5)).toBe(1);
    expect(leF64(4.5, 3.5)).toBe(0);
  });
});
