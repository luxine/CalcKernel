# IntKernel VSCode Extension V1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a VSCode extension under `/Users/lynn/code/IntKernel/ik-vscode-plugin` that recognizes `.ik` files, provides syntax highlighting, snippets, basic completions, and compiler-backed diagnostics.

**Architecture:** This is a regular VSCode extension, not an LSP server. Static editor features come from VSCode contributions, while diagnostics call the public `intkernel` package API (`SourceFile` and `check`) and convert IntKernel spans into VSCode diagnostics. The extension is bundled with esbuild so the local compiler dependency is included in the runtime output.

**Tech Stack:** TypeScript, VSCode Extension API, TextMate grammar JSON, esbuild, pnpm, Vitest, local `intkernel` package dependency via `file:..`.

---

## File Structure

- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/package.json`
  - Owns extension metadata, VSCode contribution points, scripts, dependencies, and local `intkernel` package dependency.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/tsconfig.json`
  - Type-checks extension source and Vitest tests without emitting JavaScript.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/esbuild.mjs`
  - Bundles `src/extension.ts` into `dist/extension.cjs`, excluding the `vscode` host module.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/vitest.config.ts`
  - Keeps unit tests scoped to `test/**/*.test.ts`.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/.gitignore`
  - Ignores generated extension artifacts and local dependencies.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscodeignore`
  - Excludes source-only and test-only files from `.vsix` packaging.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscode/launch.json`
  - Lets VSCode launch an Extension Development Host with `F5`.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscode/tasks.json`
  - Gives the Extension Development Host launch config a concrete `pnpm compile` prelaunch task.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/language-configuration.json`
  - Defines line comments, brackets, auto-closing pairs, and surrounding pairs
    for `{}`, `[]`, and `()` only.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/syntaxes/intkernel.tmLanguage.json`
  - Provides TextMate syntax highlighting for IntKernel.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/snippets/intkernel.json`
  - Provides snippets for common `.ik` language constructs.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/diagnosticMapping.ts`
  - Pure span-to-range coordinate conversion. This file must not import `vscode`, so it can be unit-tested in Node.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/test/diagnosticMapping.test.ts`
  - Unit tests for diagnostic range mapping and clamping.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/diagnostics.ts`
  - Registers the diagnostic collection, document listeners, debounce behavior, and compiler-backed validation.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/completions.ts`
  - Registers static keyword/type/snippet completions.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/extension.ts`
  - Extension activation entrypoint that wires diagnostics and completions together.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/README.md`
  - Documents features, local development, and manual verification.
- Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/CHANGELOG.md`
  - Records V1 initial feature set.

---

### Task 1: Scaffold Extension Package And Build Tooling

**Files:**
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/package.json`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/tsconfig.json`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/esbuild.mjs`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/vitest.config.ts`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/.gitignore`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscodeignore`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscode/launch.json`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscode/tasks.json`

- [ ] **Step 1: Create package metadata**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/package.json`:

```json
{
  "name": "ik-vscode-plugin",
  "displayName": "IntKernel",
  "description": "VSCode language support for IntKernel .ik files.",
  "version": "0.1.0",
  "publisher": "local",
  "license": "MIT",
  "engines": {
    "vscode": "^1.90.0"
  },
  "categories": [
    "Programming Languages"
  ],
  "activationEvents": [
    "onLanguage:intkernel"
  ],
  "main": "./dist/extension.cjs",
  "contributes": {
    "languages": [
      {
        "id": "intkernel",
        "aliases": [
          "IntKernel",
          "ik"
        ],
        "extensions": [
          ".ik"
        ],
        "configuration": "./language-configuration.json"
      }
    ],
    "grammars": [
      {
        "language": "intkernel",
        "scopeName": "source.intkernel",
        "path": "./syntaxes/intkernel.tmLanguage.json"
      }
    ],
    "snippets": [
      {
        "language": "intkernel",
        "path": "./snippets/intkernel.json"
      }
    ]
  },
  "scripts": {
    "precompile": "pnpm --dir .. build",
    "compile": "pnpm run precompile && tsc -p . --noEmit && node esbuild.mjs",
    "watch": "tsc -p . --noEmit --watch",
    "test": "vitest run",
    "vscode:prepublish": "pnpm run compile",
    "package": "pnpm run compile && vsce package"
  },
  "dependencies": {
    "intkernel": "file:.."
  },
  "devDependencies": {
    "@types/node": "^20.14.0",
    "@types/vscode": "^1.90.0",
    "@vscode/vsce": "^3.2.1",
    "esbuild": "^0.25.0",
    "typescript": "^5.8.0",
    "vitest": "^3.2.0"
  }
}
```

- [ ] **Step 2: Create TypeScript configuration**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "lib": [
      "ES2022"
    ],
    "strict": true,
    "noImplicitAny": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "types": [
      "node",
      "vitest/globals"
    ]
  },
  "include": [
    "src/**/*.ts",
    "test/**/*.ts",
    "vitest.config.ts"
  ]
}
```

