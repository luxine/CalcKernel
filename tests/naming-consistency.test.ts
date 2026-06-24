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
const excludedExtensions = new Set([".wasm", ".so", ".dylib", ".dll", ".exe", ".tgz"]);
const forbiddenPatterns = [
  { label: "legacy language name", pattern: /\btk\b/g },
  { label: "legacy compiler command", pattern: /\btkc\b/g },
  { label: "legacy source extension", pattern: /\.tk\b/g },
  { label: "invalid language rename", pattern: /\bLK\b/g },
  { label: "invalid lowercase language rename", pattern: /\blk\b/g },
  { label: "invalid compiler command rename", pattern: /\blkc\b/g },
  { label: "invalid source extension rename", pattern: /\.lk\b/g }
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

    if (!entry.isFile() || excludedFiles.has(relativePath) || shouldSkipByExtension(entry.name) || excludedLockfiles.has(entry.name)) {
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

  it("keeps CLI help on the canonical ikc command name", () => {
    const stdout: string[] = [];
    const stderr: string[] = [];

    const exitCode = runCli(["--help"], {
      stdout: (text) => stdout.push(text),
      stderr: (text) => stderr.push(text)
    });

    const helpText = stdout.join("");
    const violations = forbiddenPatterns
      .filter(({ pattern }) => {
        pattern.lastIndex = 0;
        return pattern.test(helpText);
      })
      .map(({ label }) => label);

    expect(exitCode).toBe(0);
    expect(stderr).toEqual([]);
    expect(helpText).toContain("ikc check <file>");
    expect(violations).toEqual([]);
  });
});
