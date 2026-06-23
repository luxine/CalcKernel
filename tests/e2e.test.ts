import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { emitCSource } from "../src/backend/c/c-emitter.js";
import { emitCHeader } from "../src/backend/c/c-header-emitter.js";
import { emitMirCSource } from "../src/backend/c/mir-c-emitter.js";
import { runCli, type CommandRunner } from "../src/cli.js";
import { lowerToMir } from "../src/mir/lower.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];
const strictClangCppFlags = ["-std=c++17", "-O3", "-Wall", "-Wextra", "-Werror"];
const buildDllFlag = "-DIK_BUILD_DLL";

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-cli-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

function hasClangCpp(): boolean {
  return spawnSync("clang++", ["--version"], { encoding: "utf8" }).status === 0;
}

function emitPricingExample(cwd: string): { cFile: string; headerFile: string } {
  writeFileSync(join(cwd, "pricing.ik"), readFileSync("examples/pricing.ik", "utf8"));
  const cFile = join(cwd, "build/pricing.c");
  const headerFile = join(cwd, "build/pricing.h");
  const exitCode = runCli(["emit-c", "pricing.ik", "--out", cFile, "--header", headerFile], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });

  expect(exitCode).toBe(0);
  return { cFile, headerFile };
}

function emitPricingCheckedExample(cwd: string): { cFile: string; headerFile: string } {
  writeFileSync(join(cwd, "pricing.ik"), readFileSync("examples/pricing.ik", "utf8"));
  const cFile = join(cwd, "build/pricing_checked.c");
  const headerFile = join(cwd, "build/pricing_checked.h");
  const exitCode = runCli(["emit-c", "pricing.ik", "--out", cFile, "--header", headerFile, "--overflow", "checked"], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });

  expect(exitCode).toBe(0);
  return { cFile, headerFile };
}

function emitScalarCheckedExample(cwd: string): { cFile: string; headerFile: string } {
  writeFileSync(join(cwd, "scalar_checked.ik"), readFileSync("examples/scalar_checked.ik", "utf8"));
  const cFile = join(cwd, "build/scalar_checked.c");
  const headerFile = join(cwd, "build/scalar_checked.h");
  const exitCode = runCli(["emit-c", "scalar_checked.ik", "--out", cFile, "--header", headerFile, "--overflow", "checked"], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });

  expect(exitCode).toBe(0);
  return { cFile, headerFile };
}

function emitScalarControlCheckedExample(cwd: string): { cFile: string; headerFile: string } {
  writeFileSync(join(cwd, "scalar_control_checked.ik"), readFileSync("examples/scalar_control_checked.ik", "utf8"));
  const cFile = join(cwd, "build/scalar_control_checked.c");
  const headerFile = join(cwd, "build/scalar_control_checked.h");
  const exitCode = runCli(["emit-c", "scalar_control_checked.ik", "--out", cFile, "--header", headerFile, "--overflow", "checked"], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });

  expect(exitCode).toBe(0);
  return { cFile, headerFile };
}

function emitScalarLogicalCheckedExample(cwd: string): { cFile: string; headerFile: string } {
  writeFileSync(join(cwd, "scalar_logical_checked.ik"), readFileSync("examples/scalar_logical_checked.ik", "utf8"));
  const cFile = join(cwd, "build/scalar_logical_checked.c");
  const headerFile = join(cwd, "build/scalar_logical_checked.h");
  const exitCode = runCli(["emit-c", "scalar_logical_checked.ik", "--out", cFile, "--header", headerFile, "--overflow", "checked"], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });

  expect(exitCode).toBe(0);
  return { cFile, headerFile };
}

function emitScalarCallsCheckedExample(cwd: string): { cFile: string; headerFile: string } {
  writeFileSync(join(cwd, "scalar_calls_checked.ik"), readFileSync("examples/scalar_calls_checked.ik", "utf8"));
  const cFile = join(cwd, "build/scalar_calls_checked.c");
  const headerFile = join(cwd, "build/scalar_calls_checked.h");
  const exitCode = runCli(["emit-c", "scalar_calls_checked.ik", "--out", cFile, "--header", headerFile, "--overflow", "checked"], {
    cwd,
    stdout: () => {},
    stderr: () => {}
  });

  expect(exitCode).toBe(0);
  return { cFile, headerFile };
}

