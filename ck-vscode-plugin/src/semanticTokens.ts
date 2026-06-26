import * as vscode from "vscode";
import { analyzeCalcKernelDocument, type CalcKernelAnalysis, type CalcKernelReference, type CalcKernelSymbol } from "./languageService";

export const semanticTokenLegend = new vscode.SemanticTokensLegend(
  ["type", "function", "parameter", "variable", "property"],
  ["declaration"]
);

export interface SemanticTokenData {
  text: string;
  range: vscode.Range;
  tokenType: number;
  tokenModifiers: number;
}

const declarationModifier = 1 << semanticTokenLegend.tokenModifiers.indexOf("declaration");

export function buildSemanticTokenData(analysis: CalcKernelAnalysis): SemanticTokenData[] {
  const tokens: SemanticTokenData[] = [
    ...analysis.symbols.map(symbolToToken),
    ...analysis.references.map(referenceToToken)
  ];
  return tokens.filter(isNonEmptyToken).sort((left, right) => left.range.start.compareTo(right.range.start));
}

export function registerSemanticTokens(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.languages.registerDocumentSemanticTokensProvider(
      { language: "calckernel" },
      {
        provideDocumentSemanticTokens(document) {
          const analysis = analyzeCalcKernelDocument(document);
          const builder = new vscode.SemanticTokensBuilder(semanticTokenLegend);
          for (const token of buildSemanticTokenData(analysis)) {
            builder.push(
              token.range.start.line,
              token.range.start.character,
              token.range.end.character - token.range.start.character,
              token.tokenType,
              token.tokenModifiers
            );
          }
          return builder.build();
        }
      },
      semanticTokenLegend
    )
  );
}

function symbolToToken(symbol: CalcKernelSymbol): SemanticTokenData {
  return {
    text: symbol.name,
    range: symbol.selectionRange,
    tokenType: tokenTypeForKind(symbol.kind),
    tokenModifiers: declarationModifier
  };
}

function referenceToToken(reference: CalcKernelReference): SemanticTokenData {
  return {
    text: reference.name,
    range: reference.range,
    tokenType: tokenTypeForKind(reference.kind),
    tokenModifiers: 0
  };
}

function isNonEmptyToken(token: SemanticTokenData): boolean {
  return token.text.length > 0 && token.range.end.compareTo(token.range.start) > 0;
}

function tokenTypeForKind(kind: string): number {
  if (kind === "struct" || kind === "type") return semanticTokenLegend.tokenTypes.indexOf("type");
  if (kind === "function") return semanticTokenLegend.tokenTypes.indexOf("function");
  if (kind === "parameter") return semanticTokenLegend.tokenTypes.indexOf("parameter");
  if (kind === "field") return semanticTokenLegend.tokenTypes.indexOf("property");
  return semanticTokenLegend.tokenTypes.indexOf("variable");
}
