import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

interface WasmInstanceLike {
  exports: Record<string, unknown>;
}

interface WasmRuntime {
  instantiate(bytes: Uint8Array): Promise<{ instance: WasmInstanceLike }>;
}

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-wasm-"));
}

function getWasmRuntime(): WasmRuntime | undefined {
  return (globalThis as { WebAssembly?: WasmRuntime }).WebAssembly;
}

describe("Node.js function-call WASM e2e", () => {
  const wasmAvailable = getWasmRuntime() !== undefined && typeof BigInt === "function";

  it.skipIf(!wasmAvailable)(
    wasmAvailable
      ? "loads generated function-call WASM and calls exported functions"
      : "loads generated function-call WASM and calls exported functions (skipped because Node.js WebAssembly BigInt support is unavailable)",
    async () => {
      const wasm = getWasmRuntime();
      expect(wasm).toBeDefined();
      const cwd = tempDir();
      writeFileSync(join(cwd, "wasm_calls.ik"), readFileSync("examples/wasm_calls.ik", "utf8"));
      const wasmFile = join(cwd, "build/wasm_calls.wasm");
      let stdout = "";
      let stderr = "";

      const exitCode = runCli(["emit-wasm", "wasm_calls.ik", "--out", "build/wasm_calls.wasm"], {
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
      expect(stdout).toBe("OK: emitted WASM build/wasm_calls.wasm\n");

      const bytes = readFileSync(wasmFile);
      const { instance } = await wasm!.instantiate(bytes);
      const calc = instance.exports.calc as (a: bigint, b: bigint) => bigint;

      expect(instance.exports.calc).toBeTypeOf("function");
      expect(instance.exports.add_i64).toBeUndefined();
      expect(instance.exports.double_i64).toBeUndefined();
      expect(calc(1n, 2n)).toBe(6n);
    }
  );
});
