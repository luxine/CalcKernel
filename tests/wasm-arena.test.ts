import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";
import { CKWasmArena, type CKWasmMemory } from "../src/wasm/ck-wasm-arena.js";

interface WasmInstanceLike {
  exports: Record<string, unknown>;
}

interface WasmRuntime {
  Memory: new (descriptor: { initial: number; maximum?: number }) => CKWasmMemory;
  instantiate(bytes: Uint8Array): Promise<{ instance: WasmInstanceLike }>;
}

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "calckernel-wasm-arena-"));
}

function getWasmRuntime(): WasmRuntime | undefined {
  return (globalThis as { WebAssembly?: WasmRuntime }).WebAssembly;
}

function closeDouble(actual: number, expected: number): boolean {
  return Math.abs(actual - expected) < 0.0000001;
}

describe("CKWasmArena", () => {
  it("allocates typed buffers at the required alignment", () => {
    const wasm = getWasmRuntime();
    expect(wasm).toBeDefined();
    const arena = new CKWasmArena(new wasm!.Memory({ initial: 1 }), { heapBase: 3 });

    expect(arena.allocBytes(1, 1)).toBe(3);

    const i32Ptr = arena.allocI32(1);
    const u32Ptr = arena.allocU32(1);
    const f64Ptr = arena.allocF64(1);
    const i64Ptr = arena.allocI64(1);
    const u64Ptr = arena.allocU64(1);

    expect(i32Ptr % 4).toBe(0);
    expect(u32Ptr % 4).toBe(0);
    expect(f64Ptr % 8).toBe(0);
    expect(i64Ptr % 8).toBe(0);
    expect(u64Ptr % 8).toBe(0);
  });

  it("copies numeric input with TypedArray set operations", () => {
    const wasm = getWasmRuntime();
    expect(wasm).toBeDefined();
    const arena = new CKWasmArena(new wasm!.Memory({ initial: 1 }), { heapBase: 64 });

    const f64 = arena.copyInF64(new Float64Array([1.25, -2.5, 3.75]));
    expect(f64.ptr % 8).toBe(0);
    expect(Array.from(f64.view)).toEqual([1.25, -2.5, 3.75]);

    const i32 = arena.copyInI32(new Int32Array([-7, 0, 42]));
    expect(i32.ptr % 4).toBe(0);
    expect(Array.from(i32.view)).toEqual([-7, 0, 42]);

    const u32 = arena.copyInU32(new Uint32Array([0, 42, 0xffff_ffff]));
    expect(u32.ptr % 4).toBe(0);
    expect(Array.from(u32.view)).toEqual([0, 42, 0xffff_ffff]);
  });

  it("creates a JS-owned Float64Array copy for copyOutF64", () => {
    const wasm = getWasmRuntime();
    expect(wasm).toBeDefined();
    const arena = new CKWasmArena(new wasm!.Memory({ initial: 1 }), { heapBase: 128 });
    const ptr = arena.allocF64(2);
    const view = arena.viewF64(ptr, 2);
    view.set([10.5, 20.25]);

    const copy = arena.copyOutF64(ptr, 2);
    expect(Array.from(copy)).toEqual([10.5, 20.25]);

    view[0] = 99.0;
    expect(copy[0]).toBe(10.5);
    expect(copy.buffer).not.toBe(view.buffer);
  });

  it("refreshes views after memory.grow without reusing the old buffer", () => {
    const wasm = getWasmRuntime();
    expect(wasm).toBeDefined();
    const memory = new wasm!.Memory({ initial: 1 });
    const arena = new CKWasmArena(memory, { heapBase: 0 });
    const ptr = arena.allocF64(1);
    const before = arena.viewF64(ptr, 1);
    before[0] = 42.5;
    const beforeBuffer = before.buffer;

    arena.ensureBytes(70_000);
    arena.refreshViewsIfNeeded();

    const after = arena.viewF64(ptr, 1);
    expect(after.buffer).toBe(memory.buffer);
    expect(after.buffer).not.toBe(beforeBuffer);
    expect(after[0]).toBe(42.5);
  });

  it("reads Float64Array view writes from generated CK / CalcKernel WASM", async () => {
    const wasm = getWasmRuntime();
    if (!wasm) {
      console.warn("skipped because WebAssembly is unavailable");
      return;
    }

    const cwd = tempDir();
    writeFileSync(
      join(cwd, "arena_read.ck"),
      `
        export fn read_f64(values: ptr<f64>, i: i32) -> f64 {
          return values[i];
        }
      `
    );
    const wasmFile = join(cwd, "build/arena_read.wasm");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wasm", "arena_read.ck", "--out", "build/arena_read.wasm"], {
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
    expect(stdout).toBe("OK: emitted WASM build/arena_read.wasm\n");

    const bytes = readFileSync(wasmFile);
    const { instance } = await wasm.instantiate(bytes);
    const memory = instance.exports.memory as CKWasmMemory;
    const readF64 = instance.exports.read_f64 as (values: number, i: number) => number;
    const arena = new CKWasmArena(memory, { heapBase: 128 });
    const ptr = arena.allocF64(3);
    arena.viewF64(ptr, 3).set([1.5, 2.25, 3.75]);

    expect(closeDouble(readF64(ptr, 0), 1.5)).toBe(true);
    expect(closeDouble(readF64(ptr, 1), 2.25)).toBe(true);
    expect(closeDouble(readF64(ptr, 2), 3.75)).toBe(true);
  });

  it("uses __ck_heap_base or __heap_base from exports when available", () => {
    const wasm = getWasmRuntime();
    expect(wasm).toBeDefined();
    const memory = new wasm!.Memory({ initial: 1 });
    const arena = CKWasmArena.fromExports({
      memory,
      __heap_base: { value: 16 },
      __ck_heap_base: { value: 64 }
    });

    expect(arena.allocBytes(1, 1)).toBe(64);
  });

  it("requires an explicit heapBase before allocation when exports do not provide one", () => {
    const wasm = getWasmRuntime();
    expect(wasm).toBeDefined();
    const arena = new CKWasmArena(new wasm!.Memory({ initial: 1 }));

    expect(() => arena.allocBytes(1, 1)).toThrow(/heapBase/);
  });

  it("keeps optimized f64 sum and axpy interop correct for views, copies, and special values", async () => {
    const wasm = getWasmRuntime();
    if (!wasm) {
      console.warn("skipped because WebAssembly is unavailable");
      return;
    }

    const cwd = tempDir();
    writeFileSync(
      join(cwd, "arena_f64_kernels.ck"),
      `
        export fn sum_f64(x: ptr<f64>, len: i32) -> f64 {
          let i: i32 = 0;
          let checksum: f64 = 0.0;

          while i < len {
            checksum = checksum + x[i];
            i = i + 1;
          }

          return checksum;
        }

        export fn axpy_f64(a: f64, x: ptr<f64>, y: ptr<f64>, len: i32) -> f64 {
          let i: i32 = 0;
          let checksum: f64 = 0.0;

          while i < len {
            let value: f64 = a * x[i] + y[i];
            y[i] = value;
            checksum = checksum + value;
            i = i + 1;
          }

          return checksum;
        }
      `
    );
    const wasmFile = join(cwd, "build/arena_f64_kernels.wasm");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wasm", "arena_f64_kernels.ck", "--out", "build/arena_f64_kernels.wasm", "-O3"], {
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
    expect(stdout).toBe("OK: emitted WASM build/arena_f64_kernels.wasm\n");

    const bytes = readFileSync(wasmFile);
    const { instance } = await wasm.instantiate(bytes);
    const memory = instance.exports.memory as CKWasmMemory;
    const sumF64 = instance.exports.sum_f64 as (x: number, len: number) => number;
    const axpyF64 = instance.exports.axpy_f64 as (a: number, x: number, y: number, len: number) => number;
    const arena = new CKWasmArena(memory, { heapBase: 128 });

    const xPtr = arena.allocF64(4);
    const yPtr = arena.allocF64(4);
    const xView = arena.viewF64(xPtr, 4);
    const yView = arena.viewF64(yPtr, 4);

    xView.set([1.25, -2.5, 3.75, 4.5]);
    const jsSum = Array.from(xView).reduce((sum, value) => sum + value, 0.0);
    expect(closeDouble(sumF64(xPtr, xView.length), jsSum)).toBe(true);

    xView.set([Number.POSITIVE_INFINITY, 1.0, 2.0, 3.0]);
    expect(sumF64(xPtr, xView.length)).toBe(Number.POSITIVE_INFINITY);

    xView.set([Number.NaN, 1.0, 2.0, 3.0]);
    expect(Number.isNaN(sumF64(xPtr, xView.length))).toBe(true);

    xView.set([1.0, 2.0, 3.0, 4.0]);
    yView.set([0.5, -1.0, 10.0, 20.0]);
    const checksum = axpyF64(2.0, xPtr, yPtr, xView.length);
    expect(closeDouble(checksum, 2.5 + 3.0 + 16.0 + 28.0)).toBe(true);
    expect(Array.from(yView)).toEqual([2.5, 3.0, 16.0, 28.0]);
    expect(yView.buffer).toBe(memory.buffer);

    const copy = arena.copyOutF64(yPtr, yView.length);
    expect(copy.buffer).not.toBe(memory.buffer);
    expect(Array.from(copy)).toEqual([2.5, 3.0, 16.0, 28.0]);

    yView[0] = 99.0;
    expect(copy[0]).toBe(2.5);

    xView.set([-0.0, Number.POSITIVE_INFINITY, Number.NaN, 1.0]);
    yView.set([-0.0, 1.0, 2.0, Number.NEGATIVE_INFINITY]);
    axpyF64(1.0, xPtr, yPtr, xView.length);
    expect(Object.is(yView[0], -0.0)).toBe(true);
    expect(yView[1]).toBe(Number.POSITIVE_INFINITY);
    expect(Number.isNaN(yView[2])).toBe(true);
    expect(yView[3]).toBe(Number.NEGATIVE_INFINITY);
  });
});
