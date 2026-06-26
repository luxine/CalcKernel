export { SourceFile } from "./source/source-file.js";
export type { SourcePosition, SourceSpan } from "./source/source-file.js";
export { formatDiagnostic, formatDiagnostics } from "./source/diagnostics.js";
export type { Diagnostic, DiagnosticCode } from "./source/diagnostics.js";
export { lex } from "./lexer/lexer.js";
export type { LexResult } from "./lexer/lexer.js";
export { TokenKind } from "./lexer/token.js";
export type { Token } from "./lexer/token.js";
export { parse } from "./parser/parser.js";
export type { ParseResult } from "./parser/parser.js";
export type * from "./parser/ast.js";
export { check, getExprType, getFieldInfo, getFunctionInfo, getLetType, getStructInfo } from "./typeck/checker.js";
export type {
  CheckedProgram,
  CheckResult,
  FunctionInfo,
  FunctionParamInfo,
  LetTypeMap,
  StructFieldInfo,
  StructInfo,
  TypeMap,
  TypedAst
} from "./typeck/checker.js";
export { Scope, SymbolTable } from "./typeck/symbols.js";
export type { FunctionSymbol, StructSymbol, VariableSymbol } from "./typeck/symbols.js";
export type { CalcKernelType, PrimitiveTypeName } from "./typeck/types.js";
export { emitCHeader } from "./backend/c/c-header-emitter.js";
export { buildSharedLibrary, emitDefaultCSource as emitCSource, emitCFiles, sharedLibraryOutputPath } from "./backend/c/c-build.js";
export type {
  BuildSharedLibraryOptions,
  BuildSharedLibraryResult,
  CommandResult,
  CommandRunner,
  EmitCFilesOptions,
  EmitDefaultCSourceOptions as EmitCSourceOptions
} from "./backend/c/c-build.js";
export { CKWasmArena } from "./wasm/ck-wasm-arena.js";
export type { CKWasmArenaCopy, CKWasmArenaOptions, CKWasmGlobal, CKWasmMemory } from "./wasm/ck-wasm-arena.js";
