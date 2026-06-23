import type { FunctionDeclaration, StructDeclaration, TypeNode } from "../../parser/ast.js";
import type { CheckResult } from "../../typeck/checker.js";
import { emitCPrimitiveType } from "./c-common.js";
import { resolveOverflowMode, type CCodegenOptions } from "./c-options.js";

export type EmitCHeaderOptions = CCodegenOptions;

export function emitCHeader(checked: CheckResult, options: EmitCHeaderOptions = {}): string {
  assertCanEmitC(checked);
  const overflowMode = resolveOverflowMode(options);

  const lines: string[] = [
    "#pragma once",
    "",
    "#include <stdint.h>",
    "#include <stdbool.h>"
  ];

  if (overflowMode === "checked") {
    lines.push("#include <stddef.h>");
  }

  lines.push(
    "",
    "#if defined(_WIN32) || defined(__CYGWIN__)",
    "  #ifdef IK_BUILD_DLL",
    "    #define IK_API __declspec(dllexport)",
    "  #else",
    "    #define IK_API __declspec(dllimport)",
    "  #endif",
    "#else",
    "  #define IK_API __attribute__((visibility(\"default\")))",
    "#endif",
  );

  if (overflowMode === "checked") {
    lines.push(
      "",
      "typedef int32_t IK_Status;",
      "",
      "#define IK_OK ((IK_Status)0)",
      "#define IK_ERR_OVERFLOW ((IK_Status)1)",
      "#define IK_ERR_DIV_BY_ZERO ((IK_Status)2)",
      "#define IK_ERR_NULL_POINTER ((IK_Status)3)"
    );
  }

  lines.push(
    "",
    "#ifdef __cplusplus",
    "extern \"C\" {",
    "#endif"
  );
  const structs = checked.ast.declarations.filter((declaration): declaration is StructDeclaration => declaration.kind === "StructDeclaration");
  const exportedFunctions = checked.ast.declarations.filter(
    (declaration): declaration is FunctionDeclaration => declaration.kind === "FunctionDeclaration" && declaration.exported
  );

  for (const structDeclaration of structs) {
    lines.push("", emitStructTypedef(structDeclaration));
  }

  for (const functionDeclaration of exportedFunctions) {
    const signature = overflowMode === "checked" ? emitCheckedFunctionSignature(functionDeclaration) : emitFunctionSignature(functionDeclaration);
    lines.push("", `IK_API ${signature};`);
  }

  lines.push("", "#ifdef __cplusplus", "}", "#endif");

  return `${lines.join("\n")}\n`;
}

export function assertCanEmitC(checked: CheckResult): void {
  if (checked.diagnostics.length > 0) {
    throw new Error("Cannot emit C for a program with diagnostics.");
  }
}

export function emitFunctionSignature(functionDeclaration: FunctionDeclaration): string {
  const returnType = emitCType(functionDeclaration.returnType);
  const params = functionDeclaration.params.map((param) => `${emitCType(param.type)} ${param.name.name}`).join(", ");
  return `${returnType} ${functionDeclaration.name.name}(${params})`;
}

export function emitCheckedFunctionSignature(functionDeclaration: FunctionDeclaration): string {
  const params = functionDeclaration.params.map((param) => `${emitCType(param.type)} ${param.name.name}`);
  params.push(`${emitCType(functionDeclaration.returnType)}* ik_return`);
  return `IK_Status ${functionDeclaration.name.name}(${params.join(", ")})`;
}

export function emitCType(type: TypeNode): string {
  switch (type.kind) {
    case "PrimitiveType":
      return emitCPrimitiveType(type.name);
    case "PointerType":
      return `${emitCType(type.elementType)}*`;
    case "NamedType":
      return type.name.name;
    case "ErrorType":
      throw new Error("Cannot emit C for unresolved type.");
  }
}

function emitStructTypedef(structDeclaration: StructDeclaration): string {
  const lines = [`typedef struct ${structDeclaration.name.name} {`];
  for (const field of structDeclaration.fields) {
    lines.push(`  ${emitCType(field.type)} ${field.name.name};`);
  }
  lines.push(`} ${structDeclaration.name.name};`);
  return lines.join("\n");
}
