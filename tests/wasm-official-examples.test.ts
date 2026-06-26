import { describe, expect, it } from "vitest";

function closeDouble(actual: number, expected: number): boolean {
  return Math.abs(actual - expected) < 0.0000001;
}

describe("official WASM interop examples", () => {
  it("runs examples/wasm/f64-sum with scalar return and no output readback", async () => {
    const exampleUrl = new URL("../examples/wasm/f64-sum/run.mjs", import.meta.url).href;
    const example = (await import(exampleUrl)) as {
      runExample(): Promise<{
        dataViewHotPath: boolean;
        outputOwnership: string;
        result: number;
        expected: number;
      }>;
    };

    const result = await example.runExample();

    expect(result.dataViewHotPath).toBe(false);
    expect(result.outputOwnership).toBe("scalar-return");
    expect(closeDouble(result.result, result.expected)).toBe(true);
  });

  it("runs examples/wasm/f64-axpy with output view as the default path", async () => {
    const exampleUrl = new URL("../examples/wasm/f64-axpy/run.mjs", import.meta.url).href;
    const example = (await import(exampleUrl)) as {
      runExample(): Promise<{
        dataViewHotPath: boolean;
        outputOwnership: string;
        output: number[];
        expected: number[];
        copyOutput: number[];
      }>;
    };

    const result = await example.runExample();

    expect(result.dataViewHotPath).toBe(false);
    expect(result.outputOwnership).toBe("wasm-memory-view");
    expect(result.output.every((value, index) => closeDouble(value, result.expected[index]!))).toBe(true);
    expect(result.copyOutput.every((value, index) => closeDouble(value, result.expected[index]!))).toBe(true);
  });

  it("runs examples/wasm/pricing-soa with fixed-point resident memory views", async () => {
    const exampleUrl = new URL("../examples/wasm/pricing-soa/run.mjs", import.meta.url).href;
    const example = (await import(exampleUrl)) as {
      runExample(): Promise<{
        dataViewHotPath: boolean;
        outputOwnership: string;
        actual: bigint[];
        expected: bigint[];
      }>;
    };

    const result = await example.runExample();

    expect(result.dataViewHotPath).toBe(false);
    expect(result.outputOwnership).toBe("wasm-memory-view");
    expect(result.actual).toEqual(result.expected);
  });
});