- [ ] **Step 3: Create esbuild bundler config**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/esbuild.mjs`:

```js
import esbuild from "esbuild";

await esbuild.build({
  entryPoints: ["src/extension.ts"],
  bundle: true,
  platform: "node",
  target: "node20",
  format: "cjs",
  outfile: "dist/extension.cjs",
  external: ["vscode"],
  sourcemap: true,
  logLevel: "info"
});
```

- [ ] **Step 4: Create Vitest config**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"]
  }
});
```

- [ ] **Step 5: Create local ignore files**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/.gitignore`:

```gitignore
node_modules/
dist/
*.vsix
```

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscodeignore`:

```gitignore
.vscode/
.gitignore
docs/
node_modules/
src/
test/
tsconfig.json
vitest.config.ts
esbuild.mjs
*.vsix
```

- [ ] **Step 6: Create VSCode launch configuration**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscode/launch.json`:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "name": "Run Extension",
      "type": "extensionHost",
      "request": "launch",
      "args": [
        "--extensionDevelopmentPath=${workspaceFolder}"
      ],
      "outFiles": [
        "${workspaceFolder}/dist/**/*.js",
        "${workspaceFolder}/dist/**/*.cjs"
      ],
      "preLaunchTask": "pnpm: compile"
    }
  ]
}
```

- [ ] **Step 7: Create VSCode compile task**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/.vscode/tasks.json`:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "pnpm: compile",
      "type": "shell",
      "command": "pnpm compile",
      "group": "build",
      "problemMatcher": "$tsc"
    }
  ]
}
```

- [ ] **Step 8: Install dependencies**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm install
```

Expected:

```text
Packages: +...
Done in ...
```

This creates `/Users/lynn/code/IntKernel/ik-vscode-plugin/pnpm-lock.yaml`.

- [ ] **Step 9: Run initial compile and observe source-entry failure**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm compile
```

Expected:

```text
Could not resolve "src/extension.ts"
```

This failure is expected because Task 6 creates the extension entrypoint.

- [ ] **Step 10: Commit scaffold and planning docs**

Run:

```bash
cd /Users/lynn/code/IntKernel
git add ik-vscode-plugin/package.json ik-vscode-plugin/tsconfig.json ik-vscode-plugin/esbuild.mjs ik-vscode-plugin/vitest.config.ts ik-vscode-plugin/.gitignore ik-vscode-plugin/.vscodeignore ik-vscode-plugin/.vscode/launch.json ik-vscode-plugin/.vscode/tasks.json ik-vscode-plugin/pnpm-lock.yaml ik-vscode-plugin/docs/superpowers/specs/2026-06-23-intkernel-vscode-v1-design.md ik-vscode-plugin/docs/superpowers/plans/2026-06-23-intkernel-vscode-v1-implementation.md
git commit -m "chore: scaffold intkernel vscode plugin"
```

Expected:

```text
[main ...] chore: scaffold intkernel vscode plugin
```

---

### Task 2: Add Static Language Assets

**Files:**
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/language-configuration.json`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/syntaxes/intkernel.tmLanguage.json`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/snippets/intkernel.json`

- [ ] **Step 1: Create language configuration**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/language-configuration.json`:

```json
{
  "comments": {
    "lineComment": "//"
  },
  "brackets": [
    [
      "{",
      "}"
    ],
    [
      "[",
      "]"
    ],
    [
      "(",
      ")"
    ]
  ],
  "autoClosingPairs": [
    {
      "open": "{",
      "close": "}"
    },
    {
      "open": "[",
      "close": "]"
    },
    {
      "open": "(",
      "close": ")"
    }
  ],
  "surroundingPairs": [
    {
      "open": "{",
      "close": "}"
    },
    {
      "open": "[",
      "close": "]"
    },
    {
      "open": "(",
      "close": ")"
    }
  ]
}
```

`ptr<T>` insertion is covered by snippets and completions, so `<` remains
comfortable as a comparison operator.

