import * as vscode from "vscode";
import { analyzeIntKernelDocument } from "./languageService";

const keywords = ["struct", "export", "fn", "let", "return", "if", "else", "while"];
const primitiveTypes = ["i32", "i64", "u32", "u64", "bool"];

export function registerCompletions(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.languages.registerCompletionItemProvider(
      { language: "intkernel" },
      {
        provideCompletionItems: (document, position) => {
          const linePrefix = document.lineAt(position).text.slice(0, position.character);
          return buildCompletionItems(analyzeIntKernelDocument(document), position, linePrefix);
        }
      },
      "."
    )
  );
}

export function buildCompletionItems(
  analysis?: import("./languageService").IntKernelAnalysis,
  position?: vscode.Position,
  linePrefix = ""
): vscode.CompletionItem[] {
  const items = [...keywordCompletions(), ...typeCompletions(), ...snippetCompletions()];
  if (!analysis || !position) {
    return items;
  }

  const receiverName = memberReceiverName(linePrefix);
  if (receiverName) {
    const receiver = nearestReceiverReference(analysis, receiverName, position) ?? visibleReceiverSymbol(analysis, receiverName, position);
    const structName = receiver?.typeLabel;
    return [
      ...items,
      ...analysis.symbols
        .filter((symbol) => symbol.kind === "field" && symbol.containerName === structName)
        .map((symbol) => memberFieldCompletion(symbol))
    ];
  }

  return [
    ...items,
    ...analysis.symbols
      .filter((symbol) => (symbol.kind === "local" || symbol.kind === "parameter") && isSymbolVisibleAtPosition(symbol, position))
      .map((symbol) => symbolCompletion(symbol, vscode.CompletionItemKind.Variable)),
    ...analysis.symbols
      .filter((symbol) => symbol.kind === "function")
      .map((symbol) => symbolCompletion(symbol, vscode.CompletionItemKind.Function)),
    ...analysis.symbols
      .filter((symbol) => symbol.kind === "struct")
      .map((symbol) => symbolCompletion(symbol, vscode.CompletionItemKind.Struct))
  ];
}

function isSymbolVisibleAtPosition(symbol: import("./languageService").IntKernelSymbol, position: vscode.Position): boolean {
  if (comparePositions(symbol.selectionRange.start, position) > 0) {
    return false;
  }
  return symbol.scopeRange?.contains(position) ?? true;
}

function symbolCompletion(
  symbol: import("./languageService").IntKernelSymbol,
  kind: vscode.CompletionItemKind
): vscode.CompletionItem {
  const item = new vscode.CompletionItem(symbol.name, kind);
  item.detail = symbol.detail ?? symbol.signatureLabel;
  item.sortText = `3_${symbol.kind}_${symbol.name}`;
  return item;
}

function memberFieldCompletion(symbol: import("./languageService").IntKernelSymbol): vscode.CompletionItem {
  const item = symbolCompletion(symbol, vscode.CompletionItemKind.Field);
  item.sortText = `!_field_${symbol.name}`;
  return item;
}

function nearestReceiverReference(
  analysis: import("./languageService").IntKernelAnalysis,
  receiverName: string,
  position: vscode.Position
): import("./languageService").IntKernelReference | undefined {
  return analysis.references
    .filter((reference) => reference.name === receiverName && comparePositions(reference.range.end, position) <= 0)
    .sort((left, right) => comparePositions(right.range.end, left.range.end))[0];
}

function visibleReceiverSymbol(
  analysis: import("./languageService").IntKernelAnalysis,
  receiverName: string,
  position: vscode.Position
): import("./languageService").IntKernelSymbol | undefined {
  return analysis.symbols
    .filter((symbol) => {
      if (symbol.name !== receiverName || (symbol.kind !== "local" && symbol.kind !== "parameter")) {
        return false;
      }
      return isSymbolVisibleAtPosition(symbol, position);
    })
    .sort((left, right) => comparePositions(right.selectionRange.start, left.selectionRange.start))[0];
}

function memberReceiverName(linePrefix: string): string | undefined {
  const match = /([A-Za-z_][A-Za-z0-9_]*)\.$/.exec(linePrefix);
  return match?.[1];
}

function comparePositions(left: vscode.Position, right: vscode.Position): number {
  if (left.line !== right.line) {
    return left.line - right.line;
  }
  return left.character - right.character;
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
