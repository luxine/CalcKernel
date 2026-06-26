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
  return mkdtempSync(join(tmpdir(), "calckernel-wasm-"));
}

function getWasmRuntime(): WasmRuntime | undefined {
  return (globalThis as { WebAssembly?: WasmRuntime }).WebAssembly;
}

describe("Node.js short-circuit WASM e2e", () => {
  const wasmAvailable = getWasmRuntime() !== undefined && typeof BigInt === "function";

  it.skipIf(!wasmAvailable)(
    wasmAvailable
      ? "preserves logical short-circuit behavior in generated WASM"
      : "preserves logical short-circuit behavior in generated WASM (skipped because Node.js WebAssembly BigInt support is unavailable)",
    async () => {
      const wasm = getWasmRuntime();
      expect(wasm).toBeDefined();
      const cwd = tempDir();
      writeFileSync(join(cwd, "wasm_short_circuit.ck"), readFileSync("examples/wasm_short_circuit.ck", "utf8"));
      const wasmFile = join(cwd, "build/wasm_short_circuit.wasm");
      let stdout = "";
      let stderr = "";

      const exitCode = runCli(["emit-wasm", "wasm_short_circuit.ck", "--out", "build/wasm_short_circuit.wasm"], {
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
      expect(stdout).toBe("OK: emitted WASM build/wasm_short_circuit.wasm\n");

      const bytes = readFileSync(wasmFile);
      const { instance } = await wasm!.instantiate(bytes);
      const andShortCircuit = instance.exports.and_short_circuit as (a: bigint, b: bigint) => number;
      const orShortCircuit = instance.exports.or_short_circuit as (a: bigint, b: bigint) => number;

      expect(andShortCircuit(0n, 10n)).toBe(0);
      expect(andShortCircuit(2n, 10n)).toBe(1);
      expect(orShortCircuit(0n, 10n)).toBe(1);
      expect(orShortCircuit(2n, 10n)).toBe(1);
    }
  );
});