- [ ] **Step 2: Create TextMate grammar**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/syntaxes/intkernel.tmLanguage.json`:

```json
{
  "$schema": "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
  "name": "IntKernel",
  "scopeName": "source.intkernel",
  "patterns": [
    {
      "include": "#comments"
    },
    {
      "include": "#functionDeclarations"
    },
    {
      "include": "#structDeclarations"
    },
    {
      "include": "#keywords"
    },
    {
      "include": "#types"
    },
    {
      "include": "#booleans"
    },
    {
      "include": "#numbers"
    },
    {
      "include": "#operators"
    }
  ],
  "repository": {
    "comments": {
      "patterns": [
        {
          "name": "comment.line.double-slash.intkernel",
          "match": "//.*$"
        }
      ]
    },
    "functionDeclarations": {
      "patterns": [
        {
          "match": "\\b(fn)\\s+([A-Za-z_][A-Za-z0-9_]*)",
          "captures": {
            "1": {
              "name": "keyword.declaration.function.intkernel"
            },
            "2": {
              "name": "entity.name.function.intkernel"
            }
          }
        }
      ]
    },
    "structDeclarations": {
      "patterns": [
        {
          "match": "\\b(struct)\\s+([A-Za-z_][A-Za-z0-9_]*)",
          "captures": {
            "1": {
              "name": "keyword.declaration.struct.intkernel"
            },
            "2": {
              "name": "entity.name.type.struct.intkernel"
            }
          }
        }
      ]
    },
    "keywords": {
      "patterns": [
        {
          "name": "keyword.control.intkernel",
          "match": "\\b(?:if|else|while|return)\\b"
        },
        {
          "name": "keyword.declaration.intkernel",
          "match": "\\b(?:export|fn|let|struct)\\b"
        }
      ]
    },
    "types": {
      "patterns": [
        {
          "name": "storage.type.primitive.intkernel",
          "match": "\\b(?:i32|i64|u32|u64|bool)\\b"
        },
        {
          "name": "storage.type.pointer.intkernel",
          "match": "\\bptr\\b"
        }
      ]
    },
    "booleans": {
      "patterns": [
        {
          "name": "constant.language.boolean.intkernel",
          "match": "\\b(?:true|false)\\b"
        }
      ]
    },
    "numbers": {
      "patterns": [
        {
          "name": "constant.numeric.integer.intkernel",
          "match": "\\b[0-9]+\\b"
        }
      ]
    },
    "operators": {
      "patterns": [
        {
          "name": "keyword.operator.intkernel",
          "match": "==|!=|<=|>=|&&|\\|\\||[+\\-*/%<>=!]"
        },
        {
          "name": "punctuation.separator.intkernel",
          "match": "[;:,]"
        },
        {
          "name": "punctuation.bracket.intkernel",
          "match": "[{}\\[\\]()]"
        }
      ]
    }
  }
}
```

- [ ] **Step 3: Create snippets**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/snippets/intkernel.json`:

```json
{
  "Struct": {
    "prefix": "struct",
    "body": [
      "struct ${1:Name} {",
      "  ${2:field}: ${3:i64};",
      "}"
    ],
    "description": "Define an IntKernel struct"
  },
  "Function": {
    "prefix": "fn",
    "body": [
      "fn ${1:name}(${2:param}: ${3:i64}) -> ${4:i64} {",
      "  return ${2:param};",
      "}"
    ],
    "description": "Define an internal IntKernel function"
  },
  "Export Function": {
    "prefix": "export fn",
    "body": [
      "export fn ${1:name}(${2:param}: ${3:i64}) -> ${4:i64} {",
      "  return ${2:param};",
      "}"
    ],
    "description": "Define an exported IntKernel function"
  },
  "Let Binding": {
    "prefix": "let",
    "body": [
      "let ${1:name}: ${2:i64} = ${3:0};"
    ],
    "description": "Create a typed let binding"
  },
  "If Statement": {
    "prefix": "if",
    "body": [
      "if ${1:condition} {",
      "  ${2:return 0;}",
      "}"
    ],
    "description": "Create an if statement"
  },
  "If Else Statement": {
    "prefix": "ifelse",
    "body": [
      "if ${1:condition} {",
      "  ${2:return 0;}",
      "} else {",
      "  ${3:return 1;}",
      "}"
    ],
    "description": "Create an if/else statement"
  },
  "While Loop": {
    "prefix": "while",
    "body": [
      "while ${1:condition} {",
      "  ${2:i = i + 1;}",
      "}"
    ],
    "description": "Create a while loop"
  },
  "Return Statement": {
    "prefix": "return",
    "body": [
      "return ${1:0};"
    ],
    "description": "Return from a function"
  },
  "Pointer Type": {
    "prefix": "ptr",
    "body": [
      "ptr<${1:Item}>"
    ],
    "description": "Insert a pointer type"
  }
}
```