function emitPricingCheckedHeader(cwd: string): string {
  const sourceText = readFileSync("examples/pricing.ik", "utf8");
  const checked = check(new SourceFile("pricing.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const headerFile = join(cwd, "pricing_checked.h");
  writeFileSync(headerFile, emitCHeader(checked, { overflowMode: "checked" }));
  return headerFile;
}

function emitMirScalarUncheckedExample(cwd: string): { cFile: string; headerFile: string } {
  const sourceText = `
    export fn add_i64(a: i64, b: i64) -> i64 {
      let x: i64 = a + b;
      return x;
    }

    export fn mul_i64(a: i64, b: i64) -> i64 {
      return a * b;
    }

    export fn less_i64(a: i64, b: i64) -> bool {
      return a < b;
    }

    export fn neg_i64(a: i64) -> i64 {
      return -a;
    }

    export fn not_bool(a: bool) -> bool {
      return !a;
    }
  `;
  const checked = check(new SourceFile("scalar_mir.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  const cFile = join(cwd, "build/scalar_mir.c");
  const headerFile = join(cwd, "build/scalar_mir.h");
  mkdirSync(join(cwd, "build"), { recursive: true });
  writeFileSync(headerFile, emitCHeader(checked));
  writeFileSync(cFile, emitMirCSource(mir, { headerFileName: "scalar_mir.h" }));
  return { cFile, headerFile };
}

function emitMirControlFlowUncheckedExample(cwd: string): { cFile: string; headerFile: string } {
  const sourceText = `
    export fn max_i32(a: i32, b: i32) -> i32 {
      if a > b {
        return a;
      } else {
        return b;
      }
    }

    export fn positive_or_zero(a: i32) -> i32 {
      if a > 0 {
        return a;
      }
      return 0;
    }

    export fn sum_to_n(n: i64) -> i64 {
      let i: i64 = 0;
      let sum: i64 = 0;

      while i < n {
        sum = sum + i;
        i = i + 1;
      }

      return sum;
    }
  `;
  const checked = check(new SourceFile("control_mir.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  const cFile = join(cwd, "build/control_mir.c");
  const headerFile = join(cwd, "build/control_mir.h");
  mkdirSync(join(cwd, "build"), { recursive: true });
  writeFileSync(headerFile, emitCHeader(checked));
  writeFileSync(cFile, emitMirCSource(mir, { headerFileName: "control_mir.h" }));
  return { cFile, headerFile };
}

function emitMirShortCircuitUncheckedExample(cwd: string): { cFile: string; headerFile: string } {
  const sourceText = `
    export fn and_short_circuit(a: i64, b: i64) -> bool {
      return a != 0 && b / a > 1;
    }

    export fn or_short_circuit(a: i64, b: i64) -> bool {
      return a == 0 || b / a > 1;
    }
  `;
  const checked = check(new SourceFile("short_circuit_mir.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  const cFile = join(cwd, "build/short_circuit_mir.c");
  const headerFile = join(cwd, "build/short_circuit_mir.h");
  mkdirSync(join(cwd, "build"), { recursive: true });
  writeFileSync(headerFile, emitCHeader(checked));
  writeFileSync(cFile, emitMirCSource(mir, { headerFileName: "short_circuit_mir.h" }));
  return { cFile, headerFile };
}

function emitMirCallsUncheckedExample(cwd: string): { cFile: string; headerFile: string } {
  const sourceText = `
    fn add_i64(a: i64, b: i64) -> i64 {
      return a + b;
    }

    fn double_i64(a: i64) -> i64 {
      return a * 2;
    }

    export fn calc(a: i64, b: i64) -> i64 {
      return double_i64(add_i64(a, b));
    }
  `;
  const checked = check(new SourceFile("calls_mir.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  const cFile = join(cwd, "build/calls_mir.c");
  const headerFile = join(cwd, "build/calls_mir.h");
  mkdirSync(join(cwd, "build"), { recursive: true });
  writeFileSync(headerFile, emitCHeader(checked));
  writeFileSync(cFile, emitMirCSource(mir, { headerFileName: "calls_mir.h" }));
  return { cFile, headerFile };
}

function emitMirPricingUncheckedExample(cwd: string): { cFile: string; headerFile: string } {
  const sourceText = readFileSync("examples/pricing.ik", "utf8");
  const checked = check(new SourceFile("pricing_mir.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  const cFile = join(cwd, "build/pricing_mir.c");
  const headerFile = join(cwd, "build/pricing_mir.h");
  mkdirSync(join(cwd, "build"), { recursive: true });
  writeFileSync(headerFile, emitCHeader(checked));
  writeFileSync(cFile, emitMirCSource(mir, { headerFileName: "pricing_mir.h" }));
  return { cFile, headerFile };
}

function emitMirCheckedExample(cwd: string, exampleName: string): { cFile: string; headerFile: string } {
  const sourceText = readFileSync(`examples/${exampleName}.ik`, "utf8");
  const checked = check(new SourceFile(`${exampleName}.ik`, sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  const cFile = join(cwd, `build/${exampleName}_mir_checked.c`);
  const headerFile = join(cwd, `build/${exampleName}_mir_checked.h`);
  mkdirSync(join(cwd, "build"), { recursive: true });
  writeFileSync(headerFile, emitCHeader(checked, { overflowMode: "checked" }));
  writeFileSync(cFile, emitMirCSource(mir, { headerFileName: `${exampleName}_mir_checked.h`, overflowMode: "checked" }));
  return { cFile, headerFile };
}

type PipelineOverflowMode = "unchecked" | "checked";

interface PipelineRegressionCase {
  name: string;
  sourceText: string;
  overflowMode: PipelineOverflowMode;
  harnessSource: string;
}

interface EmittedPipeline {
  cFile: string;
  headerText: string;
  includeDir: string;
}

function emitAstAndMirPipelines(
  cwd: string,
  fixtureName: string,
  sourceText: string,
  overflowMode: PipelineOverflowMode
): { ast: EmittedPipeline; mir: EmittedPipeline } {
  const checked = check(new SourceFile(`${fixtureName}.ik`, sourceText));
  expect(checked.diagnostics).toEqual([]);

  const headerText = emitCHeader(checked, { overflowMode });
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  const astDir = join(cwd, fixtureName, "ast");
  const mirDir = join(cwd, fixtureName, "mir");
  mkdirSync(astDir, { recursive: true });
  mkdirSync(mirDir, { recursive: true });

  const astHeaderFile = join(astDir, "kernel.h");
  const astCFile = join(astDir, "kernel.c");
  const mirHeaderFile = join(mirDir, "kernel.h");
  const mirCFile = join(mirDir, "kernel.c");

  writeFileSync(astHeaderFile, headerText);
  writeFileSync(astCFile, emitCSource(checked, { headerFileName: "kernel.h", overflowMode }));
  writeFileSync(mirHeaderFile, headerText);
  writeFileSync(mirCFile, emitMirCSource(mir, { headerFileName: "kernel.h", overflowMode }));

  return {
    ast: { cFile: astCFile, headerText, includeDir: astDir },
    mir: { cFile: mirCFile, headerText, includeDir: mirDir }
  };
}

function compileAndRunPipeline(cwd: string, name: string, pipeline: EmittedPipeline, harnessSource: string): string {
  const harnessFile = join(pipeline.includeDir, "harness.c");
  const executable = join(cwd, `${name}-harness`);
  writeFileSync(harnessFile, harnessSource);

  const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, pipeline.cFile, harnessFile, "-I", pipeline.includeDir, "-o", executable], {
    encoding: "utf8"
  });
  expect(compile.status, compile.stderr).toBe(0);

  const run = spawnSync(executable, [], { encoding: "utf8" });
  expect(run.status, run.stderr || run.stdout).toBe(0);
  return run.stdout;
}

describe("ikc CLI", () => {
  it("prints help", () => {
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["--help"], {
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(0);
    expect(stderr).toBe("");
    expect(stdout).toContain("ikc check <file>");
    expect(stdout).toContain("ikc emit-c <file> --out <c-file> --header <h-file>");
    expect(stdout).toContain("ikc emit-mir <file> [--out <mir-file>]");
    expect(stdout).toContain("ikc emit-wat <file> [--out <wat-file>] [--overflow unchecked]");
    expect(stdout).toContain("ikc emit-wasm <file> --out <wasm-file> [--overflow unchecked]");
    expect(stdout).toContain("ikc build <file> --out <output-path>");
    expect(stdout).toContain("--overflow <unchecked|checked>    Arithmetic overflow handling mode. Default: unchecked.");
  });

  it("checks valid files with a concise success message", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["check", "scalar.ik"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(0);
    expect(stdout).toBe("OK: scalar.ik\n");
    expect(stderr).toBe("");
  });

  it("checks files and prints diagnostics with code, line, column, and source snippet", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "bad.ik"), "export fn bad() -> i32 {\n  return missing;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["check", "bad.ik"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain("bad.ik:2:10: error IK2001: Unknown variable 'missing'.");
    expect(stderr).toContain("  return missing;");
    expect(stderr).toContain("         ^^^^^^^");
  });

  it("emits C and header files", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stderr = "";

    let stdout = "";

    const exitCode = runCli(["emit-c", "scalar.ik", "--out", "build/scalar.c", "--header", "build/scalar.h"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(0);
    expect(stdout).toBe(`OK: emitted C with overflow=unchecked\nWrote ${join(cwd, "build/scalar.c")}\nWrote ${join(cwd, "build/scalar.h")}\n`);
    expect(stderr).toBe("");
    expect(readFileSync(join(cwd, "build/scalar.h"), "utf8")).toContain("int64_t add(int64_t a, int64_t b);");
    expect(readFileSync(join(cwd, "build/scalar.c"), "utf8")).toContain('#include "scalar.h"');
  });

  it("treats omitted overflow mode as unchecked and accepts explicit unchecked", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let defaultStdout = "";
    let uncheckedStdout = "";

    const defaultExitCode = runCli(["emit-c", "scalar.ik", "--out", "build/default/scalar.c", "--header", "build/default/scalar.h"], {
      cwd,
      stdout: (text) => {
        defaultStdout += text;
      },
      stderr: () => {}
    });
    const uncheckedExitCode = runCli(
      ["emit-c", "scalar.ik", "--out", "build/unchecked/scalar.c", "--header", "build/unchecked/scalar.h", "--overflow", "unchecked"],
      {
        cwd,
        stdout: (text) => {
          uncheckedStdout += text;
        },
        stderr: () => {}
      }
    );

    expect(defaultExitCode).toBe(0);
    expect(uncheckedExitCode).toBe(0);
    expect(defaultStdout).toContain("OK: emitted C with overflow=unchecked");
    expect(uncheckedStdout).toContain("OK: emitted C with overflow=unchecked");
    expect(readFileSync(join(cwd, "build/unchecked/scalar.c"), "utf8")).toBe(readFileSync(join(cwd, "build/default/scalar.c"), "utf8"));
    expect(readFileSync(join(cwd, "build/unchecked/scalar.h"), "utf8")).toBe(readFileSync(join(cwd, "build/default/scalar.h"), "utf8"));
  });

  it("emits checked scalar C for supported scalar code", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";

    const exitCode = runCli(["emit-c", "scalar.ik", "--out", "build/checked.c", "--header", "build/checked.h", "--overflow", "checked"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      }
    });

    expect(exitCode).toBe(0);
    expect(stdout).toContain("OK: emitted C with overflow=checked");
    expect(stdout).toContain(`Wrote ${join(cwd, "build/checked.c")}`);
    expect(readFileSync(join(cwd, "build/checked.h"), "utf8")).toContain("IK_API IK_Status add(int64_t a, int64_t b, int64_t* ik_return);");
    expect(readFileSync(join(cwd, "build/checked.c"), "utf8")).toContain("__builtin_add_overflow");
  });

  it("rejects invalid overflow modes", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stderr = "";

    const exitCode = runCli(["emit-c", "scalar.ik", "--out", "build/scalar.c", "--header", "build/scalar.h", "--overflow", "safe"], {
      cwd,
      stdout: () => {},
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stderr).toContain("Invalid value for --overflow: safe. Expected 'unchecked' or 'checked'.");
    expect(existsSync(join(cwd, "build/scalar.c"))).toBe(false);
    expect(existsSync(join(cwd, "build/scalar.h"))).toBe(false);
  });

  it("does not write emit-c outputs for invalid files", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "bad.ik"), "export fn bad() -> i32 {\n  return missing;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-c", "bad.ik", "--out", "build/bad.c", "--header", "build/bad.h"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain("bad.ik:2:10: error IK2001: Unknown variable 'missing'.");
    expect(existsSync(join(cwd, "build/bad.c"))).toBe(false);
    expect(existsSync(join(cwd, "build/bad.h"))).toBe(false);
  });

  it("prints MIR to stdout when emit-mir has no --out", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-mir", "scalar.ik"], {
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
    expect(stdout).toContain("export fn add(a: i64, b: i64) -> i64 {");
    expect(stdout).toContain("%t0: i64 = add a, b");
    expect(stdout).toContain("return %t0");
  });

  it("emits MIR for pricing with stable struct and store output", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "pricing.ik"), readFileSync("examples/pricing.ik", "utf8"));
    let stdout = "";

    const exitCode = runCli(["emit-mir", "pricing.ik"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: () => {}
    });

    expect(exitCode).toBe(0);
    expect(stdout).toContain("struct Item {");
    expect(stdout).toContain("tax_rate_ppm: i64");
    expect(stdout).toContain("export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {");
    expect(stdout).toContain("store index(out, i),");
    expect(stdout).not.toContain(cwd);
  });

  it("writes MIR to --out when requested", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";
    const mirFile = join(cwd, "build/scalar.mir");

    const exitCode = runCli(["emit-mir", "scalar.ik", "--out", "build/scalar.mir"], {
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
    expect(stdout).toBe(`OK: emitted MIR\nWrote ${mirFile}\n`);
    expect(readFileSync(mirFile, "utf8")).toContain("%t0: i64 = add a, b");
  });

  it("does not output MIR for invalid source", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "bad.ik"), "export fn bad() -> i32 {\n  return missing;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-mir", "bad.ik", "--out", "build/bad.mir"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain("bad.ik:2:10: error IK2001: Unknown variable 'missing'.");
    expect(existsSync(join(cwd, "build/bad.mir"))).toBe(false);
  });

  it("emits WAT to --out for valid scalar source", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";
    const watFile = join(cwd, "build/scalar.wat");

    const exitCode = runCli(["emit-wat", "scalar.ik", "--out", "build/scalar.wat"], {
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
    expect(stdout).toBe("OK: emitted WAT build/scalar.wat\n");
    expect(readFileSync(watFile, "utf8")).toContain('(memory (export "memory") 1)');
    expect(readFileSync(watFile, "utf8")).toContain('(func $add (export "add")');
    expect(readFileSync(watFile, "utf8")).toContain("i64.add");
  });

  it("prints WAT to stdout when emit-wat has no --out", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wat", "scalar.ik"], {
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
    expect(stdout).toContain("(module\n");
    expect(stdout).toContain('(memory (export "memory") 1)');
    expect(stdout).toContain('(func $add (export "add")');
    expect(stdout).toContain("i64.add");
    expect(stdout).not.toContain(cwd);
  });

  it("does not output WAT for invalid source", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "bad.ik"), "export fn bad() -> i32 {\n  return missing;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wat", "bad.ik", "--out", "build/bad.wat"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain("bad.ik:2:10: error IK2001: Unknown variable 'missing'.");
    expect(existsSync(join(cwd, "build/bad.wat"))).toBe(false);
  });

  it("rejects checked overflow mode for emit-wat", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wat", "scalar.ik", "--out", "build/scalar.wat", "--overflow", "checked"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toBe(
      "error: WASM backend does not support --overflow checked yet.\n" +
        "help: use --overflow unchecked, or use emit-c/build for checked C output.\n"
    );
    expect(stderr).toContain("WASM");
    expect(stderr).toContain("checked");
    expect(existsSync(join(cwd, "build/scalar.wat"))).toBe(false);
  });

  it("rejects invalid overflow modes for emit-wat", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wat", "scalar.ik", "--overflow", "fast"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain("Invalid value for --overflow: fast. Expected 'unchecked' or 'checked'.");
  });

  it("emits WASM binary to --out for valid scalar source", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), readFileSync("examples/scalar.ik", "utf8"));
    let stdout = "";
    let stderr = "";
    const wasmFile = join(cwd, "build/scalar.wasm");

    const exitCode = runCli(["emit-wasm", "scalar.ik", "--out", "build/scalar.wasm"], {
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
    expect(stdout).toBe("OK: emitted WASM build/scalar.wasm\n");
    const bytes = readFileSync(wasmFile);
    expect(bytes.byteLength).toBeGreaterThan(8);
    expect([...bytes.subarray(0, 4)]).toEqual([0x00, 0x61, 0x73, 0x6d]);
  });

  it("requires --out for emit-wasm", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), readFileSync("examples/scalar.ik", "utf8"));
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wasm", "scalar.ik"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain("Usage error for 'emit-wasm': missing --out.");
  });

  it("creates output directories for emit-wasm", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), readFileSync("examples/scalar.ik", "utf8"));
    let stdout = "";
    let stderr = "";
    const wasmFile = join(cwd, "nested/build/scalar.wasm");

    const exitCode = runCli(["emit-wasm", "scalar.ik", "--out", "nested/build/scalar.wasm"], {
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
    expect(stdout).toBe("OK: emitted WASM nested/build/scalar.wasm\n");
    const bytes = readFileSync(wasmFile);
    expect([...bytes.subarray(0, 4)]).toEqual([0x00, 0x61, 0x73, 0x6d]);
  });

  it("does not output WASM for invalid source", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "bad.ik"), "export fn bad() -> i32 {\n  return missing;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wasm", "bad.ik", "--out", "build/bad.wasm"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain("bad.ik:2:10: error IK2001: Unknown variable 'missing'.");
    expect(existsSync(join(cwd, "build/bad.wasm"))).toBe(false);
  });

  it("rejects checked overflow mode for emit-wasm", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";

    const exitCode = runCli(["emit-wasm", "scalar.ik", "--out", "build/scalar.wasm", "--overflow", "checked"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toBe(
      "error: WASM backend does not support --overflow checked yet.\n" +
        "help: use --overflow unchecked, or use emit-c/build for checked C output.\n"
    );
    expect(stderr).toContain("WASM");
    expect(stderr).toContain("checked");
    expect(existsSync(join(cwd, "build/scalar.wasm"))).toBe(false);
  });

  it("build emits C first and invokes clang with strict Linux shared library flags", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    const calls: Array<{ command: string; args: string[] }> = [];
    const runner: CommandRunner = (command, args) => {
      calls.push({ command, args });
      return { status: 0, stdout: "", stderr: "" };
    };
    let stdout = "";

    const exitCode = runCli(["build", "scalar.ik", "--out", "build/libscalar"], {
      cwd,
      platform: "linux",
      runCommand: runner,
      stdout: (text) => {
        stdout += text;
      },
      stderr: () => {}
    });

    expect(exitCode).toBe(0);
    expect(stdout).toBe(`OK: built library with overflow=unchecked\n${join(cwd, "build/libscalar.so")}\n`);
    expect(readFileSync(join(cwd, "build/libscalar.c"), "utf8")).toContain("int64_t add");
    expect(readFileSync(join(cwd, "build/libscalar.h"), "utf8")).toContain("int64_t add");
    expect(calls).toEqual([
      { command: "clang", args: ["--version"] },
      {
        command: "clang",
        args: [...strictClangFlags, buildDllFlag, "-shared", "-fPIC", join(cwd, "build/libscalar.c"), "-o", join(cwd, "build/libscalar.so")]
      }
    ]);
  });

  it("passes checked overflow mode through build", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    const calls: Array<{ command: string; args: string[] }> = [];
    const runner: CommandRunner = (command, args) => {
      calls.push({ command, args });
      return { status: 0, stdout: "", stderr: "" };
    };
    let stdout = "";

    const exitCode = runCli(["build", "scalar.ik", "--out", "build/libscalar", "--overflow", "checked"], {
      cwd,
      platform: "linux",
      runCommand: runner,
      stdout: (text) => {
        stdout += text;
      },
      stderr: () => {}
    });

    expect(exitCode).toBe(0);
    expect(stdout).toBe(`OK: built library with overflow=checked\n${join(cwd, "build/libscalar.so")}\n`);
    expect(readFileSync(join(cwd, "build/libscalar.c"), "utf8")).toContain("IK_Status add");
    expect(readFileSync(join(cwd, "build/libscalar.h"), "utf8")).toContain("IK_Status add");
    expect(calls).toEqual([
      { command: "clang", args: ["--version"] },
      {
        command: "clang",
        args: [...strictClangFlags, buildDllFlag, "-shared", "-fPIC", join(cwd, "build/libscalar.c"), "-o", join(cwd, "build/libscalar.so")]
      }
    ]);
  });

  it("uses strict platform shared library flags for macOS and Windows", () => {
    for (const [platform, extension, platformFlags] of [
      ["darwin", ".dylib", ["-shared", "-fPIC"]],
      ["win32", ".dll", ["-shared"]]
    ] as const) {
      const cwd = tempDir();
      writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
      const calls: Array<{ command: string; args: string[] }> = [];
      const runner: CommandRunner = (command, args) => {
        calls.push({ command, args });
        return { status: 0, stdout: "", stderr: "" };
      };

      const exitCode = runCli(["build", "scalar.ik", "--out", "build/libscalar"], {
        cwd,
        platform,
        runCommand: runner,
        stdout: () => {},
        stderr: () => {}
      });

      expect(exitCode).toBe(0);
      expect(calls.at(-1)).toEqual({
        command: "clang",
        args: [...strictClangFlags, buildDllFlag, ...platformFlags, join(cwd, "build/libscalar.c"), "-o", join(cwd, `build/libscalar${extension}`)]
      });
    }
  });

  it("prints a friendly error when clang is not found", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stderr = "";
    const runner: CommandRunner = () => ({
      status: null,
      stdout: "",
      stderr: "",
      error: Object.assign(new Error("spawn clang ENOENT"), { code: "ENOENT" })
    });

    const exitCode = runCli(["build", "scalar.ik", "--out", "build/libscalar"], {
      cwd,
      platform: "darwin",
      runCommand: runner,
      stdout: () => {},
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stderr).toContain("clang was not found. Install clang and make sure it is available on PATH.");
  });

  it("prints a friendly error for checked build when clang is not found", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stdout = "";
    let stderr = "";
    const runner: CommandRunner = () => ({
      status: null,
      stdout: "",
      stderr: "",
      error: Object.assign(new Error("spawn clang ENOENT"), { code: "ENOENT" })
    });

    const exitCode = runCli(["build", "scalar.ik", "--out", "build/libscalar", "--overflow", "checked"], {
      cwd,
      platform: "linux",
      runCommand: runner,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe("");
    expect(stderr).toContain("clang was not found. Install clang and make sure it is available on PATH.");
    expect(readFileSync(join(cwd, "build/libscalar.c"), "utf8")).toContain("IK_Status add");
    expect(readFileSync(join(cwd, "build/libscalar.h"), "utf8")).toContain("IK_Status add");
  });

  it("prints clang stderr when build compilation fails", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "scalar.ik"), "export fn add(a: i64, b: i64) -> i64 {\n  return a + b;\n}\n");
    let stderr = "";
    let callCount = 0;
    const runner: CommandRunner = () => {
      callCount += 1;
      return callCount === 1
        ? { status: 0, stdout: "clang version", stderr: "" }
        : { status: 1, stdout: "", stderr: "clang strict failure\n" };
    };

    const exitCode = runCli(["build", "scalar.ik", "--out", "build/libscalar"], {
      cwd,
      platform: "linux",
      runCommand: runner,
      stdout: () => {},
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(1);
    expect(stderr).toBe("clang strict failure\n\n");
  });
});

