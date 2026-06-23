import type { WasmValueType } from "./wasm-types.js";
import { escapeWatString, toWasmIdentifier } from "./wasm-names.js";
import { WatPrinter } from "./wat-printer.js";

export interface WatParam {
  name: string;
  type: WasmValueType;
}

export interface WatLocal {
  name: string;
  type: WasmValueType;
}

export interface WatFunction {
  name: string;
  exportName?: string;
  params?: WatParam[];
  result?: WasmValueType;
  locals?: WatLocal[];
  body?: string[];
}

export interface WatMemory {
  exportName: string;
  pages: number;
}

export interface WatModule {
  memory?: WatMemory | null;
  functions?: WatFunction[];
}

export function emitWatModule(module: WatModule = {}): string {
  const printer = new WatPrinter();
  const memory = module.memory === undefined ? { exportName: "memory", pages: 1 } : module.memory;
  const functions = module.functions ?? [];

  printer.open("(module");

  if (memory) {
    printer.line(`(memory (export "${escapeWatString(memory.exportName)}") ${memory.pages})`);
  }

  for (const func of functions) {
    printer.line();
    emitWatFunction(printer, func);
  }

  printer.close();
  return printer.print();
}

function emitWatFunction(printer: WatPrinter, func: WatFunction): void {
  const exportClause = func.exportName === undefined ? "" : ` (export "${escapeWatString(func.exportName)}")`;
  printer.open(`(func ${toWasmIdentifier(func.name)}${exportClause}`);

  for (const param of func.params ?? []) {
    printer.line(`(param ${toWasmIdentifier(param.name)} ${param.type})`);
  }

  if (func.result) {
    printer.line(`(result ${func.result})`);
  }

  for (const local of func.locals ?? []) {
    printer.line(`(local ${toWasmIdentifier(local.name)} ${local.type})`);
  }

  for (const instruction of func.body ?? []) {
    printer.line(instruction);
  }

  printer.close();
}
