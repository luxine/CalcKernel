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

describe("Node.js control-flow WASM e2e", () => {
  const wasmAvailable = getWasmRuntime() !== undefined && typeof BigInt === "function";

  it.skipIf(!wasmAvailable)(
    wasmAvailable
      ? "loads generated control-flow WASM and calls exported functions"
      : "loads generated control-flow WASM and calls exported functions (skipped because Node.js WebAssembly BigInt support is unavailable)",
    async () => {
      const wasm = getWasmRuntime();
      expect(wasm).toBeDefined();
      const cwd = tempDir();
      writeFileSync(join(cwd, "wasm_control_flow.ik"), readFileSync("examples/wasm_control_flow.ik", "utf8"));
      const wasmFile = join(cwd, "build/wasm_control_flow.wasm");
      let stdout = "";
      let stderr = "";

      const exitCode = runCli(["emit-wasm", "wasm_control_flow.ik", "--out", "build/wasm_control_flow.wasm"], {
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
      expect(stdout).toBe("OK: emitted WASM build/wasm_control_flow.wasm\n");

      const bytes = readFileSync(wasmFile);
      const { instance } = await wasm!.instantiate(bytes);
      const maxI32 = instance.exports.max_i32 as (a: number, b: number) => number;
      const sumToN = instance.exports.sum_to_n as (n: bigint) => bigint;

      expect(maxI32(10, 3)).toBe(10);
      expect(maxI32(1, 3)).toBe(3);
      expect(sumToN(5n)).toBe(10n);
    }
  );
});