describe("pricing example end-to-end", () => {
  it("generates pricing.c and pricing.h from examples/pricing.ik", () => {
    const cwd = tempDir();
    const { cFile, headerFile } = emitPricingExample(cwd);

    expect(readFileSync(headerFile, "utf8")).toContain("typedef struct Item");
    expect(readFileSync(headerFile, "utf8")).toContain("int32_t calc_items(Item* items, int32_t len, int64_t* out);");
    expect(readFileSync(cFile, "utf8")).toContain('#include "pricing.h"');
    expect(readFileSync(cFile, "utf8")).toContain("int32_t calc_items(Item* items, int32_t len, int64_t* out)");
  });

  const clangAvailable = hasClang();
  it.skipIf(!clangAvailable)(
    clangAvailable ? "compiles and runs a C harness for calc_items with strict clang flags" : "compiles and runs a C harness for calc_items (skipped because clang was not found)",
    () => {
    const cwd = tempDir();
    const { cFile } = emitPricingExample(cwd);
    const harnessFile = join(cwd, "build/pricing_harness.c");
    const executable = join(cwd, "build/pricing_harness");

    writeFileSync(
      harnessFile,
      `
#include "pricing.h"

int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};

  if (calc_items(items, 2, out) != 0) {
    return 10;
  }
  if (out[0] != 20567) {
    return 20;
  }
  if (out[1] != 11000) {
    return 21;
  }
  return 0;
}
`.trimStart()
    );

    const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
      encoding: "utf8"
    });
    expect(compile.status, compile.stderr).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable ? "compiles and runs a checked C harness for calc_items with strict clang flags" : "compiles and runs a checked C harness for calc_items (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitPricingCheckedExample(cwd);
      const harnessFile = join(cwd, "build/pricing_checked_harness.c");
      const executable = join(cwd, "build/pricing_checked_harness");

      writeFileSync(
        harnessFile,
        `
#include "pricing_checked.h"

#include <limits.h>
#include <stdint.h>

int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};
  int32_t ik_return = -1;

  IK_Status status = calc_items(items, 2, out, &ik_return);
  if (status != IK_OK) {
    return 10;
  }
  if (ik_return != 0) {
    return 11;
  }
  if (out[0] != 20567) {
    return 20;
  }
  if (out[1] != 11000) {
    return 21;
  }

  Item overflow_items[1] = {
    { .price = INT64_MAX, .qty = 2, .discount = 0, .tax_rate_ppm = 0 }
  };
  int64_t overflow_out[1] = {0};
  ik_return = -1;

  status = calc_items(overflow_items, 1, overflow_out, &ik_return);
  if (status != IK_ERR_OVERFLOW) {
    return 30;
  }

  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable ? "verifies Item struct layout with C11 static assertions" : "verifies Item struct layout (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      emitPricingExample(cwd);
      const harnessFile = join(cwd, "build/pricing_layout.c");
      const objectFile = join(cwd, "build/pricing_layout.o");

      writeFileSync(
        harnessFile,
        `
#include "pricing.h"
#include <stddef.h>
#include <stdint.h>

_Static_assert(sizeof(Item) == 32, "unexpected Item size");
_Static_assert(offsetof(Item, price) == 0, "unexpected price offset");
_Static_assert(offsetof(Item, qty) == 8, "unexpected qty offset");
_Static_assert(offsetof(Item, discount) == 16, "unexpected discount offset");
_Static_assert(offsetof(Item, tax_rate_ppm) == 24, "unexpected tax_rate_ppm offset");
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, "-c", harnessFile, "-I", join(cwd, "build"), "-o", objectFile], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable ? "allows checked generated headers to be included from C" : "allows checked generated headers to be included from C (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      emitPricingCheckedHeader(cwd);
      const cFile = join(cwd, "pricing_checked_include.c");
      const objectFile = join(cwd, "pricing_checked_include.o");

      writeFileSync(
        cFile,
        `
#include "pricing_checked.h"

int main(void) {
  IK_Status status = IK_OK;
  return status == IK_OK ? 0 : 1;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, "-c", cFile, "-I", cwd, "-o", objectFile], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);
    }
  );

  const clangCppAvailable = hasClangCpp();
  const cppHarnessAvailable = clangAvailable && clangCppAvailable;
  it.skipIf(!cppHarnessAvailable)(
    cppHarnessAvailable
      ? "compiles and runs a C++ harness for calc_items with strict clang++ flags"
      : `compiles and runs a C++ harness for calc_items (${clangCppAvailable ? "skipped because clang was not found" : "skipped because clang++ was not found"})`,
    () => {
      const cwd = tempDir();
      const { cFile } = emitPricingExample(cwd);
      const objectFile = join(cwd, "build/pricing.o");
      const cppFile = join(cwd, "build/pricing_harness.cpp");
      const executable = join(cwd, "build/pricing_cpp_harness");

      writeFileSync(
        cppFile,
        `
#include "pricing.h"

int main() {
  Item items[2] = {
    {10000, 2, 1000, 82500},
    {2500, 4, 0, 100000}
  };
  int64_t out[2] = {0, 0};

  if (calc_items(items, 2, out) != 0) {
    return 10;
  }
  if (out[0] != 20567) {
    return 20;
  }
  if (out[1] != 11000) {
    return 21;
  }
  return 0;
}
`.trimStart()
      );

      const compileC = spawnSync("clang", [...strictClangFlags, buildDllFlag, "-c", cFile, "-I", join(cwd, "build"), "-o", objectFile], {
        encoding: "utf8"
      });
      expect(compileC.status, compileC.stderr).toBe(0);

      const compileCpp = spawnSync("clang++", [...strictClangCppFlags, cppFile, objectFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compileCpp.status, compileCpp.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangCppAvailable)(
    clangCppAvailable
      ? "allows checked generated headers to be included from C++"
      : "allows checked generated headers to be included from C++ (skipped because clang++ was not found)",
    () => {
      const cwd = tempDir();
      emitPricingCheckedHeader(cwd);
      const cppFile = join(cwd, "pricing_checked_include.cpp");
      const objectFile = join(cwd, "pricing_checked_include_cpp.o");

      writeFileSync(
        cppFile,
        `
#include "pricing_checked.h"

int main() {
  IK_Status status = IK_OK;
  return status == IK_OK ? 0 : 1;
}
`.trimStart()
      );

      const compile = spawnSync("clang++", [...strictClangCppFlags, "-c", cppFile, "-I", cwd, "-o", objectFile], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);
    }
  );
});

describe("MIR scalar unchecked C emitter end-to-end", () => {
  const clangAvailable = hasClang();
  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs a scalar harness emitted from MIR with strict clang flags"
      : "compiles and runs a scalar harness emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitMirScalarUncheckedExample(cwd);
      const harnessFile = join(cwd, "build/scalar_mir_harness.c");
      const executable = join(cwd, "build/scalar_mir_harness");

      writeFileSync(
        harnessFile,
        `
#include "scalar_mir.h"

#include <stdbool.h>
#include <stdint.h>

int main(void) {
  if (add_i64(1, 2) != 3) {
    return 10;
  }
  if (mul_i64(6, 7) != 42) {
    return 11;
  }
  if (!less_i64(1, 2)) {
    return 12;
  }
  if (neg_i64(5) != -5) {
    return 13;
  }
  if (!not_bool(false)) {
    return 14;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs a control-flow harness emitted from MIR with strict clang flags"
      : "compiles and runs a control-flow harness emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitMirControlFlowUncheckedExample(cwd);
      const harnessFile = join(cwd, "build/control_mir_harness.c");
      const executable = join(cwd, "build/control_mir_harness");

      writeFileSync(
        harnessFile,
        `
#include "control_mir.h"

#include <stdint.h>

int main(void) {
  if (max_i32(3, 9) != 9) {
    return 10;
  }
  if (max_i32(12, 4) != 12) {
    return 11;
  }
  if (positive_or_zero(7) != 7) {
    return 12;
  }
  if (positive_or_zero(-3) != 0) {
    return 13;
  }
  if (sum_to_n(5) != 10) {
    return 14;
  }
  if (sum_to_n(0) != 0) {
    return 15;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs a short-circuit harness emitted from MIR with strict clang flags"
      : "compiles and runs a short-circuit harness emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitMirShortCircuitUncheckedExample(cwd);
      const harnessFile = join(cwd, "build/short_circuit_mir_harness.c");
      const executable = join(cwd, "build/short_circuit_mir_harness");

      writeFileSync(
        harnessFile,
        `
#include "short_circuit_mir.h"

#include <stdbool.h>
#include <stdint.h>

int main(void) {
  if (and_short_circuit(0, 10) != false) {
    return 10;
  }
  if (and_short_circuit(2, 10) != true) {
    return 11;
  }
  if (or_short_circuit(0, 10) != true) {
    return 12;
  }
  if (or_short_circuit(2, 10) != true) {
    return 13;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs a function-call harness emitted from MIR with strict clang flags"
      : "compiles and runs a function-call harness emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile, headerFile } = emitMirCallsUncheckedExample(cwd);
      const header = readFileSync(headerFile, "utf8");
      const harnessFile = join(cwd, "build/calls_mir_harness.c");
      const executable = join(cwd, "build/calls_mir_harness");

      expect(header).toContain("IK_API int64_t calc(int64_t a, int64_t b);");
      expect(header).not.toContain("add_i64");
      expect(header).not.toContain("double_i64");

      writeFileSync(
        harnessFile,
        `
#include "calls_mir.h"

#include <stdint.h>

int main(void) {
  if (calc(1, 2) != 6) {
    return 10;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs pricing emitted from MIR with strict clang flags"
      : "compiles and runs pricing emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitMirPricingUncheckedExample(cwd);
      const harnessFile = join(cwd, "build/pricing_mir_harness.c");
      const executable = join(cwd, "build/pricing_mir_harness");

      writeFileSync(
        harnessFile,
        `
#include "pricing_mir.h"

#include <stdint.h>

int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};

  if (calc_items(items, 2, out) != 0) {
    return 10;
  }
  if (out[0] != 20567) {
    return 20;
  }
  if (out[1] != 11000) {
    return 21;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );
});

describe("MIR checked C emitter end-to-end", () => {
  const clangAvailable = hasClang();

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs checked scalar arithmetic emitted from MIR with strict clang flags"
      : "compiles and runs checked scalar arithmetic emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitMirCheckedExample(cwd, "scalar_checked");
      const harnessFile = join(cwd, "build/scalar_checked_mir_harness.c");
      const executable = join(cwd, "build/scalar_checked_mir_harness");

      writeFileSync(
        harnessFile,
        `
#include "scalar_checked_mir_checked.h"

#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

int main(void) {
  int64_t value = 0;
  bool flag = false;

  if (add_i64(1, 2, &value) != IK_OK || value != 3) {
    return 10;
  }
  if (add_i64(INT64_MAX, 1, &value) != IK_ERR_OVERFLOW) {
    return 11;
  }
  if (mul_i64(INT64_MAX, 2, &value) != IK_ERR_OVERFLOW) {
    return 12;
  }
  if (div_i64(10, 0, &value) != IK_ERR_DIV_BY_ZERO) {
    return 13;
  }
  if (div_i64(INT64_MIN, -1, &value) != IK_ERR_OVERFLOW) {
    return 14;
  }
  if (neg_i64(INT64_MIN, &value) != IK_ERR_OVERFLOW) {
    return 15;
  }
  if (less_i64(1, 2, &flag) != IK_OK || !flag) {
    return 16;
  }
  if (add_i64(1, 2, NULL) != IK_ERR_NULL_POINTER) {
    return 17;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs checked control flow emitted from MIR with strict clang flags"
      : "compiles and runs checked control flow emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitMirCheckedExample(cwd, "scalar_control_checked");
      const harnessFile = join(cwd, "build/scalar_control_checked_mir_harness.c");
      const executable = join(cwd, "build/scalar_control_checked_mir_harness");

      writeFileSync(
        harnessFile,
        `
#include "scalar_control_checked_mir_checked.h"

#include <limits.h>
#include <stdint.h>

int main(void) {
  int64_t value = 0;

  if (sum_to_n(5, &value) != IK_OK || value != 10) {
    return 10;
  }
  if (choose(10, 3, &value) != IK_OK || value != 10) {
    return 11;
  }
  if (condition_overflow(INT64_MAX, 1, &value) != IK_ERR_OVERFLOW) {
    return 12;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs checked short-circuit emitted from MIR with strict clang flags"
      : "compiles and runs checked short-circuit emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitMirCheckedExample(cwd, "scalar_logical_checked");
      const harnessFile = join(cwd, "build/scalar_logical_checked_mir_harness.c");
      const executable = join(cwd, "build/scalar_logical_checked_mir_harness");

      writeFileSync(
        harnessFile,
        `
#include "scalar_logical_checked_mir_checked.h"

#include <stdbool.h>
#include <stdint.h>

int main(void) {
  bool flag = true;

  if (and_short_circuit(0, 10, &flag) != IK_OK || flag != false) {
    return 10;
  }
  if (and_short_circuit(2, 10, &flag) != IK_OK || flag != true) {
    return 11;
  }
  if (or_short_circuit(0, 10, &flag) != IK_OK || flag != true) {
    return 12;
  }
  if (or_short_circuit(2, 10, &flag) != IK_OK || flag != true) {
    return 13;
  }
  if (and_rhs_error(false, 10, 0, &flag) != IK_OK || flag != false) {
    return 14;
  }
  if (and_rhs_error(true, 10, 0, &flag) != IK_ERR_DIV_BY_ZERO) {
    return 15;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs checked function calls emitted from MIR with strict clang flags"
      : "compiles and runs checked function calls emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile, headerFile } = emitMirCheckedExample(cwd, "scalar_calls_checked");
      const header = readFileSync(headerFile, "utf8");
      const harnessFile = join(cwd, "build/scalar_calls_checked_mir_harness.c");
      const executable = join(cwd, "build/scalar_calls_checked_mir_harness");

      expect(header).toContain("IK_API IK_Status calc(int64_t a, int64_t b, int64_t* ik_return);");
      expect(header).not.toContain("add_i64");
      expect(header).not.toContain("double_i64");

      writeFileSync(
        harnessFile,
        `
#include "scalar_calls_checked_mir_checked.h"

#include <limits.h>
#include <stdint.h>

int main(void) {
  int64_t value = 0;

  if (calc(1, 2, &value) != IK_OK || value != 6) {
    return 10;
  }
  if (calc_overflow(INT64_MAX, 1, &value) != IK_ERR_OVERFLOW) {
    return 11;
  }
  if (calc_overflow(INT64_MAX / 2 + 1, 0, &value) != IK_ERR_OVERFLOW) {
    return 12;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs checked pricing emitted from MIR with strict clang flags"
      : "compiles and runs checked pricing emitted from MIR (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitMirCheckedExample(cwd, "pricing");
      const harnessFile = join(cwd, "build/pricing_mir_checked_harness.c");
      const executable = join(cwd, "build/pricing_mir_checked_harness");

      writeFileSync(
        harnessFile,
        `
#include "pricing_mir_checked.h"

#include <limits.h>
#include <stdint.h>

int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};
  int32_t ik_return = -1;

  IK_Status status = calc_items(items, 2, out, &ik_return);
  if (status != IK_OK) {
    return 10;
  }
  if (ik_return != 0) {
    return 11;
  }
  if (out[0] != 20567) {
    return 20;
  }
  if (out[1] != 11000) {
    return 21;
  }

  Item overflow_items[1] = {
    { .price = INT64_MAX, .qty = 2, .discount = 0, .tax_rate_ppm = 0 }
  };
  int64_t overflow_out[1] = {0};
  ik_return = -1;

  status = calc_items(overflow_items, 1, overflow_out, &ik_return);
  if (status != IK_ERR_OVERFLOW) {
    return 30;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );
});

const pipelineRegressionCases: PipelineRegressionCase[] = [
  {
    name: "scalar unchecked",
    sourceText: readFileSync("examples/scalar.ik", "utf8"),
    overflowMode: "unchecked",
    harnessSource: `
#include "kernel.h"

#include <stdio.h>
#include <stdint.h>

int main(void) {
  printf("scalar:%lld:%d:%d\\n", (long long)add_i64(2, 3), max_i32(7, 4), max_i32(1, 4));
  return 0;
}
`.trimStart()
  },
  {
    name: "pricing unchecked",
    sourceText: readFileSync("examples/pricing.ik", "utf8"),
    overflowMode: "unchecked",
    harnessSource: `
#include "kernel.h"

#include <stdio.h>
#include <stdint.h>

int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};
  int32_t result = calc_items(items, 2, out);

  printf("pricing:%d:%lld:%lld\\n", result, (long long)out[0], (long long)out[1]);
  return 0;
}
`.trimStart()
  },
  {
    name: "control flow unchecked",
    sourceText: readFileSync("examples/scalar_control_checked.ik", "utf8"),
    overflowMode: "unchecked",
    harnessSource: `
#include "kernel.h"

#include <stdio.h>
#include <stdint.h>

int main(void) {
  printf(
    "control:%lld:%lld:%lld\\n",
    (long long)sum_to_n(5),
    (long long)choose(10, 3),
    (long long)condition_overflow(1, 2)
  );
  return 0;
}
`.trimStart()
  },
  {
    name: "function calls unchecked",
    sourceText: readFileSync("examples/scalar_calls_checked.ik", "utf8"),
    overflowMode: "unchecked",
    harnessSource: `
#include "kernel.h"

#include <stdio.h>
#include <stdint.h>

int main(void) {
  printf("calls:%lld:%lld\\n", (long long)calc(1, 2), (long long)calc_overflow(3, 4));
  return 0;
}
`.trimStart()
  },
  {
    name: "short-circuit unchecked",
    sourceText: readFileSync("examples/scalar_logical_checked.ik", "utf8"),
    overflowMode: "unchecked",
    harnessSource: `
#include "kernel.h"

#include <stdbool.h>
#include <stdio.h>
#include <stdint.h>

int main(void) {
  printf(
    "logical:%d:%d:%d:%d:%d\\n",
    and_short_circuit(0, 10) ? 1 : 0,
    and_short_circuit(2, 10) ? 1 : 0,
    or_short_circuit(0, 10) ? 1 : 0,
    or_short_circuit(2, 10) ? 1 : 0,
    and_rhs_error(false, 10, 0) ? 1 : 0
  );
  return 0;
}
`.trimStart()
  },
  {
    name: "scalar checked",
    sourceText: readFileSync("examples/scalar_checked.ik", "utf8"),
    overflowMode: "checked",
    harnessSource: `
#include "kernel.h"

#include <limits.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdint.h>

int main(void) {
  int64_t value = 0;
  bool flag = false;

  IK_Status add_status = add_i64(1, 2, &value);
  int64_t add_value = value;
  IK_Status overflow_status = add_i64(INT64_MAX, 1, &value);
  IK_Status div_zero_status = div_i64(10, 0, &value);
  IK_Status neg_status = neg_i64(INT64_MIN, &value);
  IK_Status less_status = less_i64(1, 2, &flag);

  printf(
    "checked-scalar:%d:%lld:%d:%d:%d:%d:%d\\n",
    (int)add_status,
    (long long)add_value,
    (int)overflow_status,
    (int)div_zero_status,
    (int)neg_status,
    (int)less_status,
    flag ? 1 : 0
  );
  return 0;
}
`.trimStart()
  },
  {
    name: "pricing checked",
    sourceText: readFileSync("examples/pricing.ik", "utf8"),
    overflowMode: "checked",
    harnessSource: `
#include "kernel.h"

#include <limits.h>
#include <stdio.h>
#include <stdint.h>

int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};
  int32_t ik_return = -1;
  IK_Status status = calc_items(items, 2, out, &ik_return);

  Item overflow_items[1] = {
    { .price = INT64_MAX, .qty = 2, .discount = 0, .tax_rate_ppm = 0 }
  };
  int64_t overflow_out[1] = {0};
  int32_t overflow_return = -1;
  IK_Status overflow_status = calc_items(overflow_items, 1, overflow_out, &overflow_return);

  printf(
    "checked-pricing:%d:%d:%lld:%lld:%d\\n",
    (int)status,
    ik_return,
    (long long)out[0],
    (long long)out[1],
    (int)overflow_status
  );
  return 0;
}
`.trimStart()
  }
];

describe("AST and MIR C pipeline regression", () => {
  const clangAvailable = hasClang();

  for (const testCase of pipelineRegressionCases) {
    it.skipIf(!clangAvailable)(
      clangAvailable
        ? `matches AST and MIR behavior for ${testCase.name}`
        : `matches AST and MIR behavior for ${testCase.name} (skipped because clang was not found)`,
      () => {
        const cwd = tempDir();
        const fixtureName = testCase.name.replaceAll(/[^A-Za-z0-9_-]/g, "_");
        const { ast, mir } = emitAstAndMirPipelines(cwd, fixtureName, testCase.sourceText, testCase.overflowMode);

        expect(mir.headerText).toBe(ast.headerText);

        const astOutput = compileAndRunPipeline(cwd, `${fixtureName}-ast`, ast, testCase.harnessSource);
        const mirOutput = compileAndRunPipeline(cwd, `${fixtureName}-mir`, mir, testCase.harnessSource);

        expect(mirOutput).toBe(astOutput);
      }
    );
  }
});

describe("checked scalar example end-to-end", () => {
  const clangAvailable = hasClang();
  it.skipIf(!clangAvailable)(
    clangAvailable ? "compiles and runs checked scalar arithmetic harness with strict clang flags" : "compiles and runs checked scalar arithmetic harness (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitScalarCheckedExample(cwd);
      const harnessFile = join(cwd, "build/scalar_checked_harness.c");
      const executable = join(cwd, "build/scalar_checked_harness");

      writeFileSync(
        harnessFile,
        `
#include "scalar_checked.h"

#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

int main(void) {
  int64_t value = 0;
  bool flag = false;

  if (add_i64(1, 2, &value) != IK_OK || value != 3) {
    return 10;
  }
  if (add_i64(INT64_MAX, 1, &value) != IK_ERR_OVERFLOW) {
    return 11;
  }
  if (mul_i64(INT64_MAX, 2, &value) != IK_ERR_OVERFLOW) {
    return 12;
  }
  if (div_i64(10, 0, &value) != IK_ERR_DIV_BY_ZERO) {
    return 13;
  }
  if (div_i64(INT64_MIN, -1, &value) != IK_ERR_OVERFLOW) {
    return 14;
  }
  if (neg_i64(INT64_MIN, &value) != IK_ERR_OVERFLOW) {
    return 15;
  }
  if (less_i64(1, 2, &flag) != IK_OK || !flag) {
    return 16;
  }
  if (add_i64(1, 2, NULL) != IK_ERR_NULL_POINTER) {
    return 17;
  }

  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs checked scalar control flow harness with strict clang flags"
      : "compiles and runs checked scalar control flow harness (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitScalarControlCheckedExample(cwd);
      const harnessFile = join(cwd, "build/scalar_control_checked_harness.c");
      const executable = join(cwd, "build/scalar_control_checked_harness");

      writeFileSync(
        harnessFile,
        `
#include "scalar_control_checked.h"

#include <limits.h>
#include <stdint.h>

int main(void) {
  int64_t value = 0;

  if (sum_to_n(5, &value) != IK_OK || value != 10) {
    return 10;
  }
  if (choose(10, 3, &value) != IK_OK || value != 10) {
    return 11;
  }
  if (condition_overflow(INT64_MAX, 1, &value) != IK_ERR_OVERFLOW) {
    return 12;
  }

  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs checked logical short-circuit harness with strict clang flags"
      : "compiles and runs checked logical short-circuit harness (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile } = emitScalarLogicalCheckedExample(cwd);
      const harnessFile = join(cwd, "build/scalar_logical_checked_harness.c");
      const executable = join(cwd, "build/scalar_logical_checked_harness");

      writeFileSync(
        harnessFile,
        `
#include "scalar_logical_checked.h"

#include <stdbool.h>
#include <stdint.h>

int main(void) {
  bool value = false;

  if (and_short_circuit(0, 10, &value) != IK_OK || value != false) {
    return 10;
  }
  if (and_short_circuit(2, 10, &value) != IK_OK || value != true) {
    return 11;
  }
  if (or_short_circuit(0, 10, &value) != IK_OK || value != true) {
    return 12;
  }
  if (or_short_circuit(2, 10, &value) != IK_OK || value != true) {
    return 13;
  }
  if (and_rhs_error(false, 10, 0, &value) != IK_OK || value != false) {
    return 14;
  }
  if (and_rhs_error(true, 10, 0, &value) != IK_ERR_DIV_BY_ZERO) {
    return 15;
  }

  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable
      ? "compiles and runs checked function call propagation harness with strict clang flags"
      : "compiles and runs checked function call propagation harness (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      const { cFile, headerFile } = emitScalarCallsCheckedExample(cwd);
      const harnessFile = join(cwd, "build/scalar_calls_checked_harness.c");
      const executable = join(cwd, "build/scalar_calls_checked_harness");
      const headerText = readFileSync(headerFile, "utf8");
      const sourceText = readFileSync(cFile, "utf8");

      expect(headerText).toContain("IK_API IK_Status calc(int64_t a, int64_t b, int64_t* ik_return);");
      expect(headerText).toContain("IK_API IK_Status calc_overflow(int64_t a, int64_t b, int64_t* ik_return);");
      expect(headerText).not.toContain("add_i64");
      expect(headerText).not.toContain("double_i64");
      expect(sourceText).toContain("static IK_Status add_i64");
      expect(sourceText).toContain("static IK_Status double_i64");

      writeFileSync(
        harnessFile,
        `
#include "scalar_calls_checked.h"

#include <limits.h>
#include <stdint.h>

int main(void) {
  int64_t value = 0;

  if (calc(1, 2, &value) != IK_OK || value != 6) {
    return 10;
  }
  if (calc_overflow(INT64_MAX, 1, &value) != IK_ERR_OVERFLOW) {
    return 11;
  }
  if (calc_overflow(INT64_MAX / 2 + 1, 0, &value) != IK_ERR_OVERFLOW) {
    return 12;
  }

  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, buildDllFlag, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], {
        encoding: "utf8"
      });
      expect(compile.status, compile.stderr).toBe(0);

      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );
});