- [ ] **Step 4: Run JSON validation through compile**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm compile
```

Expected:

```text
Could not resolve "src/extension.ts"
```

The package contribution JSON should not produce parse errors. The missing source entrypoint remains expected until Task 6.

- [ ] **Step 5: Commit static language assets**

Run:

```bash
cd /Users/lynn/code/IntKernel
git add ik-vscode-plugin/language-configuration.json ik-vscode-plugin/syntaxes/intkernel.tmLanguage.json ik-vscode-plugin/snippets/intkernel.json
git commit -m "feat: add intkernel vscode language assets"
```

Expected:

```text
[main ...] feat: add intkernel vscode language assets
```

---

### Task 3: Implement Diagnostic Position Mapping With Tests

**Files:**
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/diagnosticMapping.ts`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/test/diagnosticMapping.test.ts`

- [ ] **Step 1: Write diagnostic mapping tests**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/test/diagnosticMapping.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { spanToRangeCoordinates, type SourceSpanLike } from "../src/diagnosticMapping";

function span(startLine: number, startColumn: number, endLine: number, endColumn: number): SourceSpanLike {
  return {
    start: {
      line: startLine,
      column: startColumn,
      offset: 0
    },
    end: {
      line: endLine,
      column: endColumn,
      offset: 0
    }
  };
}

describe("spanToRangeCoordinates", () => {
  it("maps a same-line one-based IntKernel span to a zero-based VSCode range", () => {
    expect(spanToRangeCoordinates("let x: i64 = 0;", span(1, 5, 1, 6))).toEqual({
      start: { line: 0, character: 4 },
      end: { line: 0, character: 5 }
    });
  });

  it("expands an empty same-line range by one character when possible", () => {
    expect(spanToRangeCoordinates("abc", span(1, 2, 1, 2))).toEqual({
      start: { line: 0, character: 1 },
      end: { line: 0, character: 2 }
    });
  });

  it("preserves multiline ranges after converting to zero-based coordinates", () => {
    expect(spanToRangeCoordinates("first\nsecond", span(1, 3, 2, 4))).toEqual({
      start: { line: 0, character: 2 },
      end: { line: 1, character: 3 }
    });
  });

  it("clamps columns that extend past the end of a line", () => {
    expect(spanToRangeCoordinates("abc", span(1, 10, 1, 12))).toEqual({
      start: { line: 0, character: 2 },
      end: { line: 0, character: 3 }
    });
  });

  it("handles diagnostics positioned at an empty EOF line", () => {
    expect(spanToRangeCoordinates("abc\n", span(2, 1, 2, 1))).toEqual({
      start: { line: 1, character: 0 },
      end: { line: 1, character: 0 }
    });
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm test
```

Expected:

```text
Error: Failed to resolve import "../src/diagnosticMapping"
```

- [ ] **Step 3: Implement pure diagnostic mapping**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/diagnosticMapping.ts`:

```ts
export interface SourcePositionLike {
  offset: number;
  line: number;
  column: number;
}

export interface SourceSpanLike {
  start: SourcePositionLike;
  end: SourcePositionLike;
}

export interface RangePositionCoordinates {
  line: number;
  character: number;
}

export interface RangeCoordinates {
  start: RangePositionCoordinates;
  end: RangePositionCoordinates;
}

export function spanToRangeCoordinates(text: string, span: SourceSpanLike): RangeCoordinates {
  const lines = splitLines(text);
  const start = clampPosition(lines, span.start.line - 1, span.start.column - 1);
  let end = clampPosition(lines, span.end.line - 1, span.end.column - 1);

  if (comparePositions(end, start) < 0) {
    end = { ...start };
  }

  if (samePosition(start, end)) {
    const lineLength = lines[start.line]?.length ?? 0;
    if (start.character < lineLength) {
      end.character = start.character + 1;
    } else if (start.character > 0) {
      start.character -= 1;
    }
  }

  return { start, end };
}

function splitLines(text: string): string[] {
  const lines = text.split(/\r\n|\r|\n/);
  return lines.length > 0 ? lines : [""];
}

