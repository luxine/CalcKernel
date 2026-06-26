import { readdirSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const rootDir = process.cwd();
const excludedDirs = new Set([".git", "node_modules", "dist", "build", "coverage", "release"]);
const excludedLockfiles = new Set(["bun.lockb", "package-lock.json", "pnpm-lock.yaml", "yarn.lock"]);
const excludedFiles = new Set([
  "AGENTS.md",
  "docs/NAMING_CONVENTIONS.md",
  "docs/zh-CN/NAMING_CONVENTIONS.md",
  "tests/naming-consistency.test.ts",
]);
const historicalOrMigrationPrefixes = [
  "Ai_repository/",
  "docs/MIGRATION_IK_TO_CK.md",
  "docs/MIGRATION.md",
  "docs/zh-CN/MIGRATION_IK_TO_CK.md",
  "docs/zh-CN/MIGRATION.md",
  "bench/docs/2026-06-24-",
  "bench/plans/2026-06-24-",
  "ck-vscode-plugin/docs/superpowers/specs/",
  "ck-vscode-plugin/docs/superpowers/plans/"
];
const legacyVsCodePluginDir = "i" + "k-vscode-plugin";
const excludedExtensions = new Set([".wasm", ".so", ".dylib", ".dll", ".exe", ".tgz"]);
const forbiddenPatterns = [
  { label: "legacy source extension", pattern: new RegExp("\\." + "i" + "k\\b", "g") },
  { label: "legacy diagnostic code prefix", pattern: /\bIK[0-9]{4}\b/g },
  { label: "legacy project abbreviation", pattern: /\bIK\b/g },
  { label: "legacy project name", pattern: /\bIntKernel\b/g },
  { label: "legacy package name", pattern: /\bintkernel\b/g },
  { label: "legacy compiler command", pattern: /\bikc\b/g },
  { label: "legacy language alias", pattern: /"ik"/g },
  { label: "legacy markdown code fence", pattern: /```ik\b/g },
  { label: "legacy benchmark slug", pattern: /-ik-/g },
  { label: "legacy C ABI export macro", pattern: /\bIK_API\b/g },
  { label: "legacy C ABI build macro", pattern: /\bIK_BUILD_DLL\b/g },
  { label: "legacy C ABI status typedef", pattern: /\bIK_Status\b/g },
  { label: "legacy C ABI ok macro", pattern: /\bIK_OK\b/g },
  { label: "legacy C ABI error macro", pattern: /\bIK_ERR_(?:OVERFLOW|DIV_BY_ZERO|NULL_POINTER)\b/g },
  { label: "legacy C ABI prefix", pattern: /\bIK_/g },
  { label: "legacy checked return parameter", pattern: /\bik_return\b/g },
  { label: "legacy checked return variable", pattern: /\bikReturn\b/g },
  { label: "legacy VS Code plugin directory", pattern: new RegExp(`\\b${legacyVsCodePluginDir}\\b`, "g") },
  { label: "legacy language name", pattern: /\btk\b/g },
  { label: "legacy compiler command", pattern: /\btkc\b/g },
  { label: "legacy source extension", pattern: /\.tk\b/g },
  { label: "invalid language rename", pattern: /\bLK\b/g },
  { label: "invalid lowercase language rename", pattern: /\blk\b/g },
  { label: "invalid compiler command rename", pattern: /\blkc\b/g },
  { label: "invalid source extension rename", pattern: /\.lk\b/g }
];
const packageCliLegacyPatterns = [
  { label: "legacy project abbreviation", pattern: /\bIK\b/g },
  { label: "legacy project name", pattern: /\bIntKernel\b/g },
  { label: "legacy package name", pattern: /\bintkernel\b/g },
  { label: "legacy compiler command", pattern: /\bikc\b/g }
];

function walk(dir: string): string[] {
  const entries = readdirSync(dir, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const path = join(dir, entry.name);
    const relativePath = normalize(relative(rootDir, path));

    if (entry.isDirectory()) {
      if (!excludedDirs.has(entry.name)) {
        files.push(...walk(path));
      }
      continue;
    }

    if (
      !entry.isFile() ||
      excludedFiles.has(relativePath) ||
      isHistoricalOrMigrationFile(relativePath) ||
      shouldSkipByExtension(entry.name) ||
      excludedLockfiles.has(entry.name)
    ) {
      continue;
    }

    files.push(path);
  }

  return files;
}

function shouldSkipByExtension(fileName: string): boolean {
  for (const extension of excludedExtensions) {
    if (fileName.endsWith(extension)) {
      return true;
    }
  }

  return false;
}

function normalize(path: string): string {
  return path.split(sep).join("/");
}

function isHistoricalOrMigrationFile(relativePath: string): boolean {
  return historicalOrMigrationPrefixes.some((prefix) => relativePath.startsWith(prefix));
}

function isText(content: Buffer): boolean {
  return !content.includes(0);
}

describe("naming consistency", () => {
  it("does not reintroduce forbidden aliases or source extensions", () => {
    const violations: string[] = [];

    for (const file of walk(rootDir)) {
      const content = readFileSync(file);
      if (!isText(content)) {
        continue;
      }

      const text = content.toString("utf8");
      const lines = text.split(/\r?\n/);
      for (const { label, pattern } of forbiddenPatterns) {
        for (const [lineIndex, line] of lines.entries()) {
          pattern.lastIndex = 0;
          if (pattern.test(line)) {
            violations.push(`${normalize(relative(rootDir, file))}:${lineIndex + 1}: ${label}: ${line.trim()}`);
          }
        }
      }
    }

    expect(violations).toEqual([]);
  });

  it("keeps package metadata on the canonical calckernel package and ckc command", () => {
    const pkg = JSON.parse(readFileSync(join(rootDir, "package.json"), "utf8")) as {
      name?: string;
      description?: string;
      bin?: Record<string, string>;
      scripts?: Record<string, string>;
      keywords?: string[];
    };

    expect(pkg.name).toBe("calckernel");
    expect(pkg.description).toContain("CalcKernel");
    expect(pkg.bin).toEqual({ ckc: "./dist/src/cli.js" });
    expect(pkg.bin).not.toHaveProperty("i" + "kc");
    expect(pkg.scripts?.ckc).toBe("node dist/src/cli.js");
    expect(pkg.scripts).not.toHaveProperty("i" + "kc");
    expect(pkg.keywords).toContain("calckernel");
    expect(pkg.keywords).not.toContain("int" + "kernel");
  });

  it("keeps VS Code extension metadata on the canonical CalcKernel language identity", () => {
    const pkg = JSON.parse(readFileSync(join(rootDir, "ck-vscode-plugin", "package.json"), "utf8")) as {
      name?: string;
      displayName?: string;
      description?: string;
      activationEvents?: string[];
      contributes?: {
        languages?: Array<{ id?: string; aliases?: string[]; extensions?: string[] }>;
        grammars?: Array<{ language?: string; scopeName?: string; path?: string }>;
        snippets?: Array<{ language?: string; path?: string }>;
      };
      devDependencies?: Record<string, string>;
    };

    expect(pkg.name).toBe("ck-vscode-plugin");
    expect(pkg.displayName).toBe("CalcKernel");
    expect(pkg.description).toContain("CalcKernel");
    expect(pkg.activationEvents).toEqual(["onLanguage:calckernel"]);
    expect(pkg.contributes?.languages).toEqual([
      {
        id: "calckernel",
        aliases: ["CalcKernel", "ck"],
        extensions: [".ck"],
        configuration: "./language-configuration.json"
      }
    ]);
    expect(pkg.contributes?.grammars).toEqual([
      {
        language: "calckernel",
        scopeName: "source.calckernel",
        path: "./syntaxes/calckernel.tmLanguage.json"
      }
    ]);
    expect(pkg.contributes?.snippets).toEqual([{ language: "calckernel", path: "./snippets/calckernel.json" }]);
    expect(pkg.devDependencies).toHaveProperty("calckernel", "file:..");
    expect(pkg.devDependencies).not.toHaveProperty("int" + "kernel");
  });

  it("keeps CLI help on the canonical ckc command name", () => {
    const stdout: string[] = [];
    const stderr: string[] = [];

    const exitCode = runCli(["--help"], {
      stdout: (text) => stdout.push(text),
      stderr: (text) => stderr.push(text)
    });

    const helpText = stdout.join("");
    const violations = [...forbiddenPatterns, ...packageCliLegacyPatterns]
      .filter(({ pattern }) => {
        pattern.lastIndex = 0;
        return pattern.test(helpText);
      })
      .map(({ label }) => label);

    expect(exitCode).toBe(0);
    expect(stderr).toEqual([]);
    expect(helpText).toContain("ckc check <file>");
    expect(violations).toEqual([]);
  });

  it("keeps the IK to CK migration guide explicit about the breaking rename", () => {
    const guide = readFileSync(join(rootDir, "docs", "MIGRATION_IK_TO_CK.md"), "utf8");

    for (const mapping of [
      "IK -> CK",
      "IntKernel -> CalcKernel",
      "ikc -> ckc",
      ".ik -> .ck",
      "intkernel -> calckernel",
      "IK_API -> CK_API",
      "IK_BUILD_DLL -> CK_BUILD_DLL",
      "IK_Status -> CK_Status",
      "IK_OK -> CK_OK",
      "IK_ERR_OVERFLOW -> CK_ERR_OVERFLOW",
      "IK_ERR_DIV_BY_ZERO -> CK_ERR_DIV_BY_ZERO",
      "IK_ERR_NULL_POINTER -> CK_ERR_NULL_POINTER"
    ]) {
      expect(guide).toContain(mapping);
    }

    expect(guide).toContain("The `ikc` alias is not kept.");
    expect(guide).toContain("The `.ik` compatibility path is not kept.");
    expect(guide).toContain("The `IK_` C ABI compatibility alias is not kept.");
    expect(guide).toContain("breaking rename");
    expect(guide).toContain("v0.7.0");
  });
});
