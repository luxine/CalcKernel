import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

interface WasmMemoryLike {
  buffer: ArrayBuffer;
}

interface WasmInstanceLike {
  exports: Record<string, unknown>;
}

interface WasmRuntime {
  instantiate(bytes: Uint8Array): Promise<{ instance: WasmInstanceLike }>;
}

const itemSize = 32;

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-wasm-"));
}

function getWasmRuntime(): WasmRuntime | undefined {
  return (globalThis as { WebAssembly?: WasmRuntime }).WebAssembly;
}

function writeItem(view: DataView, offset: number, fields: { price: bigint; qty: bigint; discount: bigint; taxRatePpm: bigint }): void {
  view.setBigInt64(offset + 0, fields.price, true);
  view.setBigInt64(offset + 8, fields.qty, true);
  view.setBigInt64(offset + 16, fields.discount, true);
  view.setBigInt64(offset + 24, fields.taxRatePpm, true);
}

function calcExpected(fields: { price: bigint; qty: bigint; discount: bigint; taxRatePpm: bigint }): bigint {
  const subtotal = fields.price * fields.qty;
  const afterDiscount = subtotal - fields.discount;
  const tax = (afterDiscount * fields.taxRatePpm) / 1_000_000n;
  return afterDiscount + tax;
}

describe("Node.js pricing WASM e2e", () => {
  const wasmAvailable = getWasmRuntime() !== undefined && typeof BigInt === "function";

  it.skipIf(!wasmAvailable)(
    wasmAvailable
      ? "runs examples/pricing.ik through generated WASM"
      : "runs examples/pricing.ik through generated WASM (skipped because Node.js WebAssembly BigInt support is unavailable)",
    async () => {
      const wasm = getWasmRuntime();
      expect(wasm).toBeDefined();
      const cwd = tempDir();
      writeFileSync(join(cwd, "pricing.ik"), readFileSync("examples/pricing.ik", "utf8"));
      const watFile = join(cwd, "build/pricing.wat");
      const wasmFile = join(cwd, "build/pricing.wasm");

      let stdout = "";
      let stderr = "";
      const emitWatExitCode = runCli(["emit-wat", "pricing.ik", "--out", "build/pricing.wat"], {
        cwd,
        stdout: (text) => {
          stdout += text;
        },
        stderr: (text) => {
          stderr += text;
        }
      });

      expect(emitWatExitCode).toBe(0);
      expect(stderr).toBe("");
      expect(stdout).toBe("OK: emitted WAT build/pricing.wat\n");
      expect(existsSync(watFile)).toBe(true);

      stdout = "";
      stderr = "";
      const emitWasmExitCode = runCli(["emit-wasm", "pricing.ik", "--out", "build/pricing.wasm"], {
        cwd,
        stdout: (text) => {
          stdout += text;
        },
        stderr: (text) => {
          stderr += text;
        }
      });

      expect(emitWasmExitCode).toBe(0);
      expect(stderr).toBe("");
      expect(stdout).toBe("OK: emitted WASM build/pricing.wasm\n");

      const bytes = readFileSync(wasmFile);
      const { instance } = await wasm!.instantiate(bytes);
      const memory = instance.exports.memory as WasmMemoryLike;
      const view = new DataView(memory.buffer);
      const calcItems = instance.exports.calc_items as (items: number, len: number, out: number) => number;
      const itemsOffset = 0;
      const outOffset = 4096;
      const item0 = { price: 1000n, qty: 3n, discount: 250n, taxRatePpm: 100000n };
      const item1 = { price: 500n, qty: 10n, discount: 1000n, taxRatePpm: 250000n };

      writeItem(view, itemsOffset, item0);
      writeItem(view, itemsOffset + itemSize, item1);

      expect(calcItems(itemsOffset, 2, outOffset)).toBe(0);
      expect(view.getBigInt64(outOffset + 0, true)).toBe(calcExpected(item0));
      expect(view.getBigInt64(outOffset + 8, true)).toBe(calcExpected(item1));
    }
  );
});