function clampPosition(lines: string[], line: number, character: number): RangePositionCoordinates {
  const clampedLine = clamp(line, 0, Math.max(0, lines.length - 1));
  const lineLength = lines[clampedLine]?.length ?? 0;
  return {
    line: clampedLine,
    character: clamp(character, 0, lineLength)
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function comparePositions(left: RangePositionCoordinates, right: RangePositionCoordinates): number {
  if (left.line !== right.line) {
    return left.line - right.line;
  }

  return left.character - right.character;
}

function samePosition(left: RangePositionCoordinates, right: RangePositionCoordinates): boolean {
  return left.line === right.line && left.character === right.character;
}
```

- [ ] **Step 4: Run mapping tests**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm test
```

Expected:

```text
Test Files  1 passed
Tests  5 passed
```

- [ ] **Step 5: Commit diagnostic mapping**

Run:

```bash
cd /Users/lynn/code/IntKernel
git add ik-vscode-plugin/src/diagnosticMapping.ts ik-vscode-plugin/test/diagnosticMapping.test.ts
git commit -m "feat: map intkernel diagnostics to editor ranges"
```

Expected:

```text
[main ...] feat: map intkernel diagnostics to editor ranges
```

---

### Task 4: Implement Compiler-Backed Diagnostics

**Files:**
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/diagnostics.ts`

- [ ] **Step 1: Create diagnostics service**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/diagnostics.ts`:

```ts
import * as vscode from "vscode";
import { SourceFile, check, type Diagnostic as IntKernelDiagnostic } from "intkernel";
import { spanToRangeCoordinates } from "./diagnosticMapping";

const debounceMs = 250;

export function registerDiagnostics(context: vscode.ExtensionContext): void {
  const collection = vscode.languages.createDiagnosticCollection("intkernel");
  const pending = new Map<string, NodeJS.Timeout>();

  function validateNow(document: vscode.TextDocument): void {
    if (!isIntKernelDocument(document)) {
      return;
    }

    clearPending(document, pending);
    validateDocument(document, collection);
  }

  function validateSoon(document: vscode.TextDocument): void {
    if (!isIntKernelDocument(document)) {
      return;
    }

    clearPending(document, pending);
    const key = document.uri.toString();
    pending.set(
      key,
      setTimeout(() => {
        pending.delete(key);
        validateDocument(document, collection);
      }, debounceMs)
    );
  }

  context.subscriptions.push(
    collection,
    vscode.workspace.onDidOpenTextDocument(validateNow),
    vscode.workspace.onDidSaveTextDocument(validateNow),
    vscode.workspace.onDidChangeTextDocument((event) => validateSoon(event.document)),
    vscode.workspace.onDidCloseTextDocument((document) => {
      clearPending(document, pending);
      collection.delete(document.uri);
    })
  );

  for (const document of vscode.workspace.textDocuments) {
    validateNow(document);
  }
}

function validateDocument(document: vscode.TextDocument, collection: vscode.DiagnosticCollection): void {
  try {
    const source = new SourceFile(document.fileName, document.getText());
    const result = check(source);
    collection.set(
      document.uri,
      result.diagnostics.map((diagnostic) => toVscodeDiagnostic(document, diagnostic))
    );
  } catch (error) {
    collection.set(document.uri, [unexpectedValidationDiagnostic(document, error)]);
  }
}

function toVscodeDiagnostic(document: vscode.TextDocument, diagnostic: IntKernelDiagnostic): vscode.Diagnostic {
  const coordinates = spanToRangeCoordinates(document.getText(), diagnostic.span);
  const vscodeDiagnostic = new vscode.Diagnostic(
    new vscode.Range(
      coordinates.start.line,
      coordinates.start.character,
      coordinates.end.line,
      coordinates.end.character
    ),
    diagnostic.message,
    vscode.DiagnosticSeverity.Error
  );
  vscodeDiagnostic.code = diagnostic.code;
  vscodeDiagnostic.source = "intkernel";
  return vscodeDiagnostic;
}

function unexpectedValidationDiagnostic(document: vscode.TextDocument, error: unknown): vscode.Diagnostic {
  const text = document.getText();
  const endCharacter = Math.min(1, text.split(/\r\n|\r|\n/)[0]?.length ?? 0);
  const diagnostic = new vscode.Diagnostic(
    new vscode.Range(0, 0, 0, endCharacter),
    `IntKernel validation failed: ${errorMessage(error)}`,
    vscode.DiagnosticSeverity.Error
  );
  diagnostic.source = "intkernel";
  return diagnostic;
}

function isIntKernelDocument(document: vscode.TextDocument): boolean {
  return document.languageId === "intkernel" || document.fileName.endsWith(".ik");
}

function clearPending(document: vscode.TextDocument, pending: Map<string, NodeJS.Timeout>): void {
  const key = document.uri.toString();
  const timeout = pending.get(key);
  if (timeout) {
    clearTimeout(timeout);
    pending.delete(key);
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
```

- [ ] **Step 2: Run TypeScript check and observe missing extension entrypoint**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm compile
```

Expected:

```text
Could not resolve "src/extension.ts"
```

If TypeScript reports errors in `src/diagnostics.ts`, fix those errors before continuing. The esbuild entrypoint failure remains expected until Task 6.

- [ ] **Step 3: Commit diagnostics service**

Run:

```bash
cd /Users/lynn/code/IntKernel
git add ik-vscode-plugin/src/diagnostics.ts
git commit -m "feat: add compiler-backed vscode diagnostics"
```

Expected:

```text
[main ...] feat: add compiler-backed vscode diagnostics
```

---

### Task 5: Implement Static Completions

**Files:**
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/completions.ts`

- [ ] **Step 1: Create completions provider**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/completions.ts`:

```ts
import * as vscode from "vscode";

const keywords = ["struct", "export", "fn", "let", "return", "if", "else", "while"];
const primitiveTypes = ["i32", "i64", "u32", "u64", "bool"];

export function registerCompletions(context: vscode.ExtensionContext): void {
  const items = [...keywordCompletions(), ...typeCompletions(), ...snippetCompletions()];

  context.subscriptions.push(
    vscode.languages.registerCompletionItemProvider(
      { language: "intkernel" },
      {
        provideCompletionItems: () => items
      }
    )
  );
}

function keywordCompletions(): vscode.CompletionItem[] {
  return keywords.map((label) => {
    const item = new vscode.CompletionItem(label, vscode.CompletionItemKind.Keyword);
    item.detail = "IntKernel keyword";
    item.sortText = `1_${label}`;
    return item;
  });
}

function typeCompletions(): vscode.CompletionItem[] {
  return [
    ...primitiveTypes.map((label) => {
      const item = new vscode.CompletionItem(label, vscode.CompletionItemKind.TypeParameter);
      item.detail = "IntKernel primitive type";
      item.sortText = `2_${label}`;
      return item;
    }),
    snippetItem("ptr<T>", "ptr<${1:Item}>", "IntKernel pointer type", "2_ptr")
  ];
}

function snippetCompletions(): vscode.CompletionItem[] {
  return [
    snippetItem("struct declaration", "struct ${1:Name} {\n  ${2:field}: ${3:i64};\n}", "IntKernel struct declaration", "0_struct"),
    snippetItem("function declaration", "fn ${1:name}(${2:param}: ${3:i64}) -> ${4:i64} {\n  return ${2:param};\n}", "IntKernel internal function", "0_fn"),
    snippetItem("export function", "export fn ${1:name}(${2:param}: ${3:i64}) -> ${4:i64} {\n  return ${2:param};\n}", "IntKernel exported function", "0_export_fn"),
    snippetItem("let binding", "let ${1:name}: ${2:i64} = ${3:0};", "IntKernel let binding", "0_let"),
    snippetItem("if statement", "if ${1:condition} {\n  ${2:return 0;}\n}", "IntKernel if statement", "0_if"),
    snippetItem("if else statement", "if ${1:condition} {\n  ${2:return 0;}\n} else {\n  ${3:return 1;}\n}", "IntKernel if/else statement", "0_if_else"),
    snippetItem("while loop", "while ${1:condition} {\n  ${2:i = i + 1;}\n}", "IntKernel while loop", "0_while"),
    snippetItem("return statement", "return ${1:0};", "IntKernel return statement", "0_return")
  ];
}

function snippetItem(label: string, body: string, detail: string, sortText: string): vscode.CompletionItem {
  const item = new vscode.CompletionItem(label, vscode.CompletionItemKind.Snippet);
  item.insertText = new vscode.SnippetString(body);
  item.detail = detail;
  item.sortText = sortText;
  return item;
}
```

- [ ] **Step 2: Run TypeScript check and observe missing extension entrypoint**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm compile
```

Expected:

```text
Could not resolve "src/extension.ts"
```

If TypeScript reports errors in `src/completions.ts`, fix those errors before continuing. The esbuild entrypoint failure remains expected until Task 6.

- [ ] **Step 3: Commit completions provider**

Run:

```bash
cd /Users/lynn/code/IntKernel
git add ik-vscode-plugin/src/completions.ts
git commit -m "feat: add intkernel completion provider"
```

Expected:

```text
[main ...] feat: add intkernel completion provider
```

---

### Task 6: Wire Extension Activation

**Files:**
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/extension.ts`

- [ ] **Step 1: Create extension entrypoint**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/src/extension.ts`:

```ts
import * as vscode from "vscode";
import { registerCompletions } from "./completions";
import { registerDiagnostics } from "./diagnostics";

export function activate(context: vscode.ExtensionContext): void {
  registerDiagnostics(context);
  registerCompletions(context);
}

export function deactivate(): void {
  // VSCode disposes subscriptions registered on the extension context.
}
```

- [ ] **Step 2: Run unit tests**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm test
```

Expected:

```text
Test Files  1 passed
Tests  5 passed
```

- [ ] **Step 3: Run full compile**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm compile
```

Expected:

```text
dist/extension.cjs      ...
dist/extension.cjs.map  ...
Done in ...
```

- [ ] **Step 4: Smoke-check bundle exports**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
test -s dist/extension.cjs
node <<'EOF'
const Module = require("node:module");
const originalLoad = Module._load;

Module._load = function load(request, parent, isMain) {
  if (request === "vscode") {
    return {
      languages: {
        createDiagnosticCollection: () => ({ set() {}, delete() {}, dispose() {} }),
        registerCompletionItemProvider: () => ({ dispose() {} })
      },
      workspace: {
        textDocuments: [],
        onDidOpenTextDocument: () => ({ dispose() {} }),
        onDidSaveTextDocument: () => ({ dispose() {} }),
        onDidChangeTextDocument: () => ({ dispose() {} }),
        onDidCloseTextDocument: () => ({ dispose() {} })
      },
      Diagnostic: class Diagnostic {
        constructor(range, message, severity) {
          this.range = range;
          this.message = message;
          this.severity = severity;
        }
      },
      DiagnosticSeverity: { Error: 0 },
      Range: class Range {
        constructor(startLine, startCharacter, endLine, endCharacter) {
          this.start = { line: startLine, character: startCharacter };
          this.end = { line: endLine, character: endCharacter };
        }
      },
      CompletionItem: class CompletionItem {
        constructor(label, kind) {
          this.label = label;
          this.kind = kind;
        }
      },
      CompletionItemKind: { Keyword: 0, TypeParameter: 1, Snippet: 2 },
      SnippetString: class SnippetString {
        constructor(value) {
          this.value = value;
        }
      }
    };
  }

  return originalLoad.call(this, request, parent, isMain);
};

const extension = require("./dist/extension.cjs");
if (typeof extension.activate !== "function" || typeof extension.deactivate !== "function") {
  process.exit(1);
}
console.log("OK: bundle exports activate/deactivate");
EOF
```

Expected:

```text
OK: bundle exports activate/deactivate
```

- [ ] **Step 5: Commit activation wiring**

Run:

```bash
cd /Users/lynn/code/IntKernel
git add ik-vscode-plugin/src/extension.ts
git commit -m "feat: activate intkernel vscode language features"
```

Expected:

```text
[main ...] feat: activate intkernel vscode language features
```

---

### Task 7: Add Plugin Documentation

**Files:**
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/README.md`
- Create: `/Users/lynn/code/IntKernel/ik-vscode-plugin/CHANGELOG.md`

- [ ] **Step 1: Create README**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/README.md`:

```md
# IntKernel VSCode Extension

VSCode language support for IntKernel `.ik` files.

## Features

- `.ik` file recognition as the `intkernel` language.
- TextMate syntax highlighting for IntKernel keywords, types, comments, numbers, function declarations, and struct declarations.
- Language configuration for `//` comments and bracket auto-closing.
- Snippets for common declarations and statements.
- Basic keyword, primitive type, pointer type, and declaration completions.
- Compiler-backed diagnostics using the local `intkernel` package.

## Development

Install dependencies:

```sh
pnpm install
```

Compile:

```sh
pnpm compile
```

Run tests:

```sh
pnpm test
```

Package:

```sh
pnpm package
```

## Manual Verification

Open this folder in VSCode and launch an Extension Development Host.

Then:

1. Open `/Users/lynn/code/IntKernel/examples/pricing.ik`.
2. Confirm the language mode is `IntKernel`.
3. Confirm syntax highlighting is visible.
4. Confirm snippets and completions appear in a `.ik` file.
5. Introduce a type error such as assigning `true` to an `i64`.
6. Confirm the Problems panel shows an `intkernel` diagnostic.
7. Revert the type error and confirm the diagnostic clears.
```

- [ ] **Step 2: Create changelog**

Create `/Users/lynn/code/IntKernel/ik-vscode-plugin/CHANGELOG.md`:

```md
# Changelog

## 0.1.0

- Add `.ik` language registration.
- Add TextMate syntax highlighting.
- Add snippets for common IntKernel constructs.
- Add static keyword, type, and template completions.
- Add compiler-backed diagnostics through the local `intkernel` package.
```

- [ ] **Step 3: Run docs-neutral verification**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm test
pnpm compile
```

Expected:

```text
Test Files  1 passed
Tests  5 passed
dist/extension.cjs      ...
```

- [ ] **Step 4: Commit documentation**

Run:

```bash
cd /Users/lynn/code/IntKernel
git add ik-vscode-plugin/README.md ik-vscode-plugin/CHANGELOG.md
git commit -m "docs: document intkernel vscode plugin"
```

Expected:

```text
[main ...] docs: document intkernel vscode plugin
```

---

### Task 8: Verify Packaging And Manual Acceptance

**Files:**
- Read: `/Users/lynn/code/IntKernel/ik-vscode-plugin/docs/superpowers/specs/2026-06-23-intkernel-vscode-v1-design.md`
- Read: `/Users/lynn/code/IntKernel/ik-vscode-plugin/package.json`
- Verify generated: `/Users/lynn/code/IntKernel/ik-vscode-plugin/dist/extension.cjs`

- [ ] **Step 1: Run all automated checks**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm test
pnpm compile
```

Expected:

```text
Test Files  1 passed
Tests  5 passed
dist/extension.cjs      ...
```

- [ ] **Step 2: Package the extension**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
pnpm package
```

Expected:

```text
DONE  Packaged: /Users/lynn/code/IntKernel/ik-vscode-plugin/ik-vscode-plugin-0.1.0.vsix
```

- [ ] **Step 3: Verify valid examples produce no compiler diagnostics**

Create a temporary script by running this command from the plugin directory:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
node --input-type=module <<'EOF'
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { SourceFile, check } from "intkernel";

const examplesDir = join(process.cwd(), "..", "examples");
const files = readdirSync(examplesDir).filter((name) => name.endsWith(".ik"));
const failures = [];

for (const file of files) {
  const path = join(examplesDir, file);
  const result = check(new SourceFile(path, readFileSync(path, "utf8")));
  if (result.diagnostics.length > 0) {
    failures.push(`${file}: ${result.diagnostics.map((diagnostic) => diagnostic.code).join(", ")}`);
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log(`OK: ${files.length} examples checked`);
EOF
```

Expected:

```text
OK: 11 examples checked
```

If the example count changes because new `.ik` files were added, the exact number may be higher. The command must still exit with status 0.

- [ ] **Step 4: Run manual VSCode Extension Development Host check**

Run:

```bash
cd /Users/lynn/code/IntKernel/ik-vscode-plugin
code .
```

In VSCode:

1. Press `F5` to launch an Extension Development Host.
2. In the Extension Development Host, open `/Users/lynn/code/IntKernel/examples/pricing.ik`.
3. Confirm the lower-right language mode says `IntKernel`.
4. Confirm keywords such as `struct`, `export`, `fn`, `while`, and `return` are highlighted.
5. In a new `.ik` file, type `export` and confirm completion items include `export function`.
6. In the same file, type `struct` and confirm the struct snippet appears.
7. In `pricing.ik`, change `let i: i32 = 0;` to `let i: bool = 0;`.
8. Confirm the Problems panel shows an `intkernel` error.
9. Revert the change and confirm the error clears after the debounce delay or after save.

Expected: all nine checks succeed.

- [ ] **Step 5: Record manual result**

Append this section to `/Users/lynn/code/IntKernel/ik-vscode-plugin/CHANGELOG.md` after the `0.1.0` section only after the manual check succeeds:

```md

## Verification

- Automated tests pass with `pnpm test`.
- Extension bundle builds with `pnpm compile`.
- VSIX package builds with `pnpm package`.
- Manual Extension Development Host check passed for highlighting, snippets, completions, diagnostics, and diagnostic clearing.
```

- [ ] **Step 6: Commit final verification notes and package metadata**

Run:

```bash
cd /Users/lynn/code/IntKernel
git add ik-vscode-plugin/CHANGELOG.md ik-vscode-plugin/package.json ik-vscode-plugin/pnpm-lock.yaml
git commit -m "chore: record intkernel vscode plugin verification"
```

Expected:

```text
[main ...] chore: record intkernel vscode plugin verification
```

- [ ] **Step 7: Final status check**

Run:

```bash
cd /Users/lynn/code/IntKernel
git status --short --branch
```

Expected:

```text
## main
```

---

## Spec Coverage Review

- `.ik` language registration is implemented in Task 1 through `package.json`.
- TextMate syntax highlighting is implemented in Task 2 through `syntaxes/intkernel.tmLanguage.json`.
- Language configuration is implemented in Task 2 through `language-configuration.json`.
- Snippets are implemented in Task 2 through `snippets/intkernel.json`.
- Basic completions are implemented in Task 5 through `src/completions.ts`.
- Compiler-backed diagnostics are implemented in Tasks 3 and 4 through `src/diagnosticMapping.ts` and `src/diagnostics.ts`.
- Extension activation is implemented in Task 6 through `src/extension.ts`.
- Automated verification is covered in Tasks 3, 6, 7, and 8.
- Manual VSCode acceptance is covered in Task 8.
- The plan does not modify IntKernel compiler source files outside `ik-vscode-plugin`.
