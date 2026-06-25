import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
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
const priceOffset = 0;

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

function closeDouble(actual: number, expected: number): boolean {
  return Math.abs(actual - expected) < 0.0000001;
}

describe("Node.js memory WASM e2e", () => {
  const wasmAvailable = getWasmRuntime() !== undefined && typeof BigInt === "function";

  it.skipIf(!wasmAvailable)(
    wasmAvailable
      ? "loads and stores ptr/index/field values through exported memory"
      : "loads and stores ptr/index/field values through exported memory (skipped because Node.js WebAssembly BigInt support is unavailable)",
    async () => {
      const wasm = getWasmRuntime();
      expect(wasm).toBeDefined();
      const cwd = tempDir();
      writeFileSync(join(cwd, "wasm_memory.ik"), readFileSync("examples/wasm_memory.ik", "utf8"));
      const wasmFile = join(cwd, "build/wasm_memory.wasm");
      let stdout = "";
      let stderr = "";

      const exitCode = runCli(["emit-wasm", "wasm_memory.ik", "--out", "build/wasm_memory.wasm"], {
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
      expect(stdout).toBe("OK: emitted WASM build/wasm_memory.wasm\n");

      const bytes = readFileSync(wasmFile);
      const { instance } = await wasm!.instantiate(bytes);
      const memory = instance.exports.memory as WasmMemoryLike;
      const view = new DataView(memory.buffer);
      const firstPrice = instance.exports.first_price as (items: number) => bigint;
      const getPrice = instance.exports.get_price as (items: number, i: number) => bigint;
      const writeI64 = instance.exports.write_i64 as (out: number, value: bigint) => number;

      writeItem(view, 0, { price: 1234n, qty: 2n, discount: 3n, taxRatePpm: 4n });
      expect(firstPrice(0)).toBe(1234n);

      const base = 128;
      writeItem(view, base, { price: 11n, qty: 0n, discount: 0n, taxRatePpm: 0n });
      writeItem(view, base + itemSize, { price: 222n, qty: 0n, discount: 0n, taxRatePpm: 0n });
      expect(getPrice(base, 1)).toBe(222n);

      const outOffset = 512;
      expect(writeI64(outOffset, 123n)).toBe(0);
      expect(view.getBigInt64(outOffset + priceOffset, true)).toBe(123n);
    }
  );

  it("loads and stores ptr<f64> and struct f64 fields through exported memory", async () => {
    const wasm = getWasmRuntime();
    if (!wasm) {
      console.warn("skipped because WebAssembly is unavailable");
      return;
    }

    const cwd = tempDir();
    writeFileSync(
      join(cwd, "wasm_f64_memory.ik"),
      `
        struct Quote {
          price: f64;
          tax: f64;
        }

        export fn write_scale(values: ptr<f64>, i: i32, factor: f64) -> f64 {
          values[i] = values[i] * factor;
          return values[i];
        }

        export fn quote_total(quotes: ptr<Quote>, i: i32) -> f64 {
          return quotes[i].price + quotes[i].tax;
        }
      `
    );
    const wasmFile = join(cwd, "build/wasm_f64_memory.wasm");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wasm", "wasm_f64_memory.ik", "--out", "build/wasm_f64_memory.wasm"], {
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
    expect(stdout).toBe("OK: emitted WASM build/wasm_f64_memory.wasm\n");

    const bytes = readFileSync(wasmFile);
    const { instance } = await wasm.instantiate(bytes);
    const memory = instance.exports.memory as WasmMemoryLike;
    const view = new DataView(memory.buffer);
    const writeScale = instance.exports.write_scale as (values: number, i: number, factor: number) => number;
    const quoteTotal = instance.exports.quote_total as (quotes: number, i: number) => number;

    const valuesOffset = 128;
    view.setFloat64(valuesOffset + 0, 1.0, true);
    view.setFloat64(valuesOffset + 8, 2.5, true);
    view.setFloat64(valuesOffset + 16, 4.0, true);

    const scaled = writeScale(valuesOffset, 1, 4.0);
    expect(typeof scaled).toBe("number");
    expect(closeDouble(scaled, 10.0)).toBe(true);
    expect(closeDouble(view.getFloat64(valuesOffset + 8, true), 10.0)).toBe(true);

    const quotesOffset = 512;
    view.setFloat64(quotesOffset + 0, 10.25, true);
    view.setFloat64(quotesOffset + 8, 0.75, true);
    view.setFloat64(quotesOffset + 16, 20.5, true);
    view.setFloat64(quotesOffset + 24, 1.25, true);

    const total = quoteTotal(quotesOffset, 1);
    expect(typeof total).toBe("number");
    expect(closeDouble(total, 21.75)).toBe(true);
  });
});
