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
